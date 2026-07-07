//! Crystal binding generation.
//!
//! Emits a single Crystal source file per crate containing:
//! 1. A `lib` block binding the exported C FFI symbols (`fun` declarations).
//! 2. A high-level module with Ruby-style snake_case wrapper methods that
//!    marshal arguments/return values across the JSON-string C ABI.
//!
//! plus a `shard.yml` package manifest.

use std::collections::HashSet;
use std::path::PathBuf;

use heck::ToPascalCase;

use crate::codegen::naming::{
    PublicIdentifierKind, abi_symbol, public_host_identifier, wire_field_name, wire_variant_value,
};
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{Language, ResolvedCrateConfig, resolve_output_dir};
use crate::core::ir::{ApiSurface, EnumDef, ErrorDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};

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
    fn lib_params(func: &FunctionDef, opaque: &HashSet<String>) -> String {
        func.params
            .iter()
            .map(|p| {
                let name = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, &p.name);
                let cty = c_type_of(&p.ty, opaque);
                format!("{name} : {cty}")
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Render the C return type for a lib declaration.
    fn lib_return(func: &FunctionDef, opaque: &HashSet<String>) -> String {
        lib_c_return(&func.return_type, func.error_type.as_deref(), opaque)
    }

    /// Generate the `lib` block binding all exported C symbols.
    fn gen_lib_block(
        api: &ApiSurface,
        ffi_prefix: &str,
        ffi_lib_name: &str,
        ffi_header: &str,
        ffi_exclude: &HashSet<String>,
        opaque: &HashSet<String>,
        streaming: &[StreamSpec],
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

        for func in &api.functions {
            if Self::is_excluded(func, ffi_exclude) {
                continue;
            }
            let crystal_name = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &func.name);
            let c_symbol = abi_symbol(ffi_prefix, &func.name);
            out.push_str(&render(
                "lib_fun.jinja",
                minijinja::context! {
                    doc => func.doc.lines().next().unwrap_or_default().trim(),
                    crystal_name => crystal_name,
                    c_symbol => c_symbol,
                    params => Self::lib_params(func, opaque),
                    return_type => Self::lib_return(func, opaque),
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
                    params.push(format!("{pn} : {}", c_type_of(&p.ty, opaque)));
                }
                let ret = lib_c_return(&m.return_type, m.error_type.as_deref(), opaque);
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
            let mut start_params = vec!["handle : Void*".to_string()];
            for (pname, pty) in &spec.params {
                let pn = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, pname);
                start_params.push(format!("{pn} : {}", c_type_of(pty, opaque)));
            }
            out.push_str(&format!(
                "  fun {base}_start = {c_base}_start({}) : Void*\n",
                start_params.join(", ")
            ));
            out.push_str(&format!("  fun {base}_next = {c_base}_next(handle : Void*) : Void*\n"));
            out.push_str(&format!("  fun {base}_free = {c_base}_free(handle : Void*) : Void\n"));
            if item_funcs_done.insert(item_snake.clone()) {
                let c_item = abi_symbol(ffi_prefix, &spec.item);
                out.push_str(&format!(
                    "  fun {item_snake}_to_json = {c_item}_to_json(chunk : Void*) : LibC::Char*\n"
                ));
                out.push_str(&format!(
                    "  fun {item_snake}_free = {c_item}_free(chunk : Void*) : Void\n"
                ));
            }
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
    ) -> String {
        let module_name = Self::module_name(&api.crate_name);
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
        out.push_str(&Self::gen_types(api, ffi_prefix, &lib_name, opaque, streaming));

        for func in &api.functions {
            if Self::is_excluded(func, ffi_exclude) {
                continue;
            }
            out.push_str(&Self::gen_wrapper_method(func, &lib_name, opaque));
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
    ) -> String {
        let mut out = String::new();
        for ty in &api.types {
            if ty.binding_excluded || ty.is_trait {
                continue;
            }
            if ty.is_opaque {
                out.push_str(&Self::gen_opaque(ty, ffi_prefix, lib_name, opaque, streaming));
                continue;
            }
            out.push_str(&Self::gen_struct(ty));
        }
        for en in &api.enums {
            if en.binding_excluded {
                continue;
            }
            out.push_str(&Self::gen_enum(en));
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
    fn gen_struct(ty: &TypeDef) -> String {
        let name = crystal_type_name(&ty.name);
        let mut out = String::new();
        if let Some(summary) = ty.doc.lines().next().map(str::trim).filter(|s| !s.is_empty()) {
            out.push_str(&format!("\n  # {summary}\n"));
        } else {
            out.push('\n');
        }
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
            if let Some(doc) = field.doc.lines().next().map(str::trim).filter(|s| !s.is_empty()) {
                out.push_str(&format!("    # {doc}\n"));
            }
            if wire != field_name {
                out.push_str(&format!("    @[JSON::Field(key: {wire:?})]\n"));
            }
            out.push_str(&format!("    getter {field_name} : {field_ty}\n"));
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
    ) -> String {
        let name = crystal_type_name(&ty.name);
        let type_snake = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &ty.name);
        let free = format!("{type_snake}_free");
        let _ = ffi_prefix;
        let mut out = Self::doc_or_blank(&ty.doc);
        out.push_str(&format!("  class {name}\n"));
        out.push_str("    # Wraps the owned FFI handle; do not construct directly.\n");
        out.push_str("    def initialize(@handle : Void*)\n    end\n");
        out.push_str("    # Raw handle for passing back across the C ABI.\n");
        out.push_str("    def to_unsafe : Void*\n      @handle\n    end\n");
        out.push_str("    def finalize\n");
        out.push_str(&format!("      {lib_name}.{free}(@handle) unless @handle.null?\n"));
        out.push_str("    end\n");

        for m in &ty.methods {
            if m.binding_excluded {
                continue;
            }
            out.push_str(&Self::gen_opaque_method(ty, m, &type_snake, lib_name, opaque));
        }

        // Streaming methods owned by this type → fiber-fed channels.
        for spec in streaming.iter().filter(|s| s.owner == ty.name) {
            out.push_str(&Self::gen_stream_method(spec, &type_snake, lib_name));
        }

        out.push_str("  end\n");
        out
    }

    /// Emit a streaming method returning a `Channel(Item)` fed by a fiber that
    /// drives the FFI iterator (`_start`/`_next`/`_free`) — Crystal's idiomatic
    /// concurrency: `spawn` + `Channel`.
    fn gen_stream_method(spec: &StreamSpec, type_snake: &str, lib_name: &str) -> String {
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
        let sig = if sig_params.is_empty() {
            format!("def {method} : Channel({item})")
        } else {
            format!("def {method}({sig_params}) : Channel({item})")
        };
        let mut start_args = vec!["@handle".to_string()];
        for (n, ty) in &spec.params {
            let pn = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, n);
            if is_scalar(ty) {
                start_args.push(pn);
            } else {
                start_args.push(format!("{pn}.to_json"));
            }
        }
        let mut b = String::new();
        b.push_str(&format!("\n    # Stream of `{item}` items over a fiber-fed channel.\n"));
        b.push_str(&format!("    {sig}\n"));
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
        b.push_str(&format!(
            "            __jp = {lib_name}.{item_snake}_to_json(__chunk)\n"
        ));
        b.push_str("            if __jp.null?\n");
        b.push_str(&format!("              {lib_name}.{item_snake}_free(__chunk)\n"));
        b.push_str("              break\n");
        b.push_str("            end\n");
        b.push_str("            __json = String.new(__jp)\n");
        b.push_str(&format!("            {lib_name}.free_string(__jp)\n"));
        b.push_str(&format!("            {lib_name}.{item_snake}_free(__chunk)\n"));
        b.push_str(&format!("            __ch.send({item}.from_json(__json))\n"));
        b.push_str("          end\n");
        b.push_str("        ensure\n");
        b.push_str(&format!("          {lib_name}.{base}_free(__handle)\n"));
        b.push_str("          __ch.close\n");
        b.push_str("        end\n");
        b.push_str("      end\n");
        b.push_str("      __ch\n");
        b.push_str("    end\n");
        b
    }

    /// Emit one instance or static method on an opaque handle wrapper class.
    fn gen_opaque_method(
        ty: &TypeDef,
        m: &crate::core::ir::MethodDef,
        type_snake: &str,
        lib_name: &str,
        opaque: &HashSet<String>,
    ) -> String {
        let method = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &m.name);
        let method_snake = &method;
        let sig_params = m
            .params
            .iter()
            .map(|p| {
                let name = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, &p.name);
                format!("{name} : {}", crystal_type(&p.ty))
            })
            .collect::<Vec<_>>()
            .join(", ");
        let mut args: Vec<String> = if m.is_static {
            Vec::new()
        } else {
            vec!["@handle".to_string()]
        };
        for p in &m.params {
            args.push(marshal_call_arg(p, opaque));
        }
        let call = format!("{lib_name}.{type_snake}_{method_snake}({})", args.join(", "));
        let ret_annot = if matches!(m.return_type, TypeRef::Unit) && m.error_type.is_none() {
            "Nil".to_string()
        } else {
            crystal_type(&m.return_type).into_owned()
        };
        let label = format!("{type_snake}_{method_snake}");
        let body = Self::gen_call_body(&call, &m.return_type, m.error_type.as_deref(), lib_name, opaque, &label);
        let decl = if m.is_static {
            format!("self.{method}")
        } else {
            method.clone()
        };
        let doc = m.doc.lines().next().unwrap_or_default().trim();
        let _ = ty;
        format!("\n    # {doc}\n    def {decl}({sig_params}) : {ret_annot}\n{body}    end\n")
    }

    /// Emit a Crystal type for an enum.
    ///
    /// - all-unit → a plain Crystal `enum`.
    /// - externally-tagged with unit + single-payload (newtype) variants →
    ///   an abstract-class hierarchy that round-trips serde's externally-tagged JSON
    ///   (`"Unit"` for unit variants, `{"Variant": payload}` for newtype variants).
    /// - anything else (struct/multi-tuple variants, internally/adjacently-tagged,
    ///   untagged) → skipped with an explanatory note (pending).
    fn gen_enum(en: &EnumDef) -> String {
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
            // Internally-tagged: only unit + struct variants are valid in serde.
            if variants.iter().any(|v| v.is_tuple) {
                return format!(
                    "\n  # NOTE: internally-tagged enum `{name}` with tuple/newtype variants \
                     is not representable; skipped.\n"
                );
            }
            return Self::gen_internally_tagged(en, &name, &variants, tag, is_unit);
        }

        Self::gen_tagged_union(en, &name, &variants, is_unit)
    }

    /// Emit an abstract-class hierarchy for an internally-tagged enum
    /// (`#[serde(tag = "...")]` → `{"<tag>":"Variant", ...fields}`), using Crystal's
    /// native `use_json_discriminator` for dispatch. Each subclass re-emits the tag
    /// field (with a default) so `to_json` round-trips.
    fn gen_internally_tagged(
        en: &EnumDef,
        name: &str,
        variants: &[&crate::core::ir::EnumVariant],
        tag: &str,
        is_unit: impl Fn(&crate::core::ir::EnumVariant) -> bool,
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
                for f in &v.fields {
                    if f.binding_excluded {
                        continue;
                    }
                    let getter = public_host_identifier(Language::Crystal, PublicIdentifierKind::Field, &f.name);
                    let key = wire_field_name(&f.name, f.serde_rename.as_deref(), en.serde_rename_all.as_deref());
                    let mut ty = crystal_type(&f.ty).into_owned();
                    if f.optional && !ty.ends_with('?') {
                        ty.push('?');
                    }
                    if key != getter {
                        out.push_str(&format!("    @[JSON::Field(key: {key:?})]\n"));
                    }
                    out.push_str(&format!("    getter {getter} : {ty}\n"));
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
        out.push_str("    abstract def to_json(json : ::JSON::Builder) : Nil\n");
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
            out.push_str("    def to_json(json : ::JSON::Builder) : Nil\n");
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
            out.push_str("    def to_json(json : ::JSON::Builder) : Nil\n");
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
        out.push_str("    def to_json(json : ::JSON::Builder) : Nil\n");
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
        out.push_str("    abstract def to_json(json : ::JSON::Builder) : Nil\n");
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
            out.push_str("    def to_json(json : ::JSON::Builder) : Nil\n");
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
        out.push_str("    def to_json(json : ::JSON::Builder) : Nil\n");
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
        match doc.lines().next().map(str::trim).filter(|s| !s.is_empty()) {
            Some(summary) => format!("\n  # {summary}\n"),
            None => "\n".to_string(),
        }
    }

    /// Emit a Crystal exception subclass for a Rust error type.
    fn gen_error(err: &ErrorDef) -> String {
        let name = crystal_type_name(&err.name);
        let mut out = String::new();
        if let Some(summary) = err.doc.lines().next().map(str::trim).filter(|s| !s.is_empty()) {
            out.push_str(&format!("\n  # {summary}\n"));
        } else {
            out.push('\n');
        }
        out.push_str(&format!("  class {name} < Exception\n  end\n"));
        out
    }

    /// Generate a single snake_case wrapper method delegating to the lib fun.
    fn gen_wrapper_method(func: &FunctionDef, lib_name: &str, opaque: &HashSet<String>) -> String {
        let method = public_host_identifier(Language::Crystal, PublicIdentifierKind::Function, &func.name);
        let ret_ty = crystal_type(&func.return_type);

        // Signature: high-level Crystal parameter types.
        let sig_params = func
            .params
            .iter()
            .map(|p| {
                let name = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, &p.name);
                format!("{name} : {}", crystal_type(&p.ty))
            })
            .collect::<Vec<_>>()
            .join(", ");

        // Call arguments: scalars pass through, opaque handles pass their pointer,
        // everything else is JSON-encoded.
        let call_args = func
            .params
            .iter()
            .map(|p| marshal_call_arg(p, opaque))
            .collect::<Vec<_>>()
            .join(", ");

        let ret_annot = if matches!(func.return_type, TypeRef::Unit) && func.error_type.is_none() {
            "Nil".to_string()
        } else {
            ret_ty.into_owned()
        };

        let body = Self::gen_wrapper_body(func, lib_name, &method, &call_args, opaque);

        format!(
            "\n  # {doc}\n  def self.{method}({sig_params}) : {ret_annot}\n{body}  end\n",
            doc = func.doc.lines().next().unwrap_or_default().trim(),
        )
    }

    fn gen_wrapper_body(
        func: &FunctionDef,
        lib_name: &str,
        method: &str,
        call_args: &str,
        opaque: &HashSet<String>,
    ) -> String {
        let call = format!("{lib_name}.{method}({call_args})");
        Self::gen_call_body(
            &call,
            &func.return_type,
            func.error_type.as_deref(),
            lib_name,
            opaque,
            method,
        )
    }

    /// Emit the marshalling + return-decoding body for a call expression, shared by
    /// free-function wrappers and opaque instance/static methods.
    fn gen_call_body(
        call: &str,
        return_type: &TypeRef,
        error_type: Option<&str>,
        lib_name: &str,
        opaque: &HashSet<String>,
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
            TypeRef::Json => b.push_str("    JSON.parse(__json)\n"),
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::String | TypeRef::Path | TypeRef::Char => b.push_str("    __json\n"),
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

/// Scalar types pass across the C ABI by value (no JSON marshalling).
fn is_scalar(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Primitive(_) | TypeRef::Unit)
}

/// The Crystal expression passing a parameter across the C ABI: scalars pass
/// through, opaque handles pass their pointer, everything else is JSON-encoded.
fn marshal_call_arg(p: &crate::core::ir::ParamDef, opaque: &HashSet<String>) -> String {
    let name = public_host_identifier(Language::Crystal, PublicIdentifierKind::Parameter, &p.name);
    if is_scalar(&p.ty) {
        name
    } else if is_opaque_named(&p.ty, opaque) {
        format!("{name}.to_unsafe")
    } else {
        format!("{name}.to_json")
    }
}

/// The Crystal `lib` C return type for a return/error pair.
fn lib_c_return(return_type: &TypeRef, error_type: Option<&str>, opaque: &HashSet<String>) -> String {
    if is_opaque_named(return_type, opaque) {
        return "Void*".to_string();
    }
    if error_type.is_some() {
        return "LibC::Char*".to_string();
    }
    c_type_of(return_type, opaque).into_owned()
}

/// A streaming method: an owner type with a method that yields a stream of items,
/// consumed via the FFI iterator `_start`/`_next`/`_free` ABI.
struct StreamSpec {
    owner: String,
    method: String,
    item: String,
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
/// (a raw pointer) rather than the default JSON-string marshalling.
fn c_type_of(ty: &TypeRef, opaque: &HashSet<String>) -> std::borrow::Cow<'static, str> {
    if is_opaque_named(ty, opaque) {
        return std::borrow::Cow::Borrowed("Void*");
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
            supports_service_api: false,
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
            .filter(|b| !super::trait_bridge::is_supported_visitor_bridge(api, b))
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
        let deduped = api.with_deduped_functions();
        let api = &deduped;

        let ffi_prefix = config.ffi_prefix();
        let ffi_lib_name = config.ffi_lib_name();
        let ffi_header = config.ffi_header_name();

        let ffi_exclude: HashSet<String> = config
            .ffi
            .as_ref()
            .map(|f| f.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();

        let output_dir = {
            let mut d = resolve_output_dir(config.output_paths.get("crystal"), &config.name, "packages/crystal/");
            if !d.ends_with('/') {
                d.push('/');
            }
            d
        };

        let shard_name = Self::shard_name(&config.name);

        let opaque = opaque_names(api);
        let streaming = streaming_specs(config);
        let mut content = Self::gen_lib_block(
            api,
            &ffi_prefix,
            &ffi_lib_name,
            &ffi_header,
            &ffi_exclude,
            &opaque,
            &streaming,
        );
        content.push_str(&Self::gen_module(api, &ffi_prefix, &ffi_exclude, &opaque, &streaming));

        let mut files = vec![GeneratedFile {
            path: PathBuf::from(format!("{output_dir}src/{shard_name}.cr")),
            content,
            generated_header: true,
        }];

        // Visitor-style trait bridges → a `visitor.cr` per bridge (reopens the lib).
        let lib_name = Self::lib_name(&ffi_prefix);
        let module_name = Self::module_name(&api.crate_name);
        for bridge in &config.trait_bridges {
            if let Some(visitor_src) =
                super::trait_bridge::gen_visitor_file(api, bridge, &ffi_prefix, &lib_name, &module_name)
            {
                let bridge_snake = crate::codegen::naming::public_host_identifier(
                    Language::Crystal,
                    PublicIdentifierKind::Function,
                    &bridge.trait_name,
                );
                files.push(GeneratedFile {
                    path: PathBuf::from(format!("{output_dir}src/{shard_name}_{bridge_snake}_visitor.cr")),
                    content: visitor_src,
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
}
