//! Crystal binding generation.
//!
//! Emits a single Crystal source file per crate containing:
//! 1. A `lib` block binding the exported C FFI symbols (`fun` declarations).
//! 2. A high-level module with Ruby-style snake_case wrapper methods that
//!    marshal arguments/return values across the JSON-string C ABI.
//!
//! plus a `shard.yml` package manifest.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use heck::ToPascalCase;

use crate::codegen::naming::{
    PublicIdentifierKind, abi_symbol, public_host_identifier, wire_field_name, wire_variant_value,
};
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{Language, ResolvedCrateConfig, resolve_output_dir};
use crate::core::ir::{ApiSurface, DefaultValue, EnumDef, ErrorDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};

use super::template_env::render;
use super::type_map::{crystal_c_type, crystal_type, crystal_type_name};

pub struct CrystalBackend;

impl CrystalBackend {
    /// Crystal `lib` binding name, e.g. `LibSampleCore`.
    fn lib_name(ffi_prefix: &str) -> String {
        format!("Lib{}", ffi_prefix.to_pascal_case())
    }

    /// Crystal wrapper module name, e.g. `SampleCore`.
    fn module_name(crate_name: &str) -> String {
        crate_name.to_pascal_case()
    }

    /// Shard package name (Crystal shard names are conventionally snake/kebab).
    fn shard_name(crate_name: &str) -> String {
        crate_name.replace('-', "_")
    }

    /// Whether a function should be skipped for Crystal generation.
    fn is_excluded(func: &FunctionDef, ffi_exclude: &HashSet<String>) -> bool {
        func.binding_excluded || ffi_exclude.contains(&func.name)
    }

    /// Render the `fun` param list for a lib declaration.
    fn lib_params(func: &FunctionDef, opaque: &HashSet<String>, ffi_structs: &HashSet<String>) -> String {
        func.params
            .iter()
            .map(|p| {
                let name = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, &p.name);
                let cty = c_type_of(&p.ty, opaque, ffi_structs);
                format!("{name} : {cty}")
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Render the C return type for a lib declaration.
    fn lib_return(func: &FunctionDef, opaque: &HashSet<String>, ffi_structs: &HashSet<String>) -> String {
        lib_c_return(&func.return_type, func.error_type.as_deref(), opaque, ffi_structs)
    }

    /// Generate the `lib` block binding all exported C symbols.
    #[allow(clippy::too_many_arguments)]
    fn gen_lib_block(
        api: &ApiSurface,
        ffi_prefix: &str,
        ffi_lib_name: &str,
        ffi_header: &str,
        ffi_exclude: &HashSet<String>,
        opaque: &HashSet<String>,
        streaming: &[StreamSpec],
        async_methods: &[AsyncMethodSpec],
        ffi_structs: &HashSet<String>,
    ) -> String {
        let mut out = render(
            "lib_header.jinja",
            minijinja::context! {
                lib_name => Self::lib_name(ffi_prefix),
                ffi_prefix => ffi_prefix,
                ffi_lib_name => ffi_lib_name,
                ffi_header => ffi_header,
            },
        );

        // Error reporting: every fallible FFI function sets a thread-local
        // error on failure. Crystal calls these to detect and raise errors.
        out.push_str(&format!(
            "  fun last_error_code = {ffi_prefix}_last_error_code() : Int32\n"
        ));
        out.push_str(&format!(
            "  fun last_error_context = {ffi_prefix}_last_error_context() : LibC::Char*\n"
        ));
        out.push('\n');

        // Emit opaque struct declarations for C FFI struct types (non-opaque types
        // that have `from_json`/`to_json`/`free` helpers). These must be declared
        // before any `fun` that references them.
        let mut sorted_structs: Vec<&String> = ffi_structs.iter().collect();
        sorted_structs.sort();
        for name in &sorted_structs {
            let struct_name = crystal_type_name(name);
            // Crystal 1.19+ rejects empty struct declarations. Use a dummy data
            // field so the struct type compiles; the size is irrelevant because
            // these are always passed as pointers (Config*).
            out.push_str(&format!(
                "  struct {struct_name}\n    _data : Void*\n  end\n"
            ));
        }
        // Emit `from_json`/`to_json`/`free` helper declarations for each struct type.
        for name in &sorted_structs {
            let type_snake = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, name);
            out.push_str(&format!(
                "  fun {type_snake}_from_json = {ffi_prefix}_{type_snake}_from_json(json : LibC::Char*) : {struct_name}*\n",
                struct_name = crystal_type_name(name),
            ));
            out.push_str(&format!(
                "  fun {type_snake}_to_json = {ffi_prefix}_{type_snake}_to_json(ptr : {struct_name}*) : LibC::Char*\n",
                struct_name = crystal_type_name(name),
            ));
            out.push_str(&format!(
                "  fun {type_snake}_free = {ffi_prefix}_{type_snake}_free(ptr : {struct_name}*)\n",
                struct_name = crystal_type_name(name),
            ));
        }
        if !sorted_structs.is_empty() {
            out.push('\n');
        }

        for func in &api.functions {
            if Self::is_excluded(func, ffi_exclude) {
                continue;
            }
            let crystal_name = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &func.name);
            let c_symbol = abi_symbol(ffi_prefix, &func.name);
            let ret_ty = Self::lib_return(func, opaque, ffi_structs);
            out.push_str(&render(
                "lib_fun.jinja",
                minijinja::context! {
                    doc => func.doc.lines().next().unwrap_or_default().trim(),
                    crystal_name => crystal_name,
                    c_symbol => c_symbol,
                        params => Self::lib_params(func, opaque, ffi_structs),
                        return_type => ret_ty,
                },
            ));
        }

        // Opaque handle types: method bindings + a destructor per type.
        let mut opaque_types: Vec<&TypeDef> = api
            .types
            .iter()
            .filter(|t| t.is_opaque && !t.binding_excluded)
            .collect();
        opaque_types.sort_by(|a, b| a.name.cmp(&b.name));
        for ty in opaque_types {
            let type_snake = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &ty.name);
            for m in &ty.methods {
                if m.binding_excluded {
                    continue;
                }
                let method_snake = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &m.name);
                let c_symbol = format!("{}_{}", abi_symbol(ffi_prefix, &ty.name), method_snake);
                let mut params = if m.is_static {
                    Vec::new()
                } else {
                    vec!["handle : Void*".to_string()]
                };
                for p in &m.params {
                    let pn = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, &p.name);
                    params.push(format!("{pn} : {}", c_type_of(&p.ty, opaque, ffi_structs)));
                }
                let ret = lib_c_return(&m.return_type, m.error_type.as_deref(), opaque, ffi_structs);
                out.push_str(&format!(
                    "  fun {type_snake}_{method_snake} = {c_symbol}({}) : {ret}\n",
                    params.join(", ")
                ));
            }
            let c_symbol = format!("{}_free", abi_symbol(ffi_prefix, &ty.name));
            out.push_str(&format!(
                "  fun {type_snake}_free = {c_symbol}(handle : Void*) : Void\n"
            ));
        }

        // Streaming iterator bindings (`_start`/`_next`/`_free`) + per-item JSON/free.
        let mut item_funcs_done: HashSet<String> = HashSet::new();
        for spec in streaming {
            let owner_snake = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &spec.owner);
            let method_snake = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &spec.method);
            let item_snake = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &spec.item);
            let base = format!("{owner_snake}_{method_snake}");
            let c_base = format!("{}_{}", abi_symbol(ffi_prefix, &spec.owner), method_snake);
            // Opaque-owner streams pass the handle receiver; free (owner-less) streams do not.
            let mut start_params = if opaque.contains(&spec.owner) {
                vec!["handle : Void*".to_string()]
            } else {
                Vec::new()
            };
            for (pname, pty) in &spec.params {
                let pn = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, pname);
                start_params.push(format!("{pn} : {}", c_type_of(pty, opaque, ffi_structs)));
            }
            out.push_str(&format!(
                "  fun {base}_start = {c_base}_start({}) : Void*\n",
                start_params.join(", ")
            ));
            out.push_str(&format!("  fun {base}_next = {c_base}_next(handle : Void*) : Void*\n"));
            out.push_str(&format!("  fun {base}_free = {c_base}_free(handle : Void*) : Void\n"));
            if item_funcs_done.insert(item_snake.clone()) {
                let c_item = abi_symbol(ffi_prefix, &spec.item);
                // If the item type is already an FFI struct, it already has typed
                // `to_json`/`free` declarations above. The streaming code path
                // will cast the Void* chunk pointer before calling them.
                if !ffi_structs.contains(&spec.item) {
                    out.push_str(&format!(
                        "  fun {item_snake}_to_json = {c_item}_to_json(chunk : Void*) : LibC::Char*\n"
                    ));
                    out.push_str(&format!(
                        "  fun {item_snake}_free = {c_item}_free(chunk : Void*) : Void\n"
                    ));
                }
            }
        }

        // Async method adapter bindings — C functions generated by the FFI
        // backend (suffixed with `_json`) that accept a client handle, one C
        // string pointer per adapter param, and return a JSON C string.
        for spec in async_methods {
            let owner_snake = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &spec.owner);
            let method_snake = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &spec.method);
            let c_owner = abi_symbol(ffi_prefix, &spec.owner);
            let mut c_params = vec!["handle : Void*".to_string()];
            for (pn, pty) in &spec.params {
                let param_name = if matches!(pty, TypeRef::String) {
                    pn.clone()
                } else {
                    format!("{pn}_json")
                };
                c_params.push(format!("{param_name} : LibC::Char*"));
            }
            out.push_str(&format!(
                "  fun {owner_snake}_{method_snake}_json = {c_owner}_{method_snake}_json({}) : LibC::Char*\n",
                c_params.join(", ")
            ));
        }

        out.push_str("end\n\n");
        out
    }

    /// Generate the high-level Ruby-style module with wrapper methods.
    fn gen_module(
        api: &ApiSurface,
        ffi_prefix: &str,
        ffi_exclude: &HashSet<String>,
        opaque: &HashSet<String>,
        streaming: &[StreamSpec],
        async_methods: &[AsyncMethodSpec],
        ffi_structs: &HashSet<String>,
        module_name: &str,
        borrowed_handles: &HashSet<String>,
    ) -> String {
        let module_name = module_name.to_string();
        let lib_name = Self::lib_name(ffi_prefix);
        let mut out = render(
            "module_header.jinja",
            minijinja::context! {
                crate_name => api.crate_name,
                module_name => module_name,
                version => format!("{:?}", api.version),
            },
        );

        // Type definitions (structs, enums, error classes) live inside the module
        // namespace so wrapper methods can reference `Config.from_json(...)` etc.
        out.push_str(&Self::gen_types(
            api,
            ffi_prefix,
            &lib_name,
            opaque,
            streaming,
            async_methods,
            ffi_structs,
            &module_name,
            borrowed_handles,
        ));

        for func in &api.functions {
            if Self::is_excluded(func, ffi_exclude) {
                continue;
            }
            out.push_str(&Self::gen_wrapper_method(func, &lib_name, opaque, ffi_structs));
        }

        // Free (owner-less) streaming methods — owners that are not opaque handles
        // become module-level functions. Opaque-owned streams are emitted as methods
        // on their handle class by `gen_opaque`.
        for spec in streaming.iter().filter(|s| !opaque.contains(&s.owner)) {
            let owner_snake = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &spec.owner);
            out.push_str(&Self::gen_stream_method(spec, &owner_snake, &lib_name, None, opaque, ffi_structs));
        }

        out.push_str("end\n");
        out
    }

    /// Generate Crystal type definitions from the IR: structs → JSON-serializable
    /// classes, enums, error exception classes, and opaque handle wrappers.
    fn gen_types(
        api: &ApiSurface,
        ffi_prefix: &str,
        lib_name: &str,
        opaque: &HashSet<String>,
        streaming: &[StreamSpec],
        async_methods: &[AsyncMethodSpec],
        ffi_structs: &HashSet<String>,
        module_name: &str,
        borrowed_handles: &HashSet<String>,
    ) -> String {
        // Build a map of unit-enum type name → first variant name so gen_struct
        // can default non-optional enum fields (matching Rust's Default impl which
        // picks the first variant). Only unit enums (no data fields) qualify —
        // internally-tagged / tagged-union enums use abstract classes and cannot
        // be meaningfully defaulted with a single value.
        let is_unit_enum =
            |en: &EnumDef| en.variants.iter().all(|v| v.fields.is_empty() && !v.is_tuple && !v.originally_had_data_fields);
        let enum_first_variant: HashMap<String, String> = api
            .enums
            .iter()
            .filter(|en| !en.binding_excluded && is_unit_enum(en))
            .filter_map(|en| {
                // Prefer the `#[default]` variant (matches Rust's `Default` impl),
                // falling back to the first non-excluded variant.
                let default_variant = en
                    .variants
                    .iter()
                    .find(|v| !v.binding_excluded && v.is_default)
                    .or_else(|| en.variants.iter().find(|v| !v.binding_excluded));
                default_variant.map(|v| (en.name.clone(), v.name.clone()))
            })
            .collect();
        // Collect enum types that use serde(tag = "...") — they generate abstract
        // classes with `use_json_discriminator` and cannot be defaulted with
        // `from_json("{}")` (the discriminator field would be missing).
        let serde_tagged_enums: HashSet<String> = api
            .enums
            .iter()
            .filter(|en| en.serde_tag.is_some() && !en.binding_excluded)
            .map(|en| en.name.clone())
            .collect();
        // For internally-tagged enums, map the enum name → the wire value of its
        // `#[default]` variant (falling back to the first variant). Used to emit a
        // valid default for the abstract-class type: `from_json("{\"<tag>\":\"<wire>\"}")`.
        let serde_tagged_defaults: HashMap<String, String> = api
            .enums
            .iter()
            .filter(|en| en.serde_tag.is_some() && !en.binding_excluded)
            .filter_map(|en| {
                let tag = en.serde_tag.as_deref()?;
                let default_var = en
                    .variants
                    .iter()
                    .find(|v| !v.binding_excluded && v.is_default)
                    .or_else(|| en.variants.iter().find(|v| !v.binding_excluded))?;
                let wire = wire_variant_value(&default_var.name, default_var.serde_rename.as_deref(), en.serde_rename_all.as_deref());
                Some((en.name.clone(), format!("{tag:?}: {wire:?}")))
            })
            .collect();
        // For externally-tagged enums (unit + data variants, custom `new(pull)`),
        // map the enum name → the Crystal subclass constructor of its `#[default]`
        // unit variant, e.g. `OutputFormat::Plain`. Only when a default unit variant
        // exists. Used by struct_default_expr to default such fields.
        let external_defaults: HashMap<String, String> = api
            .enums
            .iter()
            .filter(|en| en.serde_tag.is_none() && !en.serde_untagged && !en.binding_excluded)
            .filter_map(|en| {
                let unit_variants: Vec<&crate::core::ir::EnumVariant> = en
                    .variants
                    .iter()
                    .filter(|v| !v.binding_excluded && v.fields.is_empty() && !v.is_tuple && !v.originally_had_data_fields)
                    .collect();
                let default_var = unit_variants
                    .iter()
                    .find(|v| v.is_default)
                    .or_else(|| unit_variants.first())?;
                let tn = crystal_type_name(&en.name);
                let class = crystal_type_name(&default_var.name);
                Some((en.name.clone(), format!("{tn}::{class}.new")))
            })
            .collect();
        // Unit enums whose wire (serde) value differs from the Crystal variant name
        // need a JSON converter module so `JSON::Serializable` can map wire strings
        // (e.g. "og:image") to variants (OgImage). See gen_enum_converter.
        let enum_converters: HashSet<String> = api
            .enums
            .iter()
            .filter(|en| !en.binding_excluded && is_unit_enum(en))
            .filter(|en| {
                en.variants.iter().filter(|v| !v.binding_excluded).any(|v| {
                    let vname = public_host_identifier(Language::Crystal, PublicIdentifierKind::EnumVariant, &v.name);
                    let wire = wire_variant_value(&v.name, v.serde_rename.as_deref(), en.serde_rename_all.as_deref());
                    // Crystal's default Enum.parse accepts the PascalCase name and its
                    // snake/underscore normalization, so only emit a converter when the
                    // wire value can't round-trip to the variant name.
                    wire != vname && !crystal_enum_parse_matches(&wire, &vname)
                })
            })
            .map(|en| en.name.clone())
            .collect();
        // Shape-discriminated (untagged) unions match variants by JSON shape via a
        // custom `def self.new(pull)`; a `{}` default matches no variant, so fields
        // of these types cannot use an eager `from_json("{}")` default.
        let untagged_unions: HashSet<String> = api
            .enums
            .iter()
            .filter(|en| !en.binding_excluded && en.serde_untagged)
            .map(|en| en.name.clone())
            .collect();

        let mut out = String::new();
        // Emit enums + their JSON converters BEFORE structs so structs that
        // reference `{Enum}Converter` via `@[JSON::Field(converter: ...)]`
        // resolve the constant (Crystal attributes need it defined earlier).
        for en in &api.enums {
            if en.binding_excluded {
                continue;
            }
            out.push_str(&Self::gen_enum(en, api, module_name));
            if enum_converters.contains(&en.name) {
                out.push_str(&Self::gen_enum_converter(en));
            }
        }
        for ty in &api.types {
            if ty.binding_excluded || ty.is_trait {
                continue;
            }
            if ty.is_opaque {
                out.push_str(&Self::gen_opaque(
                    ty,
                    ffi_prefix,
                    lib_name,
                    opaque,
                    streaming,
                    async_methods,
                    ffi_structs,
            &borrowed_handles,
        ));
                continue;
            }
            out.push_str(&Self::gen_struct(ty, opaque, &enum_first_variant, &serde_tagged_enums, &enum_converters, &serde_tagged_defaults, &external_defaults, module_name, &untagged_unions));
        }
        for err in &api.errors {
            if err.binding_excluded {
                continue;
            }
            out.push_str(&Self::gen_error(err));
        }
        out
    }

    /// Emit a Crystal `class` (reference type, so self-referential DTOs are legal)
    /// with `JSON::Serializable` and one getter per field.
    fn gen_struct(ty: &TypeDef, opaque: &HashSet<String>, enum_first_variant: &HashMap<String, String>, serde_tagged_enums: &HashSet<String>, enum_converters: &HashSet<String>, serde_tagged_defaults: &HashMap<String, String>, external_defaults: &HashMap<String, String>, module_name: &str, untagged_unions: &HashSet<String>) -> String {
        let name = crystal_type_name(&ty.name);
        let mut out = String::new();
        out.push('\n');
        use crate::codegen::doc_emission::emit_crystal_doc;
        emit_crystal_doc(&mut out, &ty.doc, "  ");
        out.push_str(&format!("  class {name}\n"));
        out.push_str("    include JSON::Serializable\n");
        for field in &ty.fields {
            if field.binding_excluded {
                continue;
            }
            let field_name = public_host_identifier(Language::Crystal, PublicIdentifierKind::Field, &field.name);
            let wire = wire_field_name(
                &field.name,
                field.serde_rename.as_deref(),
                ty.serde_rename_all.as_deref(),
            );
            let mut field_ty = crystal_type(&field.ty).into_owned();
            // `optional` fields (Rust `Option<T>` or `#[serde(default)]`) are nilable in Crystal.
            if field.optional && !field_ty.ends_with('?') {
                field_ty.push('?');
            }
            emit_crystal_doc(&mut out, &field.doc, "    ");
            if wire != field_name {
                out.push_str(&format!("    @[JSON::Field(key: {wire:?})]\n"));
            }
            // Enum fields with custom wire values need a converter annotation so
            // JSON::Serializable maps wire strings (e.g. "og:image") to variants.
            // Fully-qualify the converter constant: JSON::Serializable expands in a
            // generic context where a bare reference can resolve from the FFI `lib`
            // scope (which has no such constant) instead of the wrapper module.
            if let Some(enum_name) = enum_converter_for_type(&field.ty, enum_converters) {
                out.push_str(&format!("    @[JSON::Field(converter: {module_name}::{enum_name}Converter)]\n"));
            }
            // Opaque handle types cannot be round-tripped through JSON; they are
            // constructed from FFI pointers and must be set programmatically.
            let is_opaque_field = matches!(&field.ty, TypeRef::Named(n) if opaque.contains(n));
            if is_opaque_field {
                out.push_str("    @[JSON::Field(ignore: true)]\n");
                out.push_str(&format!("    getter {field_name} : {field_ty}\n"));
                continue;
            }
            // Bytes fields: represent as `Array(UInt8)` so the value object
            // round-trips through JSON (Rust serializes `Vec<u8>` as a JSON array
            // of ints, matching Go's `[]byte`). `Bytes` (Slice) has no stdlib
            // JSON::Serializable, so we use Array(UInt8) instead.
            if field_type_contains_bytes(&field.ty) {
                if field.optional {
                    out.push_str(&format!("    getter {field_name} : Array(UInt8)?\n"));
                } else {
                    out.push_str(&format!("    getter {field_name} : Array(UInt8) = [] of UInt8\n"));
                }
                continue;
            }
            // Emit a default value in the getter declaration so partial JSON input
            // doesn't fail (`getter field : Type = default` in Crystal provides
            // the default when the JSON key is missing — `@[JSON::Field(default:)]`
            // does NOT work for `from_json` in Crystal 1.19).
            if !field.optional {
                let default_expr = crystal_default_expr(&field.typed_default, &field.default)
                    .or_else(|| type_based_default_expr(&field.ty))
                    .or_else(|| enum_default_expr(&field.ty, enum_first_variant))
                    .or_else(|| external_enum_default_expr(&field.ty, external_defaults))
                    .or_else(|| {
                        // Untagged (shape-discriminated) unions can't be defaulted
                        // with `{}` (no variant matches); emit no eager default.
                        if matches!(&field.ty, TypeRef::Named(n) if untagged_unions.contains(n)) {
                            None
                        } else {
                            struct_default_expr(&field.ty, serde_tagged_enums, serde_tagged_defaults)
                        }
                    });
                if let Some(crystal_default) = default_expr {
                    out.push_str(&format!("    getter {field_name} : {field_ty} = {crystal_default}\n"));
                    continue;
                }
                // Untagged unions can't be eagerly defaulted; expose nilable so the
                // empty `initialize` doesn't fail (JSON input always provides it).
                if matches!(&field.ty, TypeRef::Named(n) if untagged_unions.contains(n)) {
                    out.push_str(&format!("    getter {field_name} : {field_ty}?\n"));
                    continue;
                }
                if let Some(crystal_default) = default_expr {
                    out.push_str(&format!("    getter {field_name} : {field_ty} = {crystal_default}\n"));
                    continue;
                }
            }
            out.push_str(&format!("    getter {field_name} : {field_ty}\n"));
        }
        // Zero-arg constructor: `Config.new` yields all getter-defaults and
        // serializes to Rust's default shape (mirrors Go's `Config{}`, Python's
        // `.default()`, Ruby's `Default::default()`). `from_json` still handles
        // partial/full input. Only emitted when every non-nilable field has a
        // safe getter default (see F4 + enum default fixes).
        if !ty.is_opaque {
            let all_defaultable = ty.fields.iter().filter(|f| !f.binding_excluded).all(|f| {
                f.optional
                    || crystal_default_expr(&f.typed_default, &f.default).is_some()
                    || type_based_default_expr(&f.ty).is_some()
                    || enum_default_expr(&f.ty, enum_first_variant).is_some()
                    || external_enum_default_expr(&f.ty, external_defaults).is_some()
                    || struct_default_expr(&f.ty, serde_tagged_enums, serde_tagged_defaults).is_some()
            });
            if all_defaultable {
                out.push_str("    def initialize\n    end\n");
            }
        }
        out.push_str("  end\n");
        out
    }

    /// Emit a Crystal wrapper `class` for an opaque handle type: it owns the raw
    /// FFI pointer, exposes it via `to_unsafe` (so it can be passed to `lib` funs),
    /// and frees it in `finalize`.
    fn gen_opaque(
        ty: &TypeDef,
        ffi_prefix: &str,
        lib_name: &str,
        opaque: &HashSet<String>,
        streaming: &[StreamSpec],
        async_methods: &[AsyncMethodSpec],
        ffi_structs: &HashSet<String>,
        borrowed_handles: &HashSet<String>,
    ) -> String {
        let name = crystal_type_name(&ty.name);
        let type_snake = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &ty.name);
        let free = format!("{type_snake}_free");
        let _ = ffi_prefix;
        let mut out = Self::doc_or_blank(&ty.doc);
        out.push_str(&format!("  class {name}\n"));
        out.push_str("    # Wraps the FFI handle; do not construct directly.\n");
        out.push_str("    def initialize(@handle : Void*)\n    end\n");
        out.push_str("    # Raw handle for passing back across the C ABI.\n");
        out.push_str("    def to_unsafe : Void*\n      @handle\n    end\n");
        if !borrowed_handles.contains(&ty.name) {
            out.push_str("    def finalize\n");
            out.push_str(&format!("      {lib_name}.{free}(@handle) unless @handle.null?\n"));
            out.push_str("    end\n");
        }

        for m in &ty.methods {
            if m.binding_excluded {
                continue;
            }
            out.push_str(&Self::gen_opaque_method(
                ty,
                m,
                &type_snake,
                lib_name,
                opaque,
                ffi_structs,
            ));
        }

        // Streaming methods owned by this type → fiber-fed channels.
        for spec in streaming.iter().filter(|s| s.owner == ty.name) {
            out.push_str(&Self::gen_stream_method(
                spec,
                &type_snake,
                lib_name,
                Some("@handle"),
                opaque,
                ffi_structs,
            ));
        }

        // Async method adapters owned by this type → Crystal wrapper methods
        // that marshal params/results across the JSON-string C ABI.
        for spec in async_methods.iter().filter(|s| s.owner == ty.name) {
            out.push_str(&Self::gen_adapter_method(
                spec,
                &type_snake,
                lib_name,
            ));
        }

        out.push_str("  end\n");
        out
    }

    /// Emit a streaming method returning a `Channel(Item)` fed by a fiber that
    /// drives the FFI iterator (`_start`/`_next`/`_free`) — Crystal's idiomatic
    /// concurrency: `spawn` + `Channel`.
    fn gen_stream_method(
        spec: &StreamSpec,
        type_snake: &str,
        lib_name: &str,
        receiver: Option<&str>,
        opaque: &HashSet<String>,
        ffi_structs: &HashSet<String>,
    ) -> String {
        let method = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &spec.method);
        let item = crystal_type_name(&spec.item);
        let item_snake = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &spec.item);
        let base = format!("{type_snake}_{method}");
        // High-level signature params + marshalled start-call args.
        let sig_params = spec
            .params
            .iter()
            .map(|(n, ty)| {
                let pn = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, n);
                format!("{pn} : {}", crystal_type(ty))
            })
            .collect::<Vec<_>>()
            .join(", ");
        // Instance method on an opaque handle (receiver = "@handle"), or a free
        // module-level function (`def self.…`, no receiver).
        let decl = if receiver.is_some() {
            method.clone()
        } else {
            format!("self.{method}")
        };
        let sig = if sig_params.is_empty() {
            format!("def {decl} : Channel({item})")
        } else {
            format!("def {decl}({sig_params}) : Channel({item})")
        };
        let mut start_args: Vec<String> = receiver.map(|r| r.to_string()).into_iter().collect();
        let mut stream_setup = String::new();
        let mut stream_teardown = String::new();
        for (n, ty) in &spec.params {
            let pn = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, n);
            if let TypeRef::Named(type_name) = ty {
                if is_ffi_struct(type_name, ffi_structs) {
                    let type_snake =
                        public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, type_name);
                    let handle_var = format!("__handle_{pn}");
                    stream_setup.push_str(&format!(
                        "    {handle_var} = {lib_name}.{type_snake}_from_json({pn}.to_json)\n"
                    ));
                    stream_teardown.push_str(&format!("    {lib_name}.{type_snake}_free({handle_var})\n"));
                    start_args.push(handle_var);
                    continue;
                }
            }
            start_args.push(marshal_value(&pn, ty, opaque));
        }
        let mut b = String::new();
        b.push_str(&format!("\n    # Stream of `{item}` items over a fiber-fed channel.\n"));
        b.push_str(&format!("    {sig}\n"));
        b.push_str(&stream_setup);
        b.push_str(&format!(
            "      __handle = {lib_name}.{base}_start({})\n",
            start_args.join(", ")
        ));
        b.push_str(&format!("      __ch = Channel({item}).new\n"));
        b.push_str(&format!(
            "      raise \"{lib_name}.{base}_start returned a null iterator\" if __handle.null?\n"
        ));
        b.push_str("      spawn do\n");
        b.push_str("        begin\n");
        b.push_str("          loop do\n");
        b.push_str(&format!("            __chunk = {lib_name}.{base}_next(__handle)\n"));
        b.push_str("            break if __chunk.null?\n");
        let item_struct_name = crystal_type_name(&spec.item);
        // If the item type is an FFI struct, cast the Void* chunk before calling
        // the typed to_json/free; otherwise use the Void*-accepting lib bindings.
        if ffi_structs.contains(&spec.item) {
            b.push_str(&format!(
                "            __chunk_ptr = __chunk.as({lib_name}::{item_struct_name}*)\n"
            ));
            b.push_str(&format!(
                "            __jp = {lib_name}.{item_snake}_to_json(__chunk_ptr)\n"
            ));
            b.push_str("            if __jp.null?\n");
            b.push_str(&format!(
                "              {lib_name}.{item_snake}_free(__chunk_ptr)\n"
            ));
            b.push_str("              break\n");
            b.push_str("            end\n");
            b.push_str("            __json = String.new(__jp)\n");
            b.push_str(&format!("            {lib_name}.free_string(__jp)\n"));
            b.push_str(&format!(
                "            {lib_name}.{item_snake}_free(__chunk_ptr)\n"
            ));
        } else {
            b.push_str(&format!(
                "            __jp = {lib_name}.{item_snake}_to_json(__chunk)\n"
            ));
            b.push_str("            if __jp.null?\n");
            b.push_str(&format!(
                "              {lib_name}.{item_snake}_free(__chunk)\n"
            ));
            b.push_str("              break\n");
            b.push_str("            end\n");
            b.push_str("            __json = String.new(__jp)\n");
            b.push_str(&format!("            {lib_name}.free_string(__jp)\n"));
            b.push_str(&format!(
                "            {lib_name}.{item_snake}_free(__chunk)\n"
            ));
        }
        b.push_str(&format!("            __ch.send({item}.from_json(__json))\n"));
        b.push_str("          end\n");
        b.push_str("        ensure\n");
        b.push_str(&format!("          {lib_name}.{base}_free(__handle)\n"));
        b.push_str(&stream_teardown);
        b.push_str("          __ch.close\n");
        b.push_str("        end\n");
        b.push_str("      end\n");
        b.push_str("      __ch\n");
        b.push_str("    end\n");
        b
    }

    /// Emit an async adapter method on an opaque handle type.
    /// Generates a Crystal wrapper that marshals params/results across the
    /// JSON-string C ABI, mirroring the pattern of `gen_opaque_method`.
    fn gen_adapter_method(
        spec: &AsyncMethodSpec,
        type_snake: &str,
        lib_name: &str,
    ) -> String {
        let method = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &spec.method);
        let ret_ty = crystal_type(&spec.return_type);

        // High-level Crystal parameter types.
        let sig_params = spec
            .params
            .iter()
            .map(|(n, ty)| {
                let pn = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, n);
                let base_ty = crystal_type(ty);
                let is_opt = base_ty.ends_with('?');
                let mut ty_s = base_ty.into_owned();
                if is_opt && !ty_s.ends_with('?') {
                    ty_s.push('?');
                }
                // Optional params default to nil so callers can omit them.
                if is_opt {
                    format!("{pn} : {ty_s} = nil")
                } else {
                    format!("{pn} : {ty_s}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");

        let ret_annot = if matches!(spec.return_type, TypeRef::Unit) && spec.error_type.is_none() {
            "Nil".to_string()
        } else {
            ret_ty.into_owned()
        };

        // Build FFI call args: the C `_json` wrapper takes JSON string params.
        // Pass `.to_json` for struct types, and the param directly for strings.
        // Optional (nilable) struct params pass a real null pointer when nil (the
        // wrapper treats null as `None`); non-nil values are JSON strings bound to
        // a local first so the String stays alive for the FFI call.
        let mut args = vec!["@handle".to_string()];
        let mut setup = String::new();
        let teardown = String::new();
        for (n, ty) in &spec.params {
            let pn = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, n);
            if matches!(ty, TypeRef::String) {
                args.push(pn);
            } else if crystal_type(ty).ends_with('?') {
                // Nilable struct: pass the C string when set, null pointer when nil.
                let json_var = format!("__json_{pn}");
                setup.push_str(&format!("    {json_var} = {pn}.nil? ? nil : {pn}.not_nil!.to_json\n"));
                args.push(json_var);
            } else {
                // The FFI `_json` function deserializes the JSON string internally,
                // so pass the Crystal object's .to_json() output directly.
                let json_var = format!("__json_{pn}");
                setup.push_str(&format!("    {json_var} = {pn}.to_json\n"));
                args.push(json_var);
            }
        }

        // The `_json` C wrapper returns a JSON C string directly (no struct
        // pointer round-trip). Use the JSON-string ABI pattern.
        let call = format!("{lib_name}.{type_snake}_{method}_json({})", args.join(", "));
        let mut body = String::new();
        body.push_str(&format!("    __ptr = {call}\n"));
        if matches!(spec.return_type, TypeRef::Unit) {
            body.push_str("    return nil if __ptr.null?\n");
        } else {
            body.push_str(&format!(
                "    raise \"{lib_name}.{type_snake}_{method}_json returned a null pointer\" if __ptr.null?\n"
            ));
        }
        body.push_str("    __json = String.new(__ptr)\n");
        body.push_str(&format!("    {lib_name}.free_string(__ptr)\n"));
        match &spec.return_type {
            TypeRef::Unit => body.push_str("    nil\n"),
            // Bytes crosses as a JSON array of integers; Slice has no from_json.
            // Matches both TypeRef::Bytes and path-qualified Named types that map
            // to the `Bytes` alias (e.g. `bytes::Bytes`).
            TypeRef::Bytes => body.push_str(
                "    __arr = Array(UInt8).from_json(__json)\n    Bytes.new(__arr.size) { |i| __arr[i] }\n",
            ),
            TypeRef::Optional(_) => {
                let inner = crystal_type(&spec.return_type);
                body.push_str(&format!("    {inner}.from_json(__json)\n"));
            }
            _ => {
                let ty = crystal_type(&spec.return_type);
                if ty == "Bytes" {
                    body.push_str(
                        "    __arr = Array(UInt8).from_json(__json)\n    Bytes.new(__arr.size) { |i| __arr[i] }\n",
                    );
                } else {
                    body.push_str(&format!("    {ty}.from_json(__json)\n"));
                }
            }
        }

        let mut out = String::new();
        let doc = format!("Call `{method}` via the FFI C ABI.");
        out.push_str(&format!("\n    # {doc}\n"));
        out.push_str(&format!("    def {method}({sig_params}) : {ret_annot}\n"));
        out.push_str(&setup);
        out.push_str(&body);
        out.push_str(&teardown);
        out.push_str("    end\n");
        out
    }

    /// Emit one instance or static method on an opaque handle wrapper class.
    fn gen_opaque_method(
        ty: &TypeDef,
        m: &crate::core::ir::MethodDef,
        type_snake: &str,
        lib_name: &str,
        opaque: &HashSet<String>,
        ffi_structs: &HashSet<String>,
    ) -> String {
        let method = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &m.name);
        let method_snake = &method;
        let sig_params = m
            .params
            .iter()
            .map(|p| {
                let name = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, &p.name);
                // Nilable at the Crystal type level (Option<T> or flagged optional).
                let base_ty = crystal_type(&p.ty);
                let is_opt = p.optional || base_ty.ends_with('?');
                let mut ty = base_ty.into_owned();
                if is_opt && !ty.ends_with('?') {
                    ty.push('?');
                }
                // Optional params default to nil so callers can omit them.
                if is_opt {
                    format!("{name} : {ty} = nil")
                } else {
                    format!("{name} : {ty}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let mut args: Vec<String> = if m.is_static {
            Vec::new()
        } else {
            vec!["@handle".to_string()]
        };
        let mut setup = String::new();
        let mut teardown = String::new();
        for p in &m.params {
            let pname = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, &p.name);
            // Nilable at the Crystal type level (Option<T> or flagged optional).
            let base_ty = crystal_type(&p.ty);
            let p_opt = p.optional || base_ty.ends_with('?');
            if let TypeRef::Named(type_name) = &p.ty {
                if is_ffi_struct(type_name, ffi_structs) {
                    let type_snake = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, type_name);
                    let handle_var = format!("__handle_{pname}");
                    if p_opt {
                        // Pass a null pointer for nil so the Rust side uses defaults.
                        setup.push_str(&format!(
                            "    {handle_var} = {pname}.nil? ? Pointer({lib_name}::{type_name}).null : {lib_name}.{type_snake}_from_json({pname}.not_nil!.to_json)\n"
                        ));
                        teardown.push_str(&format!(
                            "    {lib_name}.{type_snake}_free({handle_var}) unless {handle_var}.null?\n"
                        ));
                    } else {
                        setup.push_str(&format!(
                            "    {handle_var} = {lib_name}.{type_snake}_from_json({pname}.to_json)\n"
                        ));
                        teardown.push_str(&format!("    {lib_name}.{type_snake}_free({handle_var})\n"));
                    }
                    args.push(handle_var);
                    continue;
                }
            }
            args.push(marshal_value(&pname, &p.ty, opaque));
        }
        let call = format!("{lib_name}.{type_snake}_{method_snake}({})", args.join(", "));
        let ret_annot = if matches!(m.return_type, TypeRef::Unit) && m.error_type.is_none() {
            "Nil".to_string()
        } else {
            crystal_type(&m.return_type).into_owned()
        };
        let label = format!("{type_snake}_{method_snake}");
        let call_body = Self::gen_call_body(
            &call,
            &m.return_type,
            m.error_type.as_deref(),
            lib_name,
            opaque,
            ffi_structs,
            &label,
        );
        let mut body = setup;
        if teardown.is_empty() {
            body.push_str(&call_body);
        } else {
            // When teardown frees param FFI structs after the call, the call_body's
            // last expression (e.g. `CrawlEngineHandle.new(__ptr)`) must be captured
            // before teardown runs so the return type is preserved.
            body.push_str("    __result = begin\n");
            for line in call_body.lines() {
                body.push_str(&format!("      {line}\n"));
            }
            body.push_str("    end\n");
            body.push_str(&teardown);
            body.push_str("    __result\n");
        }
        let decl = if m.is_static {
            format!("self.{method}")
        } else {
            method.clone()
        };
        let _ = ty;
        let mut doc_buf = String::new();
        crate::codegen::doc_emission::emit_crystal_doc(&mut doc_buf, &m.doc, "    ");
        format!("{doc_buf}    def {decl}({sig_params}) : {ret_annot}\n{body}    end\n")
    }

    /// Emit a Crystal type for an enum.
    ///
    /// - all-unit → a plain Crystal `enum`.
    /// - externally-tagged with unit + single-payload (newtype) variants →
    ///   an abstract-class hierarchy that round-trips serde's externally-tagged JSON
    ///   (`"Unit"` for unit variants, `{"Variant": payload}` for newtype variants).
    /// - anything else (struct/multi-tuple variants, internally/adjacently-tagged,
    ///   untagged) → skipped with an explanatory note (pending).
    fn gen_enum(en: &EnumDef, api: &ApiSurface, module_name: &str) -> String {
        let name = crystal_type_name(&en.name);
        let variants: Vec<&crate::core::ir::EnumVariant> = en.variants.iter().filter(|v| !v.binding_excluded).collect();

        let is_unit =
            |v: &crate::core::ir::EnumVariant| v.fields.is_empty() && !v.is_tuple && !v.originally_had_data_fields;

        // Plain unit enum.
        if variants.iter().all(|v| is_unit(v)) {
            let mut out = String::new();
            out.push_str(&Self::doc_or_blank(&en.doc));
            out.push_str(&format!("  enum {name}\n"));
            for v in &variants {
                let vname = public_host_identifier(Language::Crystal, PublicIdentifierKind::EnumVariant, &v.name);
                out.push_str(&format!("    {vname}\n"));
            }
            out.push_str("  end\n");
            return out;
        }

        if en.serde_untagged {
            return Self::gen_untagged(en, &name, &variants, is_unit);
        }
        if let Some(tag) = en.serde_tag.as_deref() {
            return Self::gen_internally_tagged(en, &name, &variants, tag, is_unit, api, module_name);
        }

        Self::gen_tagged_union(en, &name, &variants, is_unit)
    }

    /// Emit an abstract-class hierarchy for an internally-tagged enum
    /// (`#[serde(tag = "...")]` → `{"<tag>":"Variant", ...fields}`), using Crystal's
    /// native `use_json_discriminator` for dispatch. Each subclass re-emits the tag
    /// field (with a default) so `to_json` round-trips.
    /// Emit a `JSON::Serializable` converter for a unit enum whose wire (serde)
    /// values differ from the Crystal variant names (e.g. `"og:image"` → OgImage).
    /// Crystal's default `Enum.parse` only accepts the variant name and its
    /// underscore/camel normalizations, so these need explicit mapping.
    fn gen_enum_converter(en: &EnumDef) -> String {
        let name = crystal_type_name(&en.name);
        let mut out = String::new();
        out.push_str(&format!("  module {name}Converter\n"));
        out.push_str(&format!("    def self.from_json(pull : JSON::PullParser) : {name}\n"));
        out.push_str(&format!("      case pull.read_string\n"));
        for v in en.variants.iter().filter(|v| !v.binding_excluded) {
            let vname = public_host_identifier(Language::Crystal, PublicIdentifierKind::EnumVariant, &v.name);
            let wire = wire_variant_value(&v.name, v.serde_rename.as_deref(), en.serde_rename_all.as_deref());
            out.push_str(&format!("      when {wire:?} then {name}::{vname}\n"));
        }
        out.push_str(&format!("      else pull.raise \"Unknown {name} value\"\n"));
        out.push_str("      end\n");
        out.push_str("    end\n");
        out.push_str(&format!("    def self.to_json(value : {name}, json : JSON::Builder)\n"));
        out.push_str("      json.string(case value\n");
        for v in en.variants.iter().filter(|v| !v.binding_excluded) {
            let vname = public_host_identifier(Language::Crystal, PublicIdentifierKind::EnumVariant, &v.name);
            let wire = wire_variant_value(&v.name, v.serde_rename.as_deref(), en.serde_rename_all.as_deref());
            out.push_str(&format!("      when {name}::{vname} then {wire:?}\n"));
        }
        out.push_str("      end)\n");
        out.push_str("    end\n");
        out.push_str("  end\n");
        out
    }

    fn gen_internally_tagged(
        en: &EnumDef,
        name: &str,
        variants: &[&crate::core::ir::EnumVariant],
        tag: &str,
        is_unit: impl Fn(&crate::core::ir::EnumVariant) -> bool,
        api: &ApiSurface,
        module_name: &str,
    ) -> String {
        let wire = |v: &crate::core::ir::EnumVariant| {
            wire_variant_value(&v.name, v.serde_rename.as_deref(), en.serde_rename_all.as_deref())
        };
        let class_of = |v: &crate::core::ir::EnumVariant| crystal_type_name(&v.name);
        // Crystal identifier for the discriminator getter (keyword-escaped).
        let tag_ident = public_host_identifier(Language::Crystal, PublicIdentifierKind::Field, tag);

        let mut out = String::new();
        out.push_str(&Self::doc_or_blank(&en.doc));
        out.push_str(&format!("  abstract class {name}\n"));
        out.push_str("    include JSON::Serializable\n");
        let mapping = variants
            .iter()
            .map(|v| format!("{:?} => {name}::{}", wire(v), class_of(v)))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("    use_json_discriminator {tag:?}, {{{mapping}}}\n"));
        out.push_str("  end\n");

        for v in variants {
            let vclass = class_of(v);
            out.push_str(&format!("\n  class {name}::{vclass} < {name}\n"));
            out.push_str("    include JSON::Serializable\n");
            // Re-emit the discriminator so serialization includes the tag.
            out.push_str(&format!("    @[JSON::Field(key: {tag:?})]\n"));
            out.push_str(&format!("    getter {tag_ident} : String = {:?}\n", wire(v)));
            if !is_unit(v) {
                if v.is_tuple && v.fields.len() == 1 {
                    let inner = &v.fields[0];
                    if let TypeRef::Named(inner_name) = &inner.ty {
                        if let Some(inner_type) = api.types.iter().find(|t| t.name == *inner_name) {
                            for f in &inner_type.fields {
                                if f.binding_excluded {
                                    continue;
                                }
                                let getter =
                                    public_host_identifier(Language::Crystal, PublicIdentifierKind::Field, &f.name);
                                let key = wire_field_name(
                                    &f.name,
                                    f.serde_rename.as_deref(),
                                    inner_type.serde_rename_all.as_deref(),
                                );
                                let mut ty = crystal_type(&f.ty).into_owned();
                                if f.optional && !ty.ends_with('?') {
                                    ty.push('?');
                                }
                                if key != getter {
                                    out.push_str(&format!("    @[JSON::Field(key: {key:?})]\n"));
                                }
                                if !f.optional {
                                    if let Some(def) = type_based_default_expr(&f.ty)
                                        .or_else(|| crystal_default_expr(&f.typed_default, &f.default))
                                    {
                                        out.push_str(&format!("    getter {getter} : {ty} = {def}\n"));
                                        continue;
                                    }
                                }
                                out.push_str(&format!("    getter {getter} : {ty}\n"));
                            }
                        }
                    } else {
                        let ty = crystal_type(&inner.ty).into_owned();
                        out.push_str(&format!("    getter value : {ty}\n"));
                        out.push_str(&format!("    def initialize(@value : {ty})\n    end\n"));
                    }
                } else {
                    for f in &v.fields {
                        if f.binding_excluded {
                            continue;
                        }
                        let getter = public_host_identifier(Language::Crystal, PublicIdentifierKind::Field, &f.name);
                        let key = wire_field_name(&f.name, f.serde_rename.as_deref(), en.serde_rename_all.as_deref());
                        let mut ty = variant_field_type(&f.ty, module_name).into_owned();
                        if f.optional && !ty.ends_with('?') {
                            ty.push('?');
                        }
                        if key != getter {
                            out.push_str(&format!("    @[JSON::Field(key: {key:?})]\n"));
                        }
                        // Non-nilable variant fields get a type-based getter default so
                        // partial JSON (and the enum's default constructor) round-trip.
                        if !f.optional {
                            if let Some(def) = type_based_default_expr(&f.ty)
                                .or_else(|| crystal_default_expr(&f.typed_default, &f.default))
                            {
                                out.push_str(&format!("    getter {getter} : {ty} = {def}\n"));
                                continue;
                            }
                        }
                        out.push_str(&format!("    getter {getter} : {ty}\n"));
                    }
                }
            }
            out.push_str("  end\n");
        }
        out
    }

    /// Emit an abstract-class hierarchy for an untagged enum (`#[serde(untagged)]`):
    /// each variant serializes as a bare payload and deserialization tries each
    /// variant in declaration order (matching serde), via a `JSON::Any` snapshot so
    /// the pull parser does not need to rewind.
    fn gen_untagged(
        en: &EnumDef,
        name: &str,
        variants: &[&crate::core::ir::EnumVariant],
        is_unit: impl Fn(&crate::core::ir::EnumVariant) -> bool,
    ) -> String {
        let class_of = |v: &crate::core::ir::EnumVariant| crystal_type_name(&v.name);
        let mut out = String::new();
        out.push_str(&Self::doc_or_blank(&en.doc));
        out.push_str(&format!("  abstract class {name}\n"));
        out.push_str(&format!("    def self.new(pull : ::JSON::PullParser) : {name}\n"));
        out.push_str("      __raw = ::JSON::Any.new(pull).to_json\n");
        for v in variants {
            out.push_str("      begin\n");
            out.push_str(&format!("        return {name}::{}.from_json(__raw)\n", class_of(v)));
            out.push_str("      rescue ::JSON::ParseException\n");
            out.push_str("      end\n");
        }
        out.push_str(&format!(
            "      raise ::JSON::ParseException.new(\"no {name} variant matched\", 0, 0)\n"
        ));
        out.push_str("    end\n\n");
        out.push_str(&format!("    def self.from_json(string : String) : {name}\n"));
        out.push_str("      new(::JSON::PullParser.new(string))\n");
        out.push_str("    end\n");
        out.push_str("    abstract def to_json(json : ::JSON::Builder)\n");
        out.push_str("  end\n");
        for v in variants {
            out.push_str(&Self::gen_untagged_variant(name, &class_of(v), v, &is_unit));
        }
        out
    }

    /// Emit one concrete variant subclass of an untagged enum (bare payload).
    fn gen_untagged_variant(
        name: &str,
        vclass: &str,
        v: &crate::core::ir::EnumVariant,
        is_unit: &impl Fn(&crate::core::ir::EnumVariant) -> bool,
    ) -> String {
        let mut out = format!("\n  class {name}::{vclass} < {name}\n");

        if is_unit(v) {
            out.push_str(&format!("    def self.from_json(string : String) : {name}::{vclass}\n"));
            out.push_str("      pull = ::JSON::PullParser.new(string)\n");
            out.push_str("      pull.read_null\n");
            out.push_str("      new\n");
            out.push_str("    end\n");
            out.push_str("    def to_json(json : ::JSON::Builder)\n");
            out.push_str("      json.null\n");
            out.push_str("    end\n  end\n");
            return out;
        }

        // Struct variant → lean on JSON::Serializable (bare object in/out).
        if !v.is_tuple {
            out.push_str("    include JSON::Serializable\n");
            for f in &v.fields {
                if f.binding_excluded {
                    continue;
                }
                let getter = public_host_identifier(Language::Crystal, PublicIdentifierKind::Field, &f.name);
                let key = wire_field_name(&f.name, f.serde_rename.as_deref(), None);
                let mut ty = crystal_type(&f.ty).into_owned();
                if f.optional && !ty.ends_with('?') {
                    ty.push('?');
                }
                if key != getter {
                    out.push_str(&format!("    @[JSON::Field(key: {key:?})]\n"));
                }
                out.push_str(&format!("    getter {getter} : {ty}\n"));
            }
            out.push_str("  end\n");
            return out;
        }

        // Tuple variant (incl. single-field newtype): bare payload / array.
        let types: Vec<String> = v
            .fields
            .iter()
            .map(|f| {
                let mut ty = crystal_type(&f.ty).into_owned();
                if f.optional && !ty.ends_with('?') {
                    ty.push('?');
                }
                ty
            })
            .collect();
        if types.len() == 1 {
            out.push_str(&format!("    getter value : {}\n", types[0]));
            out.push_str(&format!("    def initialize(@value : {})\n    end\n", types[0]));
            out.push_str(&format!("    def self.from_json(string : String) : {name}::{vclass}\n"));
            out.push_str(&format!("      new({}.from_json(string))\n", strip_nil(&types[0])));
            out.push_str("    end\n");
            out.push_str("    def to_json(json : ::JSON::Builder)\n");
            out.push_str("      @value.to_json(json)\n");
            out.push_str("    end\n  end\n");
            return out;
        }
        for (i, ty) in types.iter().enumerate() {
            out.push_str(&format!("    getter field{i} : {ty}\n"));
        }
        let init_args = types
            .iter()
            .enumerate()
            .map(|(i, ty)| format!("@field{i} : {ty}"))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("    def initialize({init_args})\n    end\n"));
        out.push_str(&format!("    def self.from_json(string : String) : {name}::{vclass}\n"));
        out.push_str("      pull = ::JSON::PullParser.new(string)\n");
        out.push_str("      pull.read_begin_array\n");
        for (i, ty) in types.iter().enumerate() {
            out.push_str(&format!("      __v{i} = {}.new(pull)\n", strip_nil(ty)));
        }
        out.push_str("      pull.read_end_array\n");
        let args = (0..types.len())
            .map(|i| format!("__v{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("      new({args})\n"));
        out.push_str("    end\n");
        out.push_str("    def to_json(json : ::JSON::Builder)\n");
        out.push_str("      json.array do\n");
        for i in 0..types.len() {
            out.push_str(&format!("        @field{i}.to_json(json)\n"));
        }
        out.push_str("      end\n");
        out.push_str("    end\n  end\n");
        out
    }

    /// Emit an abstract-class hierarchy for an externally-tagged enum, covering
    /// unit, tuple (incl. single-field newtype), and struct variants.
    fn gen_tagged_union(
        en: &EnumDef,
        name: &str,
        variants: &[&crate::core::ir::EnumVariant],
        is_unit: impl Fn(&crate::core::ir::EnumVariant) -> bool,
    ) -> String {
        let wire = |v: &crate::core::ir::EnumVariant| {
            wire_variant_value(&v.name, v.serde_rename.as_deref(), en.serde_rename_all.as_deref())
        };
        let class_of = |v: &crate::core::ir::EnumVariant| crystal_type_name(&v.name);

        let mut out = String::new();
        out.push_str(&Self::doc_or_blank(&en.doc));
        out.push_str(&format!("  abstract class {name}\n"));

        // from_json dispatch: bare string → unit variant; single-key object → data variant.
        out.push_str(&format!("    def self.new(pull : ::JSON::PullParser) : {name}\n"));
        out.push_str("      case pull.kind\n");
        out.push_str("      when .string?\n");
        out.push_str("        __tag = pull.read_string\n");
        out.push_str("        case __tag\n");
        for v in variants.iter().filter(|v| is_unit(v)) {
            out.push_str(&format!(
                "        when {:?} then return {name}::{}.new\n",
                wire(v),
                class_of(v)
            ));
        }
        out.push_str(&format!(
            "        else raise ::JSON::ParseException.new(\"unknown {name} variant: #{{__tag}}\", *pull.location)\n"
        ));
        out.push_str("        end\n");
        out.push_str("      when .begin_object?\n");
        out.push_str(&format!("        __result : {name}? = nil\n"));
        out.push_str("        pull.read_object do |__key|\n");
        out.push_str("          case __key\n");
        for v in variants.iter().filter(|v| !is_unit(v)) {
            out.push_str(&format!(
                "          when {:?} then __result = {name}::{}.new(pull)\n",
                wire(v),
                class_of(v)
            ));
        }
        out.push_str("          else pull.skip\n");
        out.push_str("          end\n");
        out.push_str("        end\n");
        out.push_str(&format!(
            "        return __result || raise ::JSON::ParseException.new(\"empty {name} object\", *pull.location)\n"
        ));
        out.push_str("      else\n");
        out.push_str(&format!(
            "        raise ::JSON::ParseException.new(\"invalid {name} JSON\", *pull.location)\n"
        ));
        out.push_str("      end\n");
        out.push_str("    end\n\n");
        out.push_str(&format!("    def self.from_json(string : String) : {name}\n"));
        out.push_str("      new(::JSON::PullParser.new(string))\n");
        out.push_str("    end\n\n");
        out.push_str("    abstract def to_json(json : ::JSON::Builder)\n");
        out.push_str("  end\n");

        for v in variants {
            out.push_str(&Self::gen_variant_class(name, &class_of(v), &wire(v), v, &is_unit));
        }
        out
    }

    /// Emit one concrete variant subclass of a tagged-union enum.
    fn gen_variant_class(
        name: &str,
        vclass: &str,
        wire: &str,
        v: &crate::core::ir::EnumVariant,
        is_unit: &impl Fn(&crate::core::ir::EnumVariant) -> bool,
    ) -> String {
        let mut out = format!("\n  class {name}::{vclass} < {name}\n");
        if is_unit(v) {
            out.push_str("    def to_json(json : ::JSON::Builder)\n");
            out.push_str(&format!("      json.string({wire:?})\n"));
            out.push_str("    end\n  end\n");
            return out;
        }
        struct VField {
            getter: String,
            ty: String,
            json_key: Option<String>,
        }
        let fields: Vec<VField> = v
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let mut ty = crystal_type(&f.ty).into_owned();
                if f.optional && !ty.ends_with('?') {
                    ty.push('?');
                }
                if v.is_tuple {
                    let getter = if v.fields.len() == 1 {
                        "value".to_string()
                    } else {
                        format!("field{i}")
                    };
                    VField {
                        getter,
                        ty,
                        json_key: None,
                    }
                } else {
                    let getter = public_host_identifier(Language::Crystal, PublicIdentifierKind::Field, &f.name);
                    let key = wire_field_name(&f.name, f.serde_rename.as_deref(), None);
                    VField {
                        getter,
                        ty,
                        json_key: Some(key),
                    }
                }
            })
            .collect();
        for f in &fields {
            out.push_str(&format!("    getter {} : {}\n", f.getter, f.ty));
        }
        let init_args = fields
            .iter()
            .map(|f| format!("@{} : {}", f.getter, f.ty))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("    def initialize({init_args})\n    end\n"));
        // Reader: self.new(pull) consuming the payload.
        out.push_str(&format!(
            "    def self.new(pull : ::JSON::PullParser) : {name}::{vclass}\n"
        ));
        if v.is_tuple && v.fields.len() == 1 {
            out.push_str(&format!("      __v = {}.new(pull)\n", strip_nil(&fields[0].ty)));
            out.push_str("      new(__v)\n");
        } else if v.is_tuple {
            out.push_str("      pull.read_begin_array\n");
            for (i, f) in fields.iter().enumerate() {
                out.push_str(&format!("      __v{i} = {}.new(pull)\n", strip_nil(&f.ty)));
            }
            out.push_str("      pull.read_end_array\n");
            let args = (0..fields.len())
                .map(|i| format!("__v{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("      new({args})\n"));
        } else {
            for f in &fields {
                out.push_str(&format!("      __{} : {} = nil\n", f.getter, nilable(&f.ty)));
            }
            out.push_str("      pull.read_object do |__k|\n");
            out.push_str("        case __k\n");
            for f in &fields {
                let key = f.json_key.as_deref().unwrap_or(&f.getter);
                out.push_str(&format!(
                    "        when {key:?} then __{} = {}.new(pull)\n",
                    f.getter,
                    strip_nil(&f.ty)
                ));
            }
            out.push_str("        else pull.skip\n");
            out.push_str("        end\n");
            out.push_str("      end\n");
            let args = fields
                .iter()
                .map(|f| {
                    if f.ty.ends_with('?') {
                        format!("__{}", f.getter)
                    } else {
                        format!("__{}.not_nil!", f.getter)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("      new({args})\n"));
        }
        out.push_str("    end\n");
        // Writer: wrap payload under the variant tag.
        out.push_str("    def to_json(json : ::JSON::Builder)\n");
        out.push_str("      json.object do\n");
        out.push_str(&format!("        json.field({wire:?}) do\n"));
        if v.is_tuple && v.fields.len() == 1 {
            out.push_str("          @value.to_json(json)\n");
        } else if v.is_tuple {
            out.push_str("          json.array do\n");
            for f in &fields {
                out.push_str(&format!("            @{}.to_json(json)\n", f.getter));
            }
            out.push_str("          end\n");
        } else {
            out.push_str("          json.object do\n");
            for f in &fields {
                let key = f.json_key.as_deref().unwrap_or(&f.getter);
                out.push_str(&format!(
                    "            json.field({key:?}) {{ @{}.to_json(json) }}\n",
                    f.getter
                ));
            }
            out.push_str("          end\n");
        }
        out.push_str("        end\n");
        out.push_str("      end\n");
        out.push_str("    end\n  end\n");
        out
    }

    /// Render a leading `# summary` comment (or a blank line) for a doc string.
    fn doc_or_blank(doc: &str) -> String {
        let mut out = String::new();
        out.push('\n');
        crate::codegen::doc_emission::emit_crystal_doc(&mut out, doc, "  ");
        out
    }

    /// Emit a Crystal exception subclass for a Rust error type.
    fn gen_error(err: &ErrorDef) -> String {
        let name = crystal_type_name(&err.name);
        let mut out = String::new();
        out.push('\n');
        crate::codegen::doc_emission::emit_crystal_doc(&mut out, &err.doc, "  ");
        out.push_str(&format!("  class {name} < Exception\n  end\n"));
        out
    }

    /// Generate a single snake_case wrapper method delegating to the lib fun.
    fn gen_wrapper_method(
        func: &FunctionDef,
        lib_name: &str,
        opaque: &HashSet<String>,
        ffi_structs: &HashSet<String>,
    ) -> String {
        let method = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &func.name);
        let ret_ty = crystal_type(&func.return_type);

        // Signature: high-level Crystal parameter types.
        let sig_params = func
            .params
            .iter()
            .map(|p| {
                let name = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, &p.name);
                let mut ty = crystal_type(&p.ty).into_owned();
                if p.optional && !ty.ends_with('?') {
                    ty.push('?');
                }
                // Optional params default to nil so callers can omit them
                // (matches Ruby/Python optional-param idioms).
                if p.optional {
                    format!("{name} : {ty} = nil")
                } else {
                    format!("{name} : {ty}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");

        let ret_annot = if matches!(func.return_type, TypeRef::Unit) && func.error_type.is_none() {
            "Nil".to_string()
        } else {
            ret_ty.into_owned()
        };

        let body = Self::gen_wrapper_body(func, lib_name, &method, opaque, ffi_structs);

        format!(
            "\n  # {doc}\n  def self.{method}({sig_params}) : {ret_annot}\n{body}  end\n",
            doc = func.doc.lines().next().unwrap_or_default().trim(),
        )
    }

    fn gen_wrapper_body(
        func: &FunctionDef,
        lib_name: &str,
        method: &str,
        opaque: &HashSet<String>,
        ffi_structs: &HashSet<String>,
    ) -> String {
        // Generate setup/teardown for FFI struct params that need `from_json`/`free`.
        let mut setup = String::new();
        let mut teardown = String::new();
        let mut call_parts: Vec<String> = Vec::new();

        for p in &func.params {
            let pname = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, &p.name);
            if let TypeRef::Named(type_name) = &p.ty {
                if is_ffi_struct(type_name, ffi_structs) {
                    let type_snake =
                        public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, type_name);
                    let handle_var = format!("__handle_{pname}");
                    // If the parameter is nilable (Crystal `T?`), check for nil and
                    // pass a null pointer to the FFI (the Rust side handles null as
                    // "use defaults" for config-like types).
                    if p.optional {
                        setup.push_str(&format!(
                            "    {handle_var} = {pname}.nil? ? Pointer({lib_name}::{type_name}).null : {lib_name}.{type_snake}_from_json({pname}.not_nil!.to_json)\n"
                        ));
                        teardown.push_str(&format!(
                            "    {lib_name}.{type_snake}_free({handle_var}) unless {handle_var}.null?\n"
                        ));
                    } else {
                        setup.push_str(&format!(
                            "    {handle_var} = {lib_name}.{type_snake}_from_json({pname}.to_json)\n"
                        ));
                        teardown.push_str(&format!("    {lib_name}.{type_snake}_free({handle_var})\n"));
                    }
                    call_parts.push(handle_var);
                    continue;
                }
            }
            call_parts.push(marshal_value(&pname, &p.ty, opaque));
        }

        let call = format!("{lib_name}.{method}({})", call_parts.join(", "));
        let mut body = setup;
        let call_body = Self::gen_call_body(
            &call,
            &func.return_type,
            func.error_type.as_deref(),
            lib_name,
            opaque,
            ffi_structs,
            method,
        );
        if teardown.is_empty() || matches!(func.return_type, TypeRef::Unit) {
            body.push_str(&call_body);
            body.push_str(&teardown);
        } else {
            // Splice teardown BEFORE the last expression (the return value)
            // so the method returns correctly, not the nil from teardown calls.
            let trimmed = call_body.trim_end();
            if let Some(last_nl) = trimmed.rfind('\n') {
                body.push_str(&call_body[..=last_nl]);
                body.push_str(&teardown);
                body.push_str(&call_body[last_nl + 1..]);
            } else {
                body.push_str(&call_body);
                body.push_str(&teardown);
            }
        }
        body
    }

    /// Emit the marshalling + return-decoding body for a call expression, shared by
    /// free-function wrappers and opaque instance/static methods.
    fn gen_call_body(
        call: &str,
        return_type: &TypeRef,
        error_type: Option<&str>,
        lib_name: &str,
        opaque: &HashSet<String>,
        ffi_structs: &HashSet<String>,
        label: &str,
    ) -> String {
        // Opaque return: the lib fun returns a raw handle pointer; wrap it in the
        // handle class (raising on null).
        if is_opaque_named(return_type, opaque) {
            let ty = crystal_type(return_type);
            let mut b = format!("    __ptr = {call}\n");
            b.push_str(&format!(
                "    raise \"{lib_name}.{label} returned a null pointer\" if __ptr.null?\n"
            ));
            b.push_str(&format!("    {ty}.new(__ptr)\n"));
            return b;
        }

        // FFI struct returns use `*_to_json`/`*_free` helpers instead of raw string.
        let is_struct_return = matches!(return_type, TypeRef::Named(n) if ffi_structs.contains(n));
        if is_struct_return {
            let type_name = match return_type {
                TypeRef::Named(n) => n,
                _ => unreachable!(),
            };
            let type_snake = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, type_name);
            let mut b = String::new();
            b.push_str(&format!("    __ptr = {call}\n"));
            // On a null pointer, surface the FFI's last-error context (the Rust
            // side records a message via set_last_error) instead of a bare raise.
            b.push_str("    if __ptr.null?\n");
            b.push_str(&format!(
                "      __ctx_ptr = {lib_name}.last_error_context\n"
            ));
            b.push_str("      raise String.new(__ctx_ptr) unless __ctx_ptr.null?\n");
            b.push_str(&format!(
                "      raise \"{lib_name}.{label} returned a null pointer\"\n"
            ));
            b.push_str("    end\n");
            b.push_str(&format!("    __json_ptr = {lib_name}.{type_snake}_to_json(__ptr)\n"));
            b.push_str(&format!("    {lib_name}.{type_snake}_free(__ptr)\n"));
            b.push_str("    __json = String.new(__json_ptr)\n");
            b.push_str(&format!("    {lib_name}.free_string(__json_ptr)\n"));
            let ty = crystal_type(return_type);
            b.push_str(&format!("    {ty}.from_json(__json)\n"));
            return b;
        }

        // Optional(Named(...)) struct return: nullable struct pointer, nil on null.
        // Only valid when infallible (no error type) — fallible optional struct returns
        // must use JSON-string ABI since null could mean error or None.
        let is_opt_struct_return = matches!(
            return_type,
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if ffi_structs.contains(n))
        );
        if is_opt_struct_return && error_type.is_none() {
            let inner_type = match return_type {
                TypeRef::Optional(inner) => inner,
                _ => unreachable!(),
            };
            let type_name = match inner_type.as_ref() {
                TypeRef::Named(n) => n,
                _ => unreachable!(),
            };
            let type_snake = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, type_name);
            let mut b = String::new();
            b.push_str(&format!("    __ptr = {call}\n"));
            b.push_str("    return nil if __ptr.null?\n");
            b.push_str(&format!("    __json_ptr = {lib_name}.{type_snake}_to_json(__ptr)\n"));
            b.push_str(&format!("    {lib_name}.{type_snake}_free(__ptr)\n"));
            b.push_str("    __json = String.new(__json_ptr)\n");
            b.push_str(&format!("    {lib_name}.free_string(__json_ptr)\n"));
            let ty = crystal_type(inner_type);
            b.push_str(&format!("    {ty}.from_json(__json)\n"));
            return b;
        }

        // For fallible scalar returns (e.g. `Result<usize, Error>`), the C ABI
        // returns the value directly and signals errors via `last_error_code`.
        // A Unit return (status-only, e.g. `Result<(), Error>` → `-> i32`) has a
        // Void FFI fun — call it without assigning the result.
        if error_type.is_some() && is_scalar(return_type) {
            if matches!(return_type, TypeRef::Unit) {
                let mut b = String::new();
                b.push_str(&format!("    {call}\n"));
                b.push_str(&format!("    __code = {lib_name}.last_error_code\n"));
                b.push_str("    if __code != 0\n");
                b.push_str(&format!(
                    "      __ctx_ptr = {lib_name}.last_error_context\n"
                ));
                b.push_str("      raise String.new(__ctx_ptr) unless __ctx_ptr.null?\n");
                b.push_str("      raise \"unknown error\"\n");
                b.push_str("    end\n");
                b.push_str("    nil\n");
                return b;
            }
            let mut b = String::new();
            b.push_str(&format!("    __result = {call}\n"));
            b.push_str(&format!(
                "    __code = {lib_name}.last_error_code\n"
            ));
            b.push_str("    if __code != 0\n");
            b.push_str(&format!(
                "      __ctx_ptr = {lib_name}.last_error_context\n"
            ));
            b.push_str("      raise String.new(__ctx_ptr) unless __ctx_ptr.null?\n");
            b.push_str("      raise \"unknown error\"\n");
            b.push_str("    end\n");
            b.push_str("    __result\n");
            return b;
        }

        // JSON-string ABI: complex/fallible returns come back as an owned C string.
        let returns_c_string = error_type.is_some() || !is_scalar(return_type);

        if !returns_c_string {
            if matches!(return_type, TypeRef::Unit) {
                return format!("    {call}\n    nil\n");
            }
            return format!("    {call}\n");
        }

        // A NUL pointer signals either a `None`/nil return (when the declared type is
        // nilable) or a failure. When the Crystal return type is not nilable, raise —
        // matching Crystal's exception-based error convention — so the method body
        // type-checks against its non-nilable return restriction.
        let nilable = matches!(return_type, TypeRef::Optional(_) | TypeRef::Unit);

        let mut b = String::new();
        b.push_str(&format!("    __ptr = {call}\n"));
        if nilable {
            b.push_str("    return nil if __ptr.null?\n");
        } else {
            b.push_str(&format!(
                "    raise \"{lib_name}.{label} returned a null pointer\" if __ptr.null?\n"
            ));
        }
        b.push_str("    __json = String.new(__ptr)\n");
        b.push_str(&format!("    {lib_name}.free_string(__ptr)\n"));
        match return_type {
            TypeRef::Unit => b.push_str("    nil\n"),
            TypeRef::String | TypeRef::Path | TypeRef::Char => b.push_str("    __json\n"),
            TypeRef::Bytes => {
                b.push_str("    __arr = Array(UInt8).from_json(__json)\n    Bytes.new(__arr.size) { |i| __arr[i] }\n")
            }
            TypeRef::Json => b.push_str("    JSON.parse(__json)\n"),
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::String | TypeRef::Path | TypeRef::Char => b.push_str("    __json\n"),
                TypeRef::Bytes => b.push_str(
                    "    __arr = Array(UInt8).from_json(__json)\n    Bytes.new(__arr.size) { |i| __arr[i] }\n",
                ),
                TypeRef::Json => b.push_str("    JSON.parse(__json)\n"),
                other => {
                    let ty = crystal_type(other);
                    b.push_str(&format!("    {ty}.from_json(__json)\n"));
                }
            },
            other => {
                let ty = crystal_type(other);
                b.push_str(&format!("    {ty}.from_json(__json)\n"));
            }
        }
        b
    }
}

/// Check whether a type reference (including nested Vec/Optional/Map) mentions
/// an excluded type name, so we can filter fields that would reference an
/// undefined constant.
fn type_ref_uses_excluded(ty: &TypeRef, excluded: &[String]) -> bool {
    match ty {
        TypeRef::Named(n) => excluded.contains(n),
        TypeRef::Optional(inner) => type_ref_uses_excluded(inner, excluded),
        TypeRef::Vec(inner) => type_ref_uses_excluded(inner, excluded),
        TypeRef::Map(k, v) => type_ref_uses_excluded(k, excluded) || type_ref_uses_excluded(v, excluded),
        _ => false,
    }
}

/// Check whether a type reference (including nested Vec/Optional/Map/Hash) involves
/// `Bytes`, which does not implement JSON::Serializable in Crystal stdlib.
fn field_type_contains_bytes(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Bytes => true,
        TypeRef::Optional(inner) => field_type_contains_bytes(inner),
        TypeRef::Vec(inner) => field_type_contains_bytes(inner),
        TypeRef::Map(k, v) => field_type_contains_bytes(k) || field_type_contains_bytes(v),
        _ => false,
    }
}

/// Convert a Rust `DefaultValue` (from `typed_default` or the raw `default` string) to
/// a Crystal expression suitable for `@[JSON::Field(default: ...)]`.
fn crystal_default_expr(typed_default: &Option<DefaultValue>, default: &Option<String>) -> Option<String> {
    // Prefer typed_default for precise Crystal representation.
    if let Some(td) = typed_default {
        return match td {
            DefaultValue::BoolLiteral(b) => Some(b.to_string()),
            DefaultValue::IntLiteral(n) => Some(n.to_string()),
            DefaultValue::FloatLiteral(f) => {
                let s = f.to_string();
                Some(if s.contains('.') { s } else { format!("{s}.0") })
            }
            DefaultValue::StringLiteral(s) => {
                let escaped = s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
                Some(format!("\"{escaped}\""))
            }
            DefaultValue::EnumVariant(_) | DefaultValue::Empty | DefaultValue::None => None,
        };
    }
    // Fall back to the raw Rust default expression string. Only emit simple literals.
    if let Some(s) = default {
        let trimmed = s.trim();
        match trimmed {
            "true" | "false" => Some(trimmed.to_string()),
            s if s.parse::<i64>().is_ok() => Some(s.to_string()),
            s if s.parse::<f64>().is_ok() => Some(s.to_string()),
            _ => None,
        }
    } else {
        None
    }
}

/// For a named type that is a Crystal struct with all-defaultable fields,
/// default to `Type.from_json("{}")`. This mirrors Rust's `Default` trait
/// for struct types that have `#[serde(default)]` on all their fields.
fn struct_default_expr(
    ty: &TypeRef,
    serde_tagged_enums: &HashSet<String>,
    serde_tagged_defaults: &HashMap<String, String>,
) -> Option<String> {
    match ty {
        TypeRef::Named(n) if serde_tagged_enums.contains(n) => {
            // Internally-tagged enums are abstract classes keyed on a discriminator,
            // so `from_json("{}")` fails (missing tag). Default to the `#[default]`
            // variant's wire value: `from_json("{\"<tag>\":\"<wire>\"}")`.
            let tn = crystal_type_name(n);
            let tag_default = serde_tagged_defaults.get(n)?;
            // tag_default is `"mode": "auto"`; embed it in a Crystal string literal
            // with escaped quotes so `from_json` sees `{"mode": "auto"}`.
            let escaped = tag_default.replace('"', "\\\"");
            Some(format!("{tn}.from_json(\"{{{escaped}}}\")"))
        }
        TypeRef::Named(n) => {
            let tn = crystal_type_name(n);
            Some(format!("{tn}.from_json(\"{{}}\")"))
        }
        _ => None,
    }
}

/// For a named type that is a Crystal unit enum, default to its first variant.
/// This mirrors Rust's `Default` impl for enums (first variant is default).
fn enum_default_expr(ty: &TypeRef, enum_first_variant: &HashMap<String, String>) -> Option<String> {
    match ty {
        TypeRef::Named(n) => enum_first_variant
            .get(n)
            .map(|first_var| format!("{n}::{}", first_var)),
        _ => None,
    }
}

/// For an externally-tagged enum (custom `new(pull)`) with a default unit
/// variant, emit its subclass constructor (e.g. `OutputFormat::Plain.new`).
fn external_enum_default_expr(ty: &TypeRef, external_defaults: &HashMap<String, String>) -> Option<String> {
    match ty {
        TypeRef::Named(n) => external_defaults.get(n).cloned(),
        _ => None,
    }
}

/// If a type (including its Option/Vec/Map wrappers) references a unit enum that
/// needs a JSON converter, return the enum's Crystal type name.
/// Map a variant field type to Crystal, fully-qualifying Named struct types with
/// the wrapper module. Inside a variant subclass (e.g. `ContentPart::ImageUrl`) a
/// bare `ImageUrl` reference would resolve to the enclosing class itself; the
/// module-qualified path always points at the real type.
fn variant_field_type(ty: &TypeRef, module_name: &str) -> Cow<'static, str> {
    if let TypeRef::Named(n) = ty {
        let base = crystal_type_name(n);
        if base != "String" {
            return Cow::Owned(format!("{module_name}::{base}"));
        }
    }
    crystal_type(ty)
}

fn enum_converter_for_type(ty: &TypeRef, enum_converters: &HashSet<String>) -> Option<String> {    match ty {
        TypeRef::Named(n) if enum_converters.contains(n) => Some(crystal_type_name(n)),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) | TypeRef::Map(_, inner) => {
            enum_converter_for_type(inner, enum_converters)
        }
        _ => None,
    }
}

/// Whether Crystal's default `Enum.parse` would accept `wire` for a variant whose
/// Crystal name is `vname`. Crystal normalizes the input to snake/camel/downcase
/// and matches against the variant name, so `"img"` matches `Img` but `"og:image"`
/// does not match `OgImage` (the colon is not a valid separator).
fn crystal_enum_parse_matches(wire: &str, vname: &str) -> bool {
    use heck::ToSnakeCase;
    let snake = vname.to_snake_case();
    wire == vname || wire.eq_ignore_ascii_case(vname) || wire == snake || wire == snake.to_ascii_lowercase()
}

/// Infer a Crystal default expression from the type when no explicit default
/// is available in the IR. This mirrors Rust's `Default` trait and `serde(default)`
/// so partial JSON configs work without specifying every field.
fn type_based_default_expr(ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => Some("false".to_string()),
            PrimitiveType::I8 | PrimitiveType::I16 | PrimitiveType::I32 | PrimitiveType::I64
            | PrimitiveType::U8 | PrimitiveType::U16 | PrimitiveType::U32 | PrimitiveType::U64
            | PrimitiveType::Usize => Some("0".to_string()),
            PrimitiveType::F32 | PrimitiveType::F64 => Some("0.0".to_string()),
            PrimitiveType::Isize => Some("0".to_string()),
        },
        TypeRef::String => Some("\"\"".to_string()),
        TypeRef::Vec(inner) => {
            let inner_ty = crystal_type(inner);
            Some(format!("[] of {inner_ty}"))
        }
        TypeRef::Map(k, v) => {
            let k_ty = crystal_type(k);
            let v_ty = crystal_type(v);
            Some(format!("{{}} of {k_ty} => {v_ty}"))
        }
        TypeRef::Json => Some("JSON::Any.new(nil)".to_string()),
        TypeRef::Duration => Some("0".to_string()),
        _ => None,
    }
}

/// Scalar types pass across the C ABI by value (no JSON marshalling).
fn is_scalar(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Primitive(_) | TypeRef::Duration | TypeRef::Unit)
}

/// Marshal a named value of a given type into its C-ABI call expression.
fn marshal_value(name: &str, ty: &TypeRef, opaque: &HashSet<String>) -> String {
    if is_scalar(ty) {
        name.to_string()
    } else if is_opaque_named(ty, opaque) {
        format!("{name}.to_unsafe")
    } else {
        match ty {
            // The FFI receives string params as raw `char*` (Crystal auto-converts
            // a String to a NUL-terminated pointer); JSON-encoding would double-quote.
            TypeRef::String | TypeRef::Char | TypeRef::Path => name.to_string(),
            // Bytes (Slice) has no FFI auto-conversion; serialize via Array first.
            TypeRef::Bytes => format!("{name}.to_a.to_json"),
            _ => format!("{name}.to_json"),
        }
    }
}

/// The Crystal `lib` C return type for a return/error pair.
fn lib_c_return(
    return_type: &TypeRef,
    error_type: Option<&str>,
    opaque: &HashSet<String>,
    ffi_structs: &HashSet<String>,
) -> String {
    if is_opaque_named(return_type, opaque) {
        return "Void*".to_string();
    }
    // Fallible returns: scalar types (bool, int, float) return the value
    // directly and signal errors via the `last_error_code` / `last_error_context`
    // mechanism. Named(Named) struct returns use a struct pointer (null signals
    // the error). Everything else uses JSON-string ABI so we can distinguish
    // error from valid return.
    if error_type.is_some() && is_scalar(return_type) {
        return c_type_of(return_type, opaque, ffi_structs).into_owned();
    }
    if error_type.is_some()
        && !is_opaque_named(return_type, opaque)
        && !matches!(return_type, TypeRef::Named(n) if ffi_structs.contains(n))
    {
        return "LibC::Char*".to_string();
    }
    c_type_of(return_type, opaque, ffi_structs).into_owned()
}

/// A streaming method: an owner type with a method that yields a stream of items,
/// consumed via the FFI iterator `_start`/`_next`/`_free` ABI.
struct StreamSpec {
    owner: String,
    method: String,
    item: String,
    params: Vec<(String, TypeRef)>,
}

/// An async method adapter: a method on an opaque handle type that calls the
/// underlying Rust async method over the C FFI with JSON-string marshalling.
struct AsyncMethodSpec {
    owner: String,
    method: String,
    return_type: TypeRef,
    error_type: Option<String>,
    params: Vec<(String, TypeRef)>,
}

/// Map an adapter parameter type string (a Rust type name) to an IR `TypeRef`.
fn parse_adapter_type(s: &str) -> TypeRef {
    let t = s.trim().trim_start_matches('&').trim_start_matches("'static").trim();
    match t {
        "String" | "str" | "&str" => TypeRef::String,
        "bool" => TypeRef::Primitive(PrimitiveType::Bool),
        "u8" => TypeRef::Primitive(PrimitiveType::U8),
        "u16" => TypeRef::Primitive(PrimitiveType::U16),
        "u32" => TypeRef::Primitive(PrimitiveType::U32),
        "u64" => TypeRef::Primitive(PrimitiveType::U64),
        "i8" => TypeRef::Primitive(PrimitiveType::I8),
        "i16" => TypeRef::Primitive(PrimitiveType::I16),
        "i32" => TypeRef::Primitive(PrimitiveType::I32),
        "i64" => TypeRef::Primitive(PrimitiveType::I64),
        "usize" => TypeRef::Primitive(PrimitiveType::Usize),
        "isize" => TypeRef::Primitive(PrimitiveType::Isize),
        "f32" => TypeRef::Primitive(PrimitiveType::F32),
        "f64" => TypeRef::Primitive(PrimitiveType::F64),
        other => TypeRef::Named(other.to_string()),
    }
}

/// Collect Crystal-enabled async method adapters (owner + return type resolved).
fn async_method_specs(config: &ResolvedCrateConfig) -> Vec<AsyncMethodSpec> {
    config
        .adapters
        .iter()
        .filter(|a| a.owner_type.is_some())
        .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::AsyncMethod))
        .filter(|a| !a.skip_languages.iter().any(|l| l == "crystal"))
        .filter_map(|a| {
            let params = a
                .params
                .iter()
                .map(|p| {
                    let ty = parse_adapter_type(&p.ty);
                    let ty = if p.optional {
                        TypeRef::Optional(Box::new(ty))
                    } else {
                        ty
                    };
                    (p.name.clone(), ty)
                })
                .collect();
            let return_type = a.returns.as_deref().map(parse_adapter_type)?;
            Some(AsyncMethodSpec {
                owner: a.owner_type.clone()?,
                method: a.name.clone(),
                return_type,
                error_type: a.error_type.clone(),
                params,
            })
        })
        .collect()
}

/// Collect Crystal-enabled streaming adapters (owner + item types resolved).
fn streaming_specs(config: &ResolvedCrateConfig) -> Vec<StreamSpec> {
    config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
        .filter(|a| !a.skip_languages.iter().any(|l| l == "crystal"))
        .filter_map(|a| {
            let params = a
                .params
                .iter()
                .map(|p| {
                    let ty = parse_adapter_type(&p.ty);
                    let ty = if p.optional {
                        TypeRef::Optional(Box::new(ty))
                    } else {
                        ty
                    };
                    (p.name.clone(), ty)
                })
                .collect();
            Some(StreamSpec {
                owner: a.owner_type.clone()?,
                method: a.name.clone(),
                item: a.item_type.clone()?,
                params,
            })
        })
        .collect()
}

/// Collect type names of non-opaque struct types in the API surface.
/// These have C-level `from_json`/`to_json`/`free` helpers in the FFI and must be
/// passed as struct pointers, not JSON strings.
/// Collect Named type names used in function signatures (params + returns).
/// These correspond to C FFI struct types that need `struct` declarations and
/// `from_json`/`to_json`/`free` helpers. Opaque handle types are excluded.
fn ffi_struct_names(api: &ApiSurface) -> HashSet<String> {
    let opaque = opaque_names(api);
    let mut names = HashSet::new();
    for func in &api.functions {
        for p in &func.params {
            collect_named_types(&p.ty, &mut names);
        }
        collect_named_types(&func.return_type, &mut names);
    }
    for ty in &api.types {
        for m in &ty.methods {
            for p in &m.params {
                collect_named_types(&p.ty, &mut names);
            }
            collect_named_types(&m.return_type, &mut names);
        }
    }
    names.retain(|n| !opaque.contains(n));
    names
}

fn collect_named_types(ty: &TypeRef, names: &mut HashSet<String>) {
    match ty {
        TypeRef::Named(n) => {
            names.insert(n.clone());
        }
        TypeRef::Optional(inner) => collect_named_types(inner, names),
        TypeRef::Vec(inner) => collect_named_types(inner, names),
        TypeRef::Map(k, v) => {
            collect_named_types(k, names);
            collect_named_types(v, names);
        }
        _ => {}
    }
}

/// Whether a Named type reference is a C FFI struct type (non-opaque, in the API).
fn is_ffi_struct(name: &str, ffi_structs: &HashSet<String>) -> bool {
    ffi_structs.contains(name)
}

/// The set of opaque (handle-based) type names in an API surface.
fn opaque_names(api: &ApiSurface) -> HashSet<String> {
    api.types
        .iter()
        .filter(|t| t.is_opaque && !t.binding_excluded)
        .map(|t| t.name.clone())
        .collect()
}

/// Whether a type is an opaque handle (crosses the C ABI as an opaque pointer).
fn is_opaque_named(ty: &TypeRef, opaque: &HashSet<String>) -> bool {
    matches!(ty, TypeRef::Named(n) if opaque.contains(n))
}

/// The Crystal `lib` (C-ABI) type for a value, treating opaque handles as `Void*`
/// (a raw pointer) and FFI struct types as struct pointers rather than the default
/// JSON-string marshalling.
fn c_type_of(ty: &TypeRef, opaque: &HashSet<String>, ffi_structs: &HashSet<String>) -> std::borrow::Cow<'static, str> {
    if is_opaque_named(ty, opaque) {
        return std::borrow::Cow::Borrowed("Void*");
    }
    if let TypeRef::Named(name) = ty {
        if is_ffi_struct(name, ffi_structs) {
            return std::borrow::Cow::Owned(format!("{}*", crystal_type_name(name)));
        }
    }
    // Optional(Named(...)) if the inner type is a struct → use nullable struct pointer.
    if let TypeRef::Optional(inner) = ty {
        if let TypeRef::Named(name) = inner.as_ref() {
            if is_ffi_struct(name, ffi_structs) {
                return std::borrow::Cow::Owned(format!("{}*", crystal_type_name(name)));
            }
        }
    }
    crystal_c_type(ty)
}

/// Strip a trailing `?` from a Crystal type (a nilable `T?` → `T`). Used when the
/// concrete type is needed for `T.new(pull)` deserialization.
fn strip_nil(ty: &str) -> &str {
    ty.strip_suffix('?').unwrap_or(ty)
}

/// Ensure a Crystal type is nilable (`T` → `T?`), used for `nil`-initialised locals.
fn nilable(ty: &str) -> String {
    if ty.ends_with('?') {
        ty.to_string()
    } else {
        format!("{ty}?")
    }
}

impl Backend for CrystalBackend {
    fn name(&self) -> &str {
        "crystal"
    }

    fn language(&self) -> Language {
        Language::Crystal
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: true,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            supports_callbacks: false,
            supports_streaming: true,
            supports_service_api: true,
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        // Trait bridges: visitor-style bridges (a foreign object implementing a Rust
        // trait via the C-ABI visitor-callback vtable) are supported when they carry a
        // `context_type` + a unit-variant-only `result_type` enum. Plugin-style bridges
        // (registry via `register_fn`) and data-carrying result enums (tagged unions,
        // which Crystal does not emit yet) are rejected loudly rather than silently
        // mis-generated. See `super::trait_bridge`.
        let unsupported_bridges: Vec<&str> = config
            .trait_bridges
            .iter()
            .filter(|b| {
                !super::trait_bridge::is_supported_visitor_bridge(api, b)
                    && !super::trait_bridge::is_supported_plugin_bridge(api, b)
            })
            .map(|b| b.trait_name.as_str())
            .collect();
        if !unsupported_bridges.is_empty() {
            anyhow::bail!(
                "the Crystal backend supports visitor-style trait bridges (with `context_type` \
                 and a unit-variant `result_type`) but not plugin-style/registry bridges or \
                 data-carrying result enums yet (unsupported: {}); remove `crystal` from this \
                 generation run, opt these out of the trait bridge config, or extend the Crystal \
                 backend",
                unsupported_bridges.join(", ")
            );
        }

        // Crystal is a single compiled surface over the C ABI (like Go/Zig): collapse
        // same-named cfg-variant functions so we do not emit duplicate `fun`/method defs.
        let mut deduped = api.with_deduped_functions();

        // Apply per-language exclude lists from `[crates.crystal]`.
        if let Some(c) = &config.crystal {
            if !c.exclude_functions.is_empty() {
                deduped.functions.retain(|f| !c.exclude_functions.contains(&f.name));
            }
            if !c.exclude_types.is_empty() {
                deduped.types.retain(|t| !c.exclude_types.contains(&t.name));
                deduped.enums.retain(|e| !c.exclude_types.contains(&e.name));
                deduped.errors.retain(|e| !c.exclude_types.contains(&e.name));
                // Also filter fields in remaining types that reference excluded types,
                // so we don't emit `getter x : ExcludedType?` with an undefined constant.
                for typ in &mut deduped.types {
                    typ.fields.retain(|f| !type_ref_uses_excluded(&f.ty, &c.exclude_types));
                }
            }
        }
        let _api = &deduped;
        let api = &deduped;

        let ffi_prefix = config.ffi_prefix();
        let ffi_lib_name = config.ffi_lib_name();
        let ffi_header = config.ffi_header_name();

        let ffi_exclude: HashSet<String> = config
            .ffi
            .as_ref()
            .map(|f| f.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();

        let crystal_exclude: HashSet<String> = config
            .crystal
            .as_ref()
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();

        let extra_exclude: HashSet<String> = ffi_exclude.union(&crystal_exclude).cloned().collect();

        let output_dir = {
            let mut d = resolve_output_dir(config.output_paths.get("crystal"), &config.name, "packages/crystal/");
            if !d.ends_with('/') {
                d.push('/');
            }
            d
        };

        let shard_name = Self::shard_name(&config.name);

        let opaque = opaque_names(api);
        let ffi_structs = ffi_struct_names(api);
        let streaming = streaming_specs(config);
        let async_methods = async_method_specs(config);
        let mut content = Self::gen_lib_block(
            api,
            &ffi_prefix,
            &ffi_lib_name,
            &ffi_header,
            &extra_exclude,
            &opaque,
            &streaming,
            &async_methods,
            &ffi_structs,
        );
        // Resolve module_name: prefer the config override, fall back to PascalCase crate name.
        let module_name = config
            .crystal
            .as_ref()
            .and_then(|c| c.module_name.clone())
            .unwrap_or_else(|| Self::module_name(&api.crate_name));
        let borrowed_handles: HashSet<String> = config
            .crystal
            .as_ref()
            .map(|c| c.borrowed_handles.iter().cloned().collect())
            .unwrap_or_default();
        content.push_str(&Self::gen_module(
            api,
            &ffi_prefix,
            &extra_exclude,
            &opaque,
            &streaming,
            &async_methods,
            &ffi_structs,
            &module_name,
            &borrowed_handles,
        ));

        // Append `require` statements for auxiliary bridge files before finalising content.
        let lib_name = Self::lib_name(&ffi_prefix);
        for bridge in &config.trait_bridges {
            let bridge_snake = crate::codegen::naming::public_host_identifier(
                Language::Crystal,
                PublicIdentifierKind::Function,
                &bridge.trait_name,
            );
            if super::trait_bridge::gen_visitor_file(api, bridge, &ffi_prefix, &lib_name, &module_name).is_some() {
                content.push_str(&format!("require \"./{shard_name}_{bridge_snake}_visitor\"\n"));
            }
            if super::trait_bridge::gen_plugin_file(api, bridge, &ffi_prefix, &lib_name, &module_name).is_some() {
                content.push_str(&format!("require \"./{shard_name}_{bridge_snake}_plugin\"\n"));
            }
        }

        let mut files = vec![GeneratedFile {
            path: PathBuf::from(format!("{output_dir}src/{shard_name}.cr")),
            content,
            generated_header: true,
        }];

        // Emit a minimal shard.yml so `shards build` works out of the box.
        files.push(GeneratedFile {
            path: PathBuf::from(format!("{output_dir}shard.yml")),
            content: render(
                "shard.yml.jinja",
                minijinja::context! {
                    shard_name => shard_name,
                    crate_name => api.crate_name,
                    version => api.version,
                },
            ),
            generated_header: false,
        });

        // Visitor-style trait bridges → a `visitor.cr` per bridge (reopens the lib).
        for bridge in &config.trait_bridges {
            let bridge_snake = crate::codegen::naming::public_host_identifier(
                Language::Crystal,
                PublicIdentifierKind::Function,
                &bridge.trait_name,
            );
            if let Some(visitor_src) =
                super::trait_bridge::gen_visitor_file(api, bridge, &ffi_prefix, &lib_name, &module_name)
            {
                files.push(GeneratedFile {
                    path: PathBuf::from(format!("{output_dir}src/{shard_name}_{bridge_snake}_visitor.cr")),
                    content: visitor_src,
                    generated_header: true,
                });
            }
            // Plugin-style (registry) trait bridges → a `plugin.cr` per bridge.
            if let Some(plugin_src) =
                super::trait_bridge::gen_plugin_file(api, bridge, &ffi_prefix, &lib_name, &module_name)
            {
                files.push(GeneratedFile {
                    path: PathBuf::from(format!("{output_dir}src/{shard_name}_{bridge_snake}_plugin.cr")),
                    content: plugin_src,
                    generated_header: true,
                });
            }
        }

        Ok(files)
    }

    fn generate_scaffold(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let output_dir = {
            let mut d = resolve_output_dir(config.output_paths.get("crystal"), &config.name, "packages/crystal/");
            if !d.ends_with('/') {
                d.push('/');
            }
            d
        };
        let shard_name = Self::shard_name(&config.name);
        let content = render(
            "shard.yml.jinja",
            minijinja::context! {
                shard_name => shard_name,
                crate_name => api.crate_name,
                version => api.version,
            },
        );
        Ok(vec![GeneratedFile {
            path: PathBuf::from(format!("{output_dir}shard.yml")),
            content,
            generated_header: false,
        }])
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "shards",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_param(name: &str, ty: TypeRef) -> crate::core::ir::ParamDef {
        crate::core::ir::ParamDef {
            name: name.into(),
            ty,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }
    }

    #[test]
    fn lib_and_module_names() {
        assert_eq!(CrystalBackend::lib_name("sample_core"), "LibSampleCore");
        assert_eq!(CrystalBackend::module_name("sample-core"), "SampleCore");
        assert_eq!(CrystalBackend::shard_name("sample-core"), "sample_core");
    }

    #[test]
    fn backend_identity() {
        let b = CrystalBackend;
        assert_eq!(b.name(), "crystal");
        assert_eq!(b.language(), Language::Crystal);
        assert!(b.capabilities().supports_streaming);
    }

    // ── F11: collect_named_types ──────────────────────────────────────────

    #[test]
    fn collect_named_types_adds_direct_named() {
        let mut names = HashSet::new();
        collect_named_types(&TypeRef::Named("Config".into()), &mut names);
        assert_eq!(names.len(), 1);
        assert!(names.contains("Config"));
    }

    #[test]
    fn collect_named_types_walks_optional() {
        let mut names = HashSet::new();
        collect_named_types(
            &TypeRef::Optional(Box::new(TypeRef::Named("Config".into()))),
            &mut names,
        );
        assert!(names.contains("Config"));
    }

    #[test]
    fn collect_named_types_walks_vec() {
        let mut names = HashSet::new();
        collect_named_types(
            &TypeRef::Vec(Box::new(TypeRef::Named("Item".into()))),
            &mut names,
        );
        assert!(names.contains("Item"));
    }

    #[test]
    fn collect_named_types_walks_map() {
        let mut names = HashSet::new();
        collect_named_types(
            &TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Named("Value".into())),
            ),
            &mut names,
        );
        assert!(names.contains("Value"));
    }

    #[test]
    fn collect_named_types_skips_primitives() {
        let mut names = HashSet::new();
        collect_named_types(&TypeRef::Primitive(PrimitiveType::I32), &mut names);
        assert!(names.is_empty());
    }

    #[test]
    fn collect_named_types_allows_map_key_as_type_ref() {
        let mut names = HashSet::new();
        collect_named_types(
            &TypeRef::Map(
                Box::new(TypeRef::Named("KeyType".into())),
                Box::new(TypeRef::String),
            ),
            &mut names,
        );
        assert!(names.contains("KeyType"), "Map keys should be recursed into");
    }

    // ── F11: ffi_struct_names ────────────────────────────────────────────

    #[test]
    fn ffi_struct_names_omits_opaque_handles() {
        let api = ApiSurface {
            crate_name: "demo".into(),
            version: "0.1.0".into(),
            types: vec![
                TypeDef {
                    name: "Config".into(),
                    has_serde: true,
                    ..TypeDef::default()
                },
                TypeDef {
                    name: "Handle".into(),
                    is_opaque: true,
                    ..TypeDef::default()
                },
            ],
            functions: vec![
                FunctionDef {
                    name: "build".into(),
                    params: vec![make_param("cfg", TypeRef::Named("Config".into()))],
                    return_type: TypeRef::Named("Config".into()),
                    ..FunctionDef::default()
                },
                FunctionDef {
                    name: "use_handle".into(),
                    params: vec![make_param("h", TypeRef::Named("Handle".into()))],
                    return_type: TypeRef::Unit,
                    ..FunctionDef::default()
                },
            ],
            ..ApiSurface::default()
        };
        let names = ffi_struct_names(&api);
        assert_eq!(names.len(), 1, "expected only Config, got {names:?}");
        assert!(names.contains("Config"), "Config should be in ffi_struct_names");
        assert!(!names.contains("Handle"), "opaque Handle should NOT be in ffi_struct_names");
    }

    #[test]
    fn ffi_struct_names_collects_from_type_methods() {
        let api = ApiSurface {
            crate_name: "demo".into(),
            version: "0.1.0".into(),
            types: vec![TypeDef {
                name: "Widget".into(),
                has_serde: true,
                methods: vec![crate::core::ir::MethodDef {
                    name: "update".into(),
                    params: vec![make_param("cfg", TypeRef::Named("Config".into()))],
                    return_type: TypeRef::Named("Config".into()),
                    ..crate::core::ir::MethodDef::default()
                }],
                ..TypeDef::default()
            }],
            functions: vec![],
            ..ApiSurface::default()
        };
        let names = ffi_struct_names(&api);
        assert!(names.contains("Config"));
    }

    #[test]
    fn ffi_struct_names_returns_empty_when_no_named_types() {
        let api = ApiSurface {
            crate_name: "demo".into(),
            version: "0.1.0".into(),
            types: vec![],
            functions: vec![FunctionDef {
                name: "noop".into(),
                params: vec![],
                return_type: TypeRef::Unit,
                ..FunctionDef::default()
            }],
            ..ApiSurface::default()
        };
        let names = ffi_struct_names(&api);
        assert!(names.is_empty(), "no Named types → empty set, got {names:?}");
    }

    #[test]
    fn opaque_names_collects_opaque_types() {
        let api = ApiSurface {
            types: vec![
                TypeDef {
                    name: "Handle".into(),
                    is_opaque: true,
                    ..TypeDef::default()
                },
                TypeDef {
                    name: "Visible".into(),
                    is_opaque: false,
                    ..TypeDef::default()
                },
            ],
            ..ApiSurface::default()
        };
        let opaque = opaque_names(&api);
        assert!(opaque.contains("Handle"));
        assert!(!opaque.contains("Visible"));
    }

    #[test]
    fn is_opaque_named_works() {
        let opaque: HashSet<String> = ["Handle"].into_iter().map(|s| s.to_string()).collect();
        assert!(is_opaque_named(&TypeRef::Named("Handle".into()), &opaque));
        assert!(!is_opaque_named(&TypeRef::Named("Config".into()), &opaque));
        assert!(!is_opaque_named(&TypeRef::String, &opaque));
    }

    #[test]
    fn is_ffi_struct_checks_membership() {
        let ffi: HashSet<String> = ["Config"].into_iter().map(|s| s.to_string()).collect();
        assert!(is_ffi_struct("Config", &ffi));
        assert!(!is_ffi_struct("Handle", &ffi));
    }
}
