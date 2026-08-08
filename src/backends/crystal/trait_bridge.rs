//! Crystal visitor-style trait bridge generation.
//!
//! Lets a Crystal object implement a Rust trait across the C ABI. This is the
//! host-side consumer of the visitor-callback vtable that the FFI backend
//! (`crate::backends::ffi::gen_visitor`) exports. Crystal is an excellent fit:
//! it supports real C function pointers plus `Box` for `user_data`, so it maps
//! directly onto the vtable — no handle-table workaround (unlike Go's cgo path).
//!
//! # ABI parity
//!
//! The generated layout mirrors the FFI `#[repr(C)]` structs *by construction*:
//!
//! - Context struct fields follow the context type's field order, with the same
//!   scalar C-type mapping the FFI uses (`String → char*`, `bool → i32`,
//!   integers 1:1, enum → i32 discriminant).
//! - The callbacks struct is `{ user_data, ...one fn ptr per trait method }`.
//! - Each callback is
//!   `fn(ctx: *const Context, user_data: *mut void, ...params.., out_custom: **char, out_len: *usize) -> i32`.
//! - Result codes are the result enum's unit-variant index, sourced from the
//!   shared [`visitor_result_metadata`] (identical to the FFI/Go side).
//!
//! Struct *names* do not affect the C ABI (Crystal matches by layout); only the
//! `fun` symbol names must match, and those reuse the FFI's `{prefix}_visitor_*`
//! formulas. End-to-end link parity should still be validated via the FFI e2e
//! harness.
//!
//! Scope: unit-variant results and `String`/scalar/enum context + `String`/
//! scalar callback parameters (the common visitor shape). Methods using
//! unsupported parameter shapes (e.g. table cell slices) are skipped with a note,
//! matching the FFI backend's own behaviour.

use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier};
use crate::codegen::visitor_result::visitor_result_metadata;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, MethodDef, PrimitiveType, TypeDef, TypeRef};

use super::type_map::{crystal_type, crystal_type_name};

/// One resolved callback parameter (beyond the shared ctx/user_data/out prefix).
struct CbParam {
    /// snake_case Crystal identifier.
    name: String,
    /// Crystal `lib` (C-ABI) type for the callback function-pointer signature.
    c_type: &'static str,
    /// High-level Crystal type exposed to the visitor method.
    hi_type: String,
    /// Expression converting the raw C arg (named `<name>`) to the high-level value.
    decode: String,
    /// When `true`, this param is a length twin (e.g. `cell_count` for a
    /// `*const *const c_char` string array) — present in the C signature but
    /// NOT forwarded to the high-level visitor method.
    is_count: bool,
}

/// One resolved context field.
struct CtxField {
    name: String,
    c_type: &'static str,
    hi_type: String,
    /// Expression decoding `raw.<name>` (a `lib` struct field) to the high-level value.
    decode: String,
}

/// One visitor callback (trait method) that can be bridged.
struct Callback {
    /// snake_case method name.
    method: String,
    doc: String,
    params: Vec<CbParam>,
}

/// Map a context field's `TypeRef` to (lib C type, high-level type, decode-expr),
/// replicating `backends::ffi::gen_visitor::context::context_c_type`.
/// Return the first non-excluded variant name for an enum type, if any.
fn enums_first_variant_name<'a>(api: &'a ApiSurface, name: &str) -> Option<&'a str> {
    api.enums
        .iter()
        .find(|e| e.name == name)
        .and_then(|e| e.variants.iter().find(|v| !v.binding_excluded))
        .map(|v| v.name.as_str())
}

fn context_field(field_ty: &TypeRef, raw_field: &str, api: &ApiSurface) -> Option<(&'static str, String, String)> {
    Some(match field_ty {
        TypeRef::String => ("LibC::Char*", "String?".into(), format!("raw.{raw_field}.null? ? nil : String.new(raw.{raw_field})")),
        TypeRef::Primitive(PrimitiveType::Bool) => ("Int32", "Bool".into(), format!("raw.{raw_field} != 0")),
        TypeRef::Primitive(p) => {
            let (c, hi) = crystal_scalar(p);
            (c, hi.into(), format!("raw.{raw_field}"))
        }
        TypeRef::Named(name) if api.enums.iter().any(|e| e.name == *name) => {
            let en = crystal_type_name(name);
            // Use safe `from_value?` to avoid exceptions when the Rust side
            // sends an enum discriminant that falls outside Crystal's variant
            // range (e.g. due to C ABI nesting or version skew).
            let first = enums_first_variant_name(api, name).unwrap_or("Text");
            ("Int32", format!("{en}?"), format!("{en}.from_value?(raw.{raw_field}) || {en}::{first}"))
        }
        _ => return None,
    })
}

/// Crystal (lib C type, high-level type) for a primitive, matching the FFI mapping.
fn crystal_scalar(p: &PrimitiveType) -> (&'static str, &'static str) {
    match p {
        PrimitiveType::Bool => ("Int32", "Bool"),
        PrimitiveType::U8 => ("UInt8", "UInt8"),
        PrimitiveType::U16 => ("UInt16", "UInt16"),
        PrimitiveType::U32 => ("UInt32", "UInt32"),
        PrimitiveType::U64 => ("UInt64", "UInt64"),
        PrimitiveType::I8 => ("Int8", "Int8"),
        PrimitiveType::I16 => ("Int16", "Int16"),
        PrimitiveType::I32 => ("Int32", "Int32"),
        PrimitiveType::I64 => ("Int64", "Int64"),
        PrimitiveType::Usize => ("LibC::SizeT", "LibC::SizeT"),
        PrimitiveType::Isize => ("LibC::SSizeT", "LibC::SSizeT"),
        PrimitiveType::F32 => ("Float32", "Float32"),
        PrimitiveType::F64 => ("Float64", "Float64"),
    }
}

/// Resolve a trait method into a bridgeable callback, or `None` if it uses an
/// unsupported shape (matching the FFI backend's skip behaviour).
fn resolve_callback(m: &MethodDef, context_type: &str, result_type: &str) -> Option<Callback> {
    if m.trait_source.is_some() {
        return None;
    }
    // Must return the result type and take the context.
    if !matches!(&m.return_type, TypeRef::Named(name) if name == result_type) {
        return None;
    }
    if !m
        .params
        .iter()
        .any(|p| matches!(&p.ty, TypeRef::Named(name) if name == context_type))
    {
        return None;
    }
    let mut params = Vec::new();
    for p in &m.params {
        if matches!(&p.ty, TypeRef::Named(name) if name == context_type) {
            continue; // threaded via ctx
        }
        let raw = p.name.trim_start_matches('_').to_string();
        let (c_type, hi_type, decode): (&'static str, String, String) = match (&p.ty, p.optional) {
            // Always null-check C string pointers, even for non-optional params,
            // to prevent segfaults from null C pointers (C ABI doesn't guarantee
            // non-null even when Rust declares non-optional).
            (TypeRef::String, _) => (
                "LibC::Char*",
                "String?".into(),
                format!("{raw}.null? ? nil : String.new({raw})"),
            ),
            (TypeRef::Primitive(PrimitiveType::Bool), false) => ("Int32", "Bool".into(), format!("{raw} != 0")),
            (TypeRef::Primitive(PrimitiveType::U32), false) => ("UInt32", "UInt32".into(), raw.clone()),
            (TypeRef::Primitive(PrimitiveType::Usize), false) => ("LibC::SizeT", "LibC::SizeT".into(), raw.clone()),
            // `&[String]` (e.g. `visit_table_row(cells)`) crosses as a NUL-terminated
            // C-string array pointer + count. The count is a separate C arg that the
            // decode references but which is NOT forwarded to the visitor method.
            (TypeRef::Vec(inner), _) if matches!(&**inner, TypeRef::String) => (
                "LibC::Char**",
                "Array(String)".into(),
                format!(
                    "(0...{raw}_count).compact_map {{ |i| ptr = {raw}[i]; ptr.null? ? nil : String.new(ptr) }}"
                ),
            ),
            _ => return None, // unsupported param shape → skip whole method
        };
        // For a string-array param the FFI passes a trailing `cell_count` usize arg.
        let count_twin = matches!(&p.ty, TypeRef::Vec(inner) if matches!(&**inner, TypeRef::String));
        params.push(CbParam {
            name: raw.clone(),
            c_type,
            hi_type,
            decode,
            is_count: false,
        });
        if count_twin {
            params.push(CbParam {
                name: format!("{raw}_count"),
                c_type: "LibC::SizeT",
                hi_type: "LibC::SizeT".into(),
                decode: String::new(),
                is_count: true,
            });
        }
    }
    Some(Callback {
        method: public_host_identifier(
            crate::core::config::Language::Crystal,
            PublicIdentifierKind::Function,
            &m.name,
        ),
        doc: m.doc.lines().next().unwrap_or_default().trim().to_string(),
        params,
    })
}

/// Generate the `visitor.cr` file body for a visitor-style trait bridge.
///
/// Returns `None` when the bridge is not a supported visitor bridge (missing
/// context/result types, or no bridgeable methods) so the caller can fall back.
pub(crate) fn gen_visitor_file(
    api: &ApiSurface,
    bridge: &TraitBridgeConfig,
    ffi_prefix: &str,
    lib_name: &str,
    module_name: &str,
) -> Option<String> {
    let context_type = bridge.context_type.as_deref()?;
    let result_type = bridge.result_type.as_deref()?;
    let context_def: &TypeDef = api.types.iter().find(|t| t.name == context_type)?;
    let result_meta = visitor_result_metadata(api, bridge)?;

    let trait_name = crystal_type_name(&bridge.trait_name);
    let trait_snake = public_host_identifier(
        crate::core::config::Language::Crystal,
        PublicIdentifierKind::Function,
        &bridge.trait_name,
    );
    // A visitor-specific value type holding the decoded C context. Named distinctly
    // from the context DTO (which is a JSON-serializable `class` emitted separately)
    // to avoid a same-name struct/class collision in the module.
    let ctx_hi = format!("{trait_name}VisitorContext");
    let result_hi = crystal_type_name(result_type);

    // Resolve context fields.
    let ctx_fields: Vec<CtxField> = context_def
        .fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .filter_map(|f| {
            let raw = public_host_identifier(
                crate::core::config::Language::Crystal,
                PublicIdentifierKind::Field,
                &f.name,
            );
            context_field(&f.ty, &raw, api).map(|(c_type, hi_type, decode)| CtxField {
                name: raw,
                c_type,
                hi_type,
                decode,
            })
        })
        .collect();

    // Resolve bridgeable callbacks.
    let callbacks: Vec<Callback> = context_def_methods(api, &bridge.trait_name)
        .iter()
        .filter_map(|m| resolve_callback(m, context_type, result_type))
        .collect();
    if callbacks.is_empty() {
        return None;
    }

    let default_code = result_meta.default_variant.code;
    let ctx_c_struct = format!("{trait_name}Context");
    let callbacks_c_struct = format!("{trait_name}VisitorCallbacks");

    // When the result enum carries string payloads it is emitted as a tagged-union
    // abstract class (variants are classes → constructed with `.new`); a unit-only
    // result enum is a plain Crystal `enum` (variants are members).
    let result_is_class = !result_meta.string_payload_variants.is_empty();
    let default_ctor = if result_is_class { ".new" } else { "" };

    let mut out = String::new();
    out.push_str(&format!(
        "# Visitor trait bridge for `{}` — a Crystal object implementing the Rust\n\
         # `{}` trait across the C ABI. Layout mirrors the FFI visitor vtable.\n\
         require \"json\"\n\n",
        bridge.trait_name, bridge.trait_name
    ));

    // ---- lib layer (reopens the existing lib) ------------------------------
    out.push_str(&format!("lib {lib_name}\n"));
    out.push_str(&format!("  struct {ctx_c_struct}\n"));
    for f in &ctx_fields {
        out.push_str(&format!("    {} : {}\n", f.name, f.c_type));
    }
    out.push_str("  end\n\n");

    out.push_str(&format!("  alias {trait_name}VisitorHandle = Void*\n\n"));

    out.push_str(&format!("  struct {callbacks_c_struct}\n"));
    out.push_str("    user_data : Void*\n");
    for cb in &callbacks {
        let mut sig = format!("{ctx_c_struct}*, Void*");
        for p in &cb.params {
            sig.push_str(&format!(", {}", p.c_type));
        }
        sig.push_str(", LibC::Char**, LibC::SizeT*");
        out.push_str(&format!("    {} : ({sig}) -> Int32\n", cb.method));
    }
    out.push_str("  end\n\n");

    out.push_str(&format!(
        "  fun {trait_snake}_visitor_create = {ffi_prefix}_visitor_create(callbacks : {callbacks_c_struct}*) : {trait_name}VisitorHandle\n"
    ));
    out.push_str(&format!(
        "  fun {trait_snake}_visitor_free = {ffi_prefix}_visitor_free(handle : {trait_name}VisitorHandle) : Void\n"
    ));
    out.push_str(&format!(
        "  fun {trait_snake}_options_set_visitor = {ffi_prefix}_options_set_visitor(options : Void*, handle : {trait_name}VisitorHandle) : Void\n"
    ));
    out.push_str("end\n\n");

    // ---- high-level module -------------------------------------------------
    out.push_str(&format!("module {module_name}\n"));

    // Context wrapper decoded from the C struct.
    out.push_str(&format!(
        "\n  # Decoded visitor context passed to each callback.\n  struct {ctx_hi}\n"
    ));
    for f in &ctx_fields {
        out.push_str(&format!("    getter {} : {}\n", f.name, f.hi_type));
    }
    let ctx_ctor_args = ctx_fields
        .iter()
        .map(|f| format!("@{}", f.name))
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&format!("    def initialize({ctx_ctor_args})\n    end\n  end\n"));

    // Abstract visitor base class: users subclass and override methods.
    out.push_str(&format!(
        "\n  # Subclass and override the methods you care about; each defaults to\n  # `{result_hi}::{}`.\n  abstract class {trait_name}Visitor\n",
        result_meta.default_variant.name
    ));
    for cb in &callbacks {
        let mut params = format!("ctx : {ctx_hi}");
        for p in &cb.params {
            if p.is_count {
                continue; // length twin not exposed to the visitor method
            }
            params.push_str(&format!(", {} : {}", p.name, p.hi_type));
        }
        if !cb.doc.is_empty() {
            out.push_str(&format!("    # {}\n", cb.doc));
        }
        out.push_str(&format!(
            "    def {}({params}) : {result_hi}\n      {result_hi}::{}{default_ctor}\n    end\n",
            cb.method, result_meta.default_variant.name
        ));
    }
    out.push_str("  end\n");

    // Registration: box the visitor, build callbacks with closure-free trampolines.
    out.push_str(&format!(
        "\n  # Register a Crystal visitor and return an opaque handle. Attach it to a\n  # conversion via `{lib_name}.{trait_snake}_options_set_visitor(opts, handle)`,\n  # run the conversion, then release it with `free_{trait_snake}_visitor`.\n  def self.register_{trait_snake}_visitor(impl : {trait_name}Visitor) : {lib_name}::{trait_name}VisitorHandle\n"
    ));
    out.push_str("    boxed = Box.box(impl)\n");
    out.push_str(&format!("    callbacks = {lib_name}::{callbacks_c_struct}.new\n"));
    out.push_str("    callbacks.user_data = boxed\n");
    for cb in &callbacks {
        // Closure-free trampoline (uses only its args + Box.unbox), valid as a C fn ptr.
        let mut lam_params = format!("ctx : {lib_name}::{ctx_c_struct}*, user_data : Void*");
        for p in &cb.params {
            lam_params.push_str(&format!(", {} : {}", p.name, p.c_type));
        }
        lam_params.push_str(", out_custom : LibC::Char**, out_len : LibC::SizeT*");
        out.push_str(&format!("    callbacks.{} = ->({lam_params}) do\n", cb.method));
        out.push_str("      begin\n");
        out.push_str(&format!("        visitor = Box({trait_name}Visitor).unbox(user_data)\n"));
        out.push_str("        raw = ctx.value\n");
        // Build high-level context.
        let ctx_args = ctx_fields
            .iter()
            .map(|f| f.decode.clone())
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("        context = {ctx_hi}.new({ctx_args})\n"));
        // Decode extras.
        let mut call_args = String::from("context");
        for p in &cb.params {
            if p.is_count {
                continue; // length twin — only referenced by its array's decode
            }
            out.push_str(&format!("        {}_value = {}\n", p.name, p.decode));
            call_args.push_str(&format!(", {}_value", p.name));
        }
        out.push_str(&format!("        decision = visitor.{}({call_args})\n", cb.method));
        // Map the decision to its FFI result code; string-payload variants also
        // write a malloc'd copy of the payload into `out_custom` (Rust takes
        // ownership via `CString::from_raw`, so the allocator must be libc malloc).
        out.push_str("        case decision\n");
        for v in &result_meta.unit_variants {
            out.push_str(&format!(
                "        when {result_hi}::{} then {}\n",
                crystal_type_name(&v.name),
                v.code
            ));
        }
        for v in &result_meta.string_payload_variants {
            out.push_str(&format!("        when {result_hi}::{}\n", crystal_type_name(&v.name)));
            out.push_str("          __payload = decision.value.to_slice\n");
            out.push_str("          __buf = LibC.malloc(__payload.size + 1).as(UInt8*)\n");
            out.push_str("          __buf.copy_from(__payload.to_unsafe, __payload.size)\n");
            out.push_str("          __buf[__payload.size] = 0_u8\n");
            out.push_str("          out_custom.value = __buf\n");
            out.push_str("          out_len.value = LibC::SizeT.new(__payload.size)\n");
            out.push_str(&format!("          {}\n", v.code));
        }
        out.push_str(&format!("        else {default_code}\n"));
        out.push_str("        end\n");
        // Wrap in rescue to prevent Crystal exceptions from unwinding through
        // Rust's `extern "C"` FFI (which would abort the process).
        // Print the exception to stderr for debugging, then return the default code.
        out.push_str("      rescue e\n");
        out.push_str("        STDERR.puts \"[visitor callback error] #{e}\"\n");
        out.push_str("        STDERR.puts \"[visitor callback backtrace] #{e.backtrace.first(3).join(\"\\n\")}\" if e.backtrace\n");
        out.push_str("        out_custom.value = Pointer(LibC::Char).null\n");
        out.push_str("        out_len.value = LibC::SizeT.new(0)\n");
        // Return error code (typically 4 for most visitors).
        out.push_str(&format!("        {}\n", result_meta.default_variant.code));
        out.push_str("      end\n");
        out.push_str("    end\n");
    }
    out.push_str(&format!(
        "    {lib_name}.{trait_snake}_visitor_create(pointerof(callbacks))\n"
    ));
    out.push_str("  end\n");

    out.push_str(&format!(
        "\n  # Release a visitor handle returned by `register_{trait_snake}_visitor`.\n  def self.free_{trait_snake}_visitor(handle : {lib_name}::{trait_name}VisitorHandle) : Nil\n    {lib_name}.{trait_snake}_visitor_free(handle)\n    nil\n  end\n"
    ));

    out.push_str("end\n");
    Some(out)
}

/// Collect the methods of the bridged trait from the IR.
fn context_def_methods<'a>(api: &'a ApiSurface, trait_name: &str) -> &'a [MethodDef] {
    api.types
        .iter()
        .find(|t| t.is_trait && t.name == trait_name)
        .map(|t| t.methods.as_slice())
        .unwrap_or(&[])
}

/// Whether a visitor bridge is fully emittable by the current Crystal backend.
///
/// Requires `context_type` + `result_type`, and a **unit-variant-only** result
/// enum — data-carrying (tagged-union) result variants (e.g. `Custom(String)`)
/// need Crystal tagged-union emission, which is not implemented yet. Returns
/// `false` so the caller can reject the bridge loudly rather than emit a binding
/// that references a not-yet-generated type.
pub(crate) fn is_supported_visitor_bridge(api: &ApiSurface, bridge: &TraitBridgeConfig) -> bool {
    let (Some(context_type), Some(result_type)) = (bridge.context_type.as_deref(), bridge.result_type.as_deref())
    else {
        return false;
    };
    if !api.types.iter().any(|t| t.name == context_type) {
        return false;
    }
    let Some(result_enum) = api.enums.iter().find(|e| e.name == result_type) else {
        return false;
    };
    // Every result variant must be a unit variant or a single-`String` newtype
    // (the string-payload channel). Other data shapes aren't representable yet.
    result_enum.variants.iter().all(|v| {
        let is_unit = v.fields.is_empty() && !v.is_tuple && !v.originally_had_data_fields;
        let is_string_payload = v.is_tuple && v.fields.len() == 1 && matches!(v.fields[0].ty, TypeRef::String);
        is_unit || is_string_payload
    })
}

// ===========================================================================
// Plugin-style trait bridges (register_fn / registry pattern)
// ===========================================================================

/// The Crystal `lib` C type for a plugin-bridge parameter/field, matching the
/// FFI `prim_to_c` + JSON conventions (bool → i32, complex → JSON `char*`).
fn plugin_c_type(ty: &TypeRef) -> Option<&'static str> {
    Some(match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "LibC::Char*",
        TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Bytes => "LibC::Char*",
        TypeRef::Optional(_) => "LibC::Char*",
        TypeRef::Primitive(PrimitiveType::Bool) => "Int32",
        TypeRef::Primitive(PrimitiveType::U8) => "UInt8",
        TypeRef::Primitive(PrimitiveType::U16) => "UInt16",
        TypeRef::Primitive(PrimitiveType::U32) => "UInt32",
        TypeRef::Primitive(PrimitiveType::U64) => "UInt64",
        TypeRef::Primitive(PrimitiveType::I8) => "Int8",
        TypeRef::Primitive(PrimitiveType::I16) => "Int16",
        TypeRef::Primitive(PrimitiveType::I32) => "Int32",
        TypeRef::Primitive(PrimitiveType::I64) => "Int64",
        TypeRef::Primitive(PrimitiveType::F32) => "Float32",
        TypeRef::Primitive(PrimitiveType::F64) => "Float64",
        TypeRef::Primitive(PrimitiveType::Usize) => "LibC::SizeT",
        TypeRef::Primitive(PrimitiveType::Isize) => "LibC::SSizeT",
        TypeRef::Duration => "UInt64",
        _ => return None, // Unit params unsupported for now
    })
}

/// Whether a plugin (registry) trait bridge is fully emittable.
pub(crate) fn is_supported_plugin_bridge(api: &ApiSurface, bridge: &TraitBridgeConfig) -> bool {
    if bridge.register_fn.is_none() {
        return false;
    }
    let Some(trait_def) = api.types.iter().find(|t| t.is_trait && t.name == bridge.trait_name) else {
        return false;
    };
    trait_def
        .methods
        .iter()
        .filter(|m| m.trait_source.is_none() && !m.binding_excluded)
        .all(plugin_method_supported)
}

/// A method is supported when all params map to a C type and the return shape is
/// one we can marshal (Unit, String-ish, JSON-able, or infallible scalar).
fn plugin_method_supported(m: &MethodDef) -> bool {
    if m.params.iter().any(|p| plugin_c_type(&p.ty).is_none()) {
        return false;
    }
    match &m.return_type {
        TypeRef::Unit
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Path
        | TypeRef::Json
        | TypeRef::Bytes
        | TypeRef::Named(_)
        | TypeRef::Vec(_)
        | TypeRef::Map(_, _) => true,
        TypeRef::Optional(inner) => !matches!(inner.as_ref(), TypeRef::Unit | TypeRef::Optional(_)),
        TypeRef::Primitive(_) | TypeRef::Duration => m.error_type.is_none(),
    }
}

/// Return `true` when a return type crosses the ABI as a JSON `out_result` string
/// (as opposed to a raw string or a by-value scalar).
fn plugin_returns_json(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Bytes
    )
}

fn plugin_returns_string(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json)
}

/// Generate the `<trait>_plugin.cr` file for a plugin-style trait bridge, or
/// `None` when the bridge is not a supported plugin bridge.
pub(crate) fn gen_plugin_file(
    api: &ApiSurface,
    bridge: &TraitBridgeConfig,
    ffi_prefix: &str,
    lib_name: &str,
    module_name: &str,
) -> Option<String> {
    if !is_supported_plugin_bridge(api, bridge) {
        return None;
    }
    let trait_def = api.types.iter().find(|t| t.is_trait && t.name == bridge.trait_name)?;
    let trait_name = crystal_type_name(&bridge.trait_name);
    let trait_snake = public_host_identifier(
        crate::core::config::Language::Crystal,
        PublicIdentifierKind::Function,
        &bridge.trait_name,
    );
    let vtable_ty = format!("{trait_name}VTable");
    let has_super = bridge.super_trait.is_some();
    let register_fn = bridge.register_fn.as_deref()?;
    let plugins_var = format!("@@{trait_snake}_plugins");

    let own_methods: Vec<&MethodDef> = trait_def
        .methods
        .iter()
        .filter(|m| m.trait_source.is_none() && !m.binding_excluded)
        .collect();

    let mut out = String::new();
    out.push_str(&format!(
        "# Plugin trait bridge for `{}` — a Crystal object registered into the Rust\n# `{}` registry, implementing the trait across the C-ABI vtable.\nrequire \"json\"\n\n",
        bridge.trait_name, bridge.trait_name
    ));

    // ---- lib layer ---------------------------------------------------------
    out.push_str(&format!("lib {lib_name}\n"));
    out.push_str(&format!("  struct {vtable_ty}\n"));
    if has_super {
        out.push_str("    name_fn : (Void*, LibC::Char**, LibC::Char**) -> Int32\n");
        out.push_str("    version_fn : (Void*, LibC::Char**, LibC::Char**) -> Int32\n");
        out.push_str("    initialize_fn : (Void*, LibC::Char**) -> Int32\n");
        out.push_str("    shutdown_fn : (Void*, LibC::Char**) -> Int32\n");
    }
    for m in &own_methods {
        let field = public_host_identifier(
            crate::core::config::Language::Crystal,
            PublicIdentifierKind::Function,
            &m.name,
        );
        let mut params = vec!["Void*".to_string()];
        for p in &m.params {
            params.push(plugin_c_type(&p.ty)?.to_string());
        }
        let (ret, out_ps) = plugin_vtable_return(&m.return_type, m.error_type.is_some());
        params.extend(out_ps.iter().map(|s| s.to_string()));
        out.push_str(&format!("    {field} : ({}) -> {ret}\n", params.join(", ")));
    }
    out.push_str("    free_string : (LibC::Char*) -> Void\n");
    out.push_str("    free_user_data : (Void*) -> Void\n");
    out.push_str("  end\n\n");
    out.push_str(&format!(
        "  fun register_{trait_snake} = {ffi_prefix}_{register_fn}(name : LibC::Char*, vtable : {vtable_ty}*, user_data : Void*, out_error : LibC::Char**) : Int32\n"
    ));
    out.push_str(&format!(
        "  fun unregister_{trait_snake} = {ffi_prefix}_unregister_{trait_snake}(name : LibC::Char*, out_error : LibC::Char**) : Int32\n"
    ));
    out.push_str("end\n\n");

    // ---- high-level module -------------------------------------------------
    out.push_str(&format!("module {module_name}\n"));
    // Keep boxed impls alive for the registration lifetime (conservative GC scans this).
    out.push_str(&format!("  {plugins_var} = {{}} of String => Void*\n"));

    // Abstract base class.
    out.push_str(&format!(
        "\n  # Subclass and override the trait methods to implement `{}`.\n  abstract class {trait_name}\n",
        bridge.trait_name
    ));
    if has_super {
        out.push_str("    def name : String\n      \"\"\n    end\n");
        out.push_str("    def version : String\n      \"0.0.0\"\n    end\n");
        out.push_str("    def initialize_plugin : Nil\n    end\n");
        out.push_str("    def shutdown : Nil\n    end\n");
    }
    for m in &own_methods {
        let method = public_host_identifier(
            crate::core::config::Language::Crystal,
            PublicIdentifierKind::Function,
            &m.name,
        );
        let sig_params = m
            .params
            .iter()
            .map(|p| {
                let n = public_host_identifier(
                    crate::core::config::Language::Crystal,
                    PublicIdentifierKind::Parameter,
                    &p.name,
                );
                format!("{n} : {}", crystal_type(&p.ty))
            })
            .collect::<Vec<_>>()
            .join(", ");
        let ret_hi = if matches!(m.return_type, TypeRef::Unit) {
            "Nil".to_string()
        } else {
            crystal_type(&m.return_type).into_owned()
        };
        let params_decl = if sig_params.is_empty() {
            String::new()
        } else {
            format!("({sig_params})")
        };
        if !m.doc.is_empty() {
            out.push_str(&format!("    # {}\n", m.doc.lines().next().unwrap_or_default().trim()));
        }
        out.push_str(&format!(
            "    def {method}{params_decl} : {ret_hi}\n      raise \"not implemented: {method}\"\n    end\n"
        ));
    }
    out.push_str("  end\n");

    // C-string dup helper (idempotent redefinition across bridge files is fine).
    out.push_str(
        "\n  # Copy a Crystal String to a malloc'd NUL-terminated C string (Rust frees it via free_string).\n  def self.__alef_dup_cstr(s : String) : LibC::Char*\n    bytes = s.to_slice\n    buf = LibC.malloc(bytes.size + 1).as(UInt8*)\n    buf.copy_from(bytes.to_unsafe, bytes.size)\n    buf[bytes.size] = 0_u8\n    buf.as(LibC::Char*)\n  end\n",
    );

    // register_<trait>(name, impl)
    out.push_str(&format!(
        "\n  # Register a Crystal `{trait_name}` implementation into the Rust registry.\n  def self.register_{trait_snake}(name : String, impl : {trait_name}) : Bool\n"
    ));
    out.push_str("    ud = Box.box(impl)\n");
    out.push_str(&format!("    {plugins_var}[name] = ud\n"));
    out.push_str(&format!("    vtable = {lib_name}::{vtable_ty}.new\n"));
    if has_super {
        out.push_str(&format!("    vtable.name_fn = ->(user_data : Void*, out_name : LibC::Char**, out_error : LibC::Char**) do\n      out_name.value = {module_name}.__alef_dup_cstr(Box({trait_name}).unbox(user_data).name)\n      0\n    end\n"));
        out.push_str(&format!("    vtable.version_fn = ->(user_data : Void*, out_version : LibC::Char**, out_error : LibC::Char**) do\n      out_version.value = {module_name}.__alef_dup_cstr(Box({trait_name}).unbox(user_data).version)\n      0\n    end\n"));
        out.push_str(&format!("    vtable.initialize_fn = ->(user_data : Void*, out_error : LibC::Char**) do\n      Box({trait_name}).unbox(user_data).initialize_plugin\n      0\n    end\n"));
        out.push_str(&format!("    vtable.shutdown_fn = ->(user_data : Void*, out_error : LibC::Char**) do\n      Box({trait_name}).unbox(user_data).shutdown\n      0\n    end\n"));
    }
    for m in &own_methods {
        out.push_str(&gen_plugin_method_trampoline(m, &trait_name, module_name));
    }
    out.push_str("    vtable.free_string = ->(p : LibC::Char*) { LibC.free(p.as(Void*)) }\n");
    out.push_str("    out_error = Pointer(LibC::Char).null\n");
    out.push_str(&format!(
        "    {lib_name}.register_{trait_snake}(name, pointerof(vtable), ud, pointerof(out_error)) == 0\n"
    ));
    out.push_str("  end\n");

    // unregister_<trait>(name)
    out.push_str(&format!(
        "\n  # Unregister a previously registered `{trait_name}` implementation.\n  def self.unregister_{trait_snake}(name : String) : Bool\n    out_error = Pointer(LibC::Char).null\n    ok = {lib_name}.unregister_{trait_snake}(name, pointerof(out_error)) == 0\n    {plugins_var}.delete(name)\n    ok\n  end\n"
    ));

    out.push_str("end\n");
    Some(out)
}

/// The Crystal vtable fn-pointer return type + trailing out-params for a method.
fn plugin_vtable_return(ty: &TypeRef, has_error: bool) -> (&'static str, Vec<&'static str>) {
    if plugin_returns_string(ty) || plugin_returns_json(ty) {
        return ("Int32", vec!["LibC::Char**", "LibC::Char**"]); // out_result, out_error
    }
    match ty {
        TypeRef::Unit => {
            if has_error {
                ("Int32", vec!["LibC::Char**"]) // out_error
            } else {
                ("Void", vec![])
            }
        }
        TypeRef::Primitive(p) => (prim_ret(p), vec![]),
        TypeRef::Duration => ("UInt64", vec![]),
        _ => ("Int32", vec!["LibC::Char**", "LibC::Char**"]),
    }
}

fn prim_ret(p: &PrimitiveType) -> &'static str {
    match p {
        PrimitiveType::Bool => "Int32",
        PrimitiveType::U8 => "UInt8",
        PrimitiveType::U16 => "UInt16",
        PrimitiveType::U32 => "UInt32",
        PrimitiveType::U64 => "UInt64",
        PrimitiveType::I8 => "Int8",
        PrimitiveType::I16 => "Int16",
        PrimitiveType::I32 => "Int32",
        PrimitiveType::I64 => "Int64",
        PrimitiveType::F32 => "Float32",
        PrimitiveType::F64 => "Float64",
        PrimitiveType::Usize => "LibC::SizeT",
        PrimitiveType::Isize => "LibC::SSizeT",
    }
}

/// Emit one own-method trampoline assigned to the vtable field.
fn gen_plugin_method_trampoline(m: &MethodDef, trait_name: &str, module_name: &str) -> String {
    let field = public_host_identifier(
        crate::core::config::Language::Crystal,
        PublicIdentifierKind::Function,
        &m.name,
    );
    // fn-pointer parameter list.
    let mut lam = vec!["user_data : Void*".to_string()];
    let mut decoded: Vec<String> = Vec::new();
    let mut call_args: Vec<String> = Vec::new();
    for p in &m.params {
        let n = public_host_identifier(
            crate::core::config::Language::Crystal,
            PublicIdentifierKind::Parameter,
            &p.name,
        );
        let c = plugin_c_type(&p.ty).unwrap_or("LibC::Char*");
        lam.push(format!("{n} : {c}"));
        let (val, expr) = plugin_decode_param(&p.ty, &n);
        decoded.push(format!("      {val} = {expr}\n"));
        call_args.push(val);
    }
    let (_, out_ps) = plugin_vtable_return(&m.return_type, m.error_type.is_some());
    let has_out_result =
        out_ps.len() == 2 || plugin_returns_string(&m.return_type) || plugin_returns_json(&m.return_type);
    let has_out_error =
        out_ps.contains(&"LibC::Char**") && (out_ps.len() == 2 || matches!(m.return_type, TypeRef::Unit));
    if has_out_result {
        lam.push("out_result : LibC::Char**".to_string());
        lam.push("out_error : LibC::Char**".to_string());
    } else if has_out_error {
        lam.push("out_error : LibC::Char**".to_string());
    }

    let call = format!("Box({trait_name}).unbox(user_data).{field}({})", call_args.join(", "));
    let mut body = String::new();
    for d in &decoded {
        body.push_str(d);
    }

    // Result epilogue.
    if plugin_returns_string(&m.return_type) {
        body.push_str("      begin\n");
        body.push_str(&format!(
            "        out_result.value = {module_name}.__alef_dup_cstr({call})\n        0\n"
        ));
        body.push_str("      rescue __ex\n        out_error.value = ");
        body.push_str(&format!(
            "{module_name}.__alef_dup_cstr(__ex.message || \"error\")\n        1\n      end\n"
        ));
    } else if matches!(m.return_type, TypeRef::Bytes) {
        body.push_str("      begin\n");
        body.push_str(&format!(
            "        out_result.value = {module_name}.__alef_dup_cstr(({call}).to_a.to_json)\n        0\n"
        ));
        body.push_str(&format!("      rescue __ex\n        out_error.value = {module_name}.__alef_dup_cstr(__ex.message || \"error\")\n        1\n      end\n"));
    } else if plugin_returns_json(&m.return_type) {
        body.push_str("      begin\n");
        body.push_str(&format!(
            "        out_result.value = {module_name}.__alef_dup_cstr(({call}).to_json)\n        0\n"
        ));
        body.push_str(&format!("      rescue __ex\n        out_error.value = {module_name}.__alef_dup_cstr(__ex.message || \"error\")\n        1\n      end\n"));
    } else if matches!(m.return_type, TypeRef::Unit) {
        if m.error_type.is_some() {
            body.push_str(&format!("      begin\n        {call}\n        0\n      rescue __ex\n        out_error.value = {module_name}.__alef_dup_cstr(__ex.message || \"error\")\n        1\n      end\n"));
        } else {
            body.push_str(&format!("      {call}\n"));
        }
    } else {
        // Infallible scalar: return the value directly (Bool → Int32).
        if matches!(m.return_type, TypeRef::Primitive(PrimitiveType::Bool)) {
            body.push_str(&format!("      ({call}) ? 1 : 0\n"));
        } else {
            body.push_str(&format!("      {call}\n"));
        }
    }

    format!("    vtable.{field} = ->({}) do\n{body}    end\n", lam.join(", "))
}

/// Decode a vtable param (raw string / JSON / scalar) into a Crystal value.
fn plugin_decode_param(ty: &TypeRef, name: &str) -> (String, String) {
    let val = format!("__{name}");
    let expr = match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path => format!("String.new({name})"),
        TypeRef::Json => format!("JSON.parse(String.new({name}))"),
        TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            format!("{}.from_json(String.new({name}))", crystal_type(ty))
        }
        TypeRef::Bytes => {
            format!(
                "(begin; __arr = Array(UInt8).from_json(String.new({name})); Bytes.new(__arr.to_unsafe, __arr.size); end)"
            )
        }
        TypeRef::Optional(inner) => {
            let (_, inner_expr) = plugin_decode_param(inner, name);
            format!("{name}.null? ? nil : ({inner_expr})")
        }
        TypeRef::Primitive(PrimitiveType::Bool) => format!("({name} != 0)"),
        _ => name.to_string(),
    };
    (val, expr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{MethodDef, ParamDef, PrimitiveType, TypeRef};

    fn method_with_param(ty: TypeRef) -> MethodDef {
        MethodDef {
            name: "do_work".to_string(),
            params: vec![ParamDef {
                name: "input".to_string(),
                ty,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Unit,
            ..MethodDef::default()
        }
    }

    // ── plugin_c_type ─────────────────────────────────────────────────

    #[test]
    fn optional_string_param_has_c_type() {
        let ctype = plugin_c_type(&TypeRef::Optional(Box::new(TypeRef::String)));
        assert_eq!(ctype, Some("LibC::Char*"));
    }

    #[test]
    fn optional_named_param_has_c_type() {
        let ctype = plugin_c_type(&TypeRef::Optional(Box::new(TypeRef::Named("Foo".to_string()))));
        assert_eq!(ctype, Some("LibC::Char*"));
    }

    #[test]
    fn optional_vec_param_has_c_type() {
        let ctype = plugin_c_type(&TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::String)))));
        assert_eq!(ctype, Some("LibC::Char*"));
    }

    #[test]
    fn optional_scalar_param_has_c_type() {
        let ctype = plugin_c_type(&TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I32))));
        assert_eq!(ctype, Some("LibC::Char*"));
    }

    // ── plugin_method_supported ────────────────────────────────────────

    #[test]
    fn method_with_optional_string_param_is_supported() {
        let m = method_with_param(TypeRef::Optional(Box::new(TypeRef::String)));
        assert!(plugin_method_supported(&m));
    }

    #[test]
    fn method_with_optional_named_param_is_supported() {
        let m = method_with_param(TypeRef::Optional(Box::new(TypeRef::Named("Foo".to_string()))));
        assert!(plugin_method_supported(&m));
    }

    #[test]
    fn bytes_params_are_supported() {
        let m = method_with_param(TypeRef::Bytes);
        assert!(plugin_method_supported(&m));
    }

    #[test]
    fn bytes_return_type_is_supported() {
        let m = MethodDef {
            name: "get_data".to_string(),
            return_type: TypeRef::Bytes,
            ..MethodDef::default()
        };
        assert!(plugin_method_supported(&m));
    }

    #[test]
    fn decode_optional_string_emits_null_check() {
        let (val, expr) = plugin_decode_param(&TypeRef::Optional(Box::new(TypeRef::String)), "data");
        assert_eq!(val, "__data");
        assert!(expr.contains("data.null? ? nil"), "expr: {expr}");
        assert!(expr.contains("String.new(data)"), "expr: {expr}");
    }

    #[test]
    fn decode_optional_named_emits_null_check_with_from_json() {
        let (val, expr) = plugin_decode_param(&TypeRef::Optional(Box::new(TypeRef::Named("Foo".to_string()))), "cfg");
        assert_eq!(val, "__cfg");
        assert!(expr.contains("cfg.null? ? nil"), "expr: {expr}");
        assert!(expr.contains("Foo.from_json"), "expr: {expr}");
    }

    // ── plugin_c_type base types ───────────────────────────────────────

    #[test]
    fn string_base_type_has_c_type() {
        assert_eq!(plugin_c_type(&TypeRef::String), Some("LibC::Char*"));
    }

    #[test]
    fn char_base_type_has_c_type() {
        assert_eq!(plugin_c_type(&TypeRef::Char), Some("LibC::Char*"));
    }

    #[test]
    fn path_base_type_has_c_type() {
        assert_eq!(plugin_c_type(&TypeRef::Path), Some("LibC::Char*"));
    }

    #[test]
    fn json_base_type_has_c_type() {
        assert_eq!(plugin_c_type(&TypeRef::Json), Some("LibC::Char*"));
    }

    #[test]
    fn named_base_type_has_c_type() {
        assert_eq!(plugin_c_type(&TypeRef::Named("Foo".to_string())), Some("LibC::Char*"));
    }

    #[test]
    fn vec_base_type_has_c_type() {
        assert_eq!(
            plugin_c_type(&TypeRef::Vec(Box::new(TypeRef::String))),
            Some("LibC::Char*")
        );
    }

    #[test]
    fn map_base_type_has_c_type() {
        assert_eq!(
            plugin_c_type(&TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Primitive(PrimitiveType::I32))
            )),
            Some("LibC::Char*")
        );
    }

    #[test]
    fn bytes_base_type_has_c_type() {
        assert_eq!(plugin_c_type(&TypeRef::Bytes), Some("LibC::Char*"));
    }

    #[test]
    fn duration_has_u64_c_type() {
        assert_eq!(plugin_c_type(&TypeRef::Duration), Some("UInt64"));
    }

    #[test]
    fn all_primitive_types_have_c_type() {
        let cases: Vec<(TypeRef, &str)> = vec![
            (TypeRef::Primitive(PrimitiveType::Bool), "Int32"),
            (TypeRef::Primitive(PrimitiveType::U8), "UInt8"),
            (TypeRef::Primitive(PrimitiveType::U16), "UInt16"),
            (TypeRef::Primitive(PrimitiveType::U32), "UInt32"),
            (TypeRef::Primitive(PrimitiveType::U64), "UInt64"),
            (TypeRef::Primitive(PrimitiveType::I8), "Int8"),
            (TypeRef::Primitive(PrimitiveType::I16), "Int16"),
            (TypeRef::Primitive(PrimitiveType::I32), "Int32"),
            (TypeRef::Primitive(PrimitiveType::I64), "Int64"),
            (TypeRef::Primitive(PrimitiveType::F32), "Float32"),
            (TypeRef::Primitive(PrimitiveType::F64), "Float64"),
            (TypeRef::Primitive(PrimitiveType::Usize), "LibC::SizeT"),
            (TypeRef::Primitive(PrimitiveType::Isize), "LibC::SSizeT"),
        ];
        for (ty, expected) in &cases {
            assert_eq!(plugin_c_type(ty), Some(*expected), "plugin_c_type for {ty:?}");
        }
    }

    #[test]
    fn unit_param_returns_none() {
        assert_eq!(plugin_c_type(&TypeRef::Unit), None);
    }

    // ── plugin_returns_json / plugin_returns_string ────────────────────

    #[test]
    fn named_returns_json() {
        assert!(plugin_returns_json(&TypeRef::Named("Foo".to_string())));
    }

    #[test]
    fn vec_returns_json() {
        assert!(plugin_returns_json(&TypeRef::Vec(Box::new(TypeRef::String))));
    }

    #[test]
    fn map_returns_json() {
        assert!(plugin_returns_json(&TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Primitive(PrimitiveType::I32))
        )));
    }

    #[test]
    fn bytes_returns_json() {
        assert!(plugin_returns_json(&TypeRef::Bytes));
    }

    #[test]
    fn string_does_not_return_json() {
        assert!(!plugin_returns_json(&TypeRef::String));
    }

    #[test]
    fn scalar_does_not_return_json() {
        assert!(!plugin_returns_json(&TypeRef::Primitive(PrimitiveType::I32)));
    }

    #[test]
    fn string_returns_string() {
        assert!(plugin_returns_string(&TypeRef::String));
    }

    #[test]
    fn char_returns_string() {
        assert!(plugin_returns_string(&TypeRef::Char));
    }

    #[test]
    fn path_returns_string() {
        assert!(plugin_returns_string(&TypeRef::Path));
    }

    #[test]
    fn json_returns_string() {
        assert!(plugin_returns_string(&TypeRef::Json));
    }

    #[test]
    fn named_does_not_return_string() {
        assert!(!plugin_returns_string(&TypeRef::Named("Foo".to_string())));
    }

    #[test]
    fn scalar_does_not_return_string() {
        assert!(!plugin_returns_string(&TypeRef::Primitive(PrimitiveType::I32)));
    }

    // ── plugin_method_supported return types ────────────────────────────

    #[test]
    fn unit_return_is_supported() {
        let m = MethodDef {
            name: "nothing".to_string(),
            return_type: TypeRef::Unit,
            ..MethodDef::default()
        };
        assert!(plugin_method_supported(&m));
    }

    #[test]
    fn string_return_is_supported() {
        let m = MethodDef {
            name: "greet".to_string(),
            return_type: TypeRef::String,
            ..MethodDef::default()
        };
        assert!(plugin_method_supported(&m));
    }

    #[test]
    fn optional_string_return_is_supported() {
        let m = MethodDef {
            name: "maybe".to_string(),
            return_type: TypeRef::Optional(Box::new(TypeRef::String)),
            ..MethodDef::default()
        };
        assert!(plugin_method_supported(&m));
    }

    #[test]
    fn optional_unit_return_is_not_supported() {
        let m = MethodDef {
            name: "maybe_nil".to_string(),
            return_type: TypeRef::Optional(Box::new(TypeRef::Unit)),
            ..MethodDef::default()
        };
        assert!(!plugin_method_supported(&m));
    }

    #[test]
    fn double_optional_return_is_not_supported() {
        let m = MethodDef {
            name: "nested".to_string(),
            return_type: TypeRef::Optional(Box::new(TypeRef::Optional(Box::new(TypeRef::String)))),
            ..MethodDef::default()
        };
        assert!(!plugin_method_supported(&m));
    }

    #[test]
    fn primitive_return_with_error_is_not_supported() {
        let m = MethodDef {
            name: "risky".to_string(),
            return_type: TypeRef::Primitive(PrimitiveType::I32),
            error_type: Some("E".to_string()),
            ..MethodDef::default()
        };
        assert!(!plugin_method_supported(&m));
    }

    #[test]
    fn vec_return_is_supported() {
        let m = MethodDef {
            name: "items".to_string(),
            return_type: TypeRef::Vec(Box::new(TypeRef::String)),
            ..MethodDef::default()
        };
        assert!(plugin_method_supported(&m));
    }

    #[test]
    fn map_return_is_supported() {
        let m = MethodDef {
            name: "dict".to_string(),
            return_type: TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Primitive(PrimitiveType::I32)),
            ),
            ..MethodDef::default()
        };
        assert!(plugin_method_supported(&m));
    }

    #[test]
    fn named_return_is_supported() {
        let m = MethodDef {
            name: "config".to_string(),
            return_type: TypeRef::Named("MyConfig".to_string()),
            ..MethodDef::default()
        };
        assert!(plugin_method_supported(&m));
    }

    // ── plugin_decode_param base types ─────────────────────────────────

    #[test]
    fn decode_string_param() {
        let (val, expr) = plugin_decode_param(&TypeRef::String, "text");
        assert_eq!(val, "__text");
        assert_eq!(expr, "String.new(text)");
    }

    #[test]
    fn decode_json_param() {
        let (val, expr) = plugin_decode_param(&TypeRef::Json, "payload");
        assert_eq!(val, "__payload");
        assert_eq!(expr, "JSON.parse(String.new(payload))");
    }

    #[test]
    fn decode_named_param() {
        let (val, expr) = plugin_decode_param(&TypeRef::Named("Foo".to_string()), "obj");
        assert_eq!(val, "__obj");
        assert_eq!(expr, "Foo.from_json(String.new(obj))");
    }

    #[test]
    fn decode_vec_param() {
        let (val, expr) = plugin_decode_param(&TypeRef::Vec(Box::new(TypeRef::String)), "items");
        assert_eq!(val, "__items");
        assert_eq!(expr, "Array(String).from_json(String.new(items))");
    }

    #[test]
    fn decode_map_param() {
        let (val, expr) = plugin_decode_param(
            &TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Primitive(PrimitiveType::I32)),
            ),
            "mapping",
        );
        assert_eq!(val, "__mapping");
        assert_eq!(expr, "Hash(String, Int32).from_json(String.new(mapping))");
    }

    #[test]
    fn decode_bytes_param() {
        let (val, expr) = plugin_decode_param(&TypeRef::Bytes, "data");
        assert_eq!(val, "__data");
        assert!(
            expr.contains("Array(UInt8).from_json(String.new(data))"),
            "expr: {expr}"
        );
        assert!(expr.contains("Bytes.new(__arr.to_unsafe"), "expr: {expr}");
    }

    #[test]
    fn decode_bool_param() {
        let (val, expr) = plugin_decode_param(&TypeRef::Primitive(PrimitiveType::Bool), "flag");
        assert_eq!(val, "__flag");
        assert_eq!(expr, "(flag != 0)");
    }

    #[test]
    fn decode_duration_param() {
        let (val, expr) = plugin_decode_param(&TypeRef::Duration, "timeout");
        assert_eq!(val, "__timeout");
        assert_eq!(expr, "timeout");
    }

    #[test]
    fn decode_i32_param() {
        let (val, expr) = plugin_decode_param(&TypeRef::Primitive(PrimitiveType::I32), "n");
        assert_eq!(val, "__n");
        assert_eq!(expr, "n");
    }

    // ── crystal_scalar ─────────────────────────────────────────────────

    #[test]
    fn scalar_bool() {
        assert_eq!(crystal_scalar(&PrimitiveType::Bool), ("Int32", "Bool"));
    }

    #[test]
    fn scalar_u8() {
        assert_eq!(crystal_scalar(&PrimitiveType::U8), ("UInt8", "UInt8"));
    }

    #[test]
    fn scalar_i64() {
        assert_eq!(crystal_scalar(&PrimitiveType::I64), ("Int64", "Int64"));
    }

    #[test]
    fn scalar_usize() {
        assert_eq!(crystal_scalar(&PrimitiveType::Usize), ("LibC::SizeT", "LibC::SizeT"));
    }

    #[test]
    fn scalar_f32() {
        assert_eq!(crystal_scalar(&PrimitiveType::F32), ("Float32", "Float32"));
    }

    // ── resolve_callback ───────────────────────────────────────────────

    fn tracer_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params,
            return_type,
            ..MethodDef::default()
        }
    }

    #[test]
    fn callback_with_trait_source_is_skipped() {
        let mut m = tracer_method(
            "visit",
            vec![
                ParamDef {
                    name: "ctx".into(),
                    ty: TypeRef::Named("Ctx".into()),
                    ..Default::default()
                },
                ParamDef {
                    name: "_text".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                },
            ],
            TypeRef::Named("Decision".into()),
        );
        m.trait_source = Some("SuperTrait".into());
        assert!(resolve_callback(&m, "Ctx", "Decision").is_none());
    }

    #[test]
    fn callback_with_wrong_return_type_is_skipped() {
        let m = tracer_method(
            "visit",
            vec![
                ParamDef {
                    name: "ctx".into(),
                    ty: TypeRef::Named("Ctx".into()),
                    ..Default::default()
                },
                ParamDef {
                    name: "_text".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                },
            ],
            TypeRef::Named("WrongType".into()),
        );
        assert!(resolve_callback(&m, "Ctx", "Decision").is_none());
    }

    #[test]
    fn callback_without_context_param_is_skipped() {
        let m = tracer_method(
            "visit",
            vec![ParamDef {
                name: "_text".into(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            TypeRef::Named("Decision".into()),
        );
        assert!(resolve_callback(&m, "Ctx", "Decision").is_none());
    }

    #[test]
    fn callback_with_unsupported_param_type_is_skipped() {
        let m = tracer_method(
            "visit",
            vec![
                ParamDef {
                    name: "ctx".into(),
                    ty: TypeRef::Named("Ctx".into()),
                    ..Default::default()
                },
                ParamDef {
                    name: "_data".into(),
                    ty: TypeRef::Bytes,
                    ..Default::default()
                },
            ],
            TypeRef::Named("Decision".into()),
        );
        assert!(resolve_callback(&m, "Ctx", "Decision").is_none());
    }

    #[test]
    fn callback_with_string_param_resolves() {
        let m = tracer_method(
            "visit_text",
            vec![
                ParamDef {
                    name: "ctx".into(),
                    ty: TypeRef::Named("Ctx".into()),
                    ..Default::default()
                },
                ParamDef {
                    name: "_text".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                },
            ],
            TypeRef::Named("Decision".into()),
        );
        let cb = resolve_callback(&m, "Ctx", "Decision").expect("should resolve");
        assert_eq!(cb.method, "visit_text");
        assert_eq!(cb.params.len(), 1);
        assert_eq!(cb.params[0].name, "text");
        assert_eq!(cb.params[0].c_type, "LibC::Char*");
        assert_eq!(cb.params[0].hi_type, "String?");
        assert!(cb.params[0].decode.contains("text.null? ? nil : String.new(text)"));
    }

    #[test]
    fn callback_with_optional_string_resolves() {
        let m = tracer_method(
            "visit",
            vec![
                ParamDef {
                    name: "ctx".into(),
                    ty: TypeRef::Named("Ctx".into()),
                    ..Default::default()
                },
                ParamDef {
                    name: "_label".into(),
                    ty: TypeRef::String,
                    optional: true,
                    ..Default::default()
                },
            ],
            TypeRef::Named("Decision".into()),
        );
        let cb = resolve_callback(&m, "Ctx", "Decision").expect("should resolve");
        assert_eq!(cb.params.len(), 1);
        assert_eq!(cb.params[0].name, "label");
        assert_eq!(cb.params[0].hi_type, "String?");
        assert!(
            cb.params[0].decode.contains("nil"),
            "decode should handle nil: {}",
            cb.params[0].decode
        );
    }

    #[test]
    fn callback_with_bool_param_resolves() {
        let m = tracer_method(
            "visit",
            vec![
                ParamDef {
                    name: "ctx".into(),
                    ty: TypeRef::Named("Ctx".into()),
                    ..Default::default()
                },
                ParamDef {
                    name: "_flag".into(),
                    ty: TypeRef::Primitive(PrimitiveType::Bool),
                    ..Default::default()
                },
            ],
            TypeRef::Named("Decision".into()),
        );
        let cb = resolve_callback(&m, "Ctx", "Decision").expect("should resolve");
        assert_eq!(cb.params.len(), 1);
        assert_eq!(cb.params[0].name, "flag");
        assert_eq!(cb.params[0].c_type, "Int32");
        assert_eq!(cb.params[0].hi_type, "Bool");
        assert_eq!(cb.params[0].decode, "flag != 0");
    }

    #[test]
    fn callback_with_only_context_param_resolves_empty_params() {
        let m = tracer_method(
            "simple",
            vec![ParamDef {
                name: "ctx".into(),
                ty: TypeRef::Named("Ctx".into()),
                ..Default::default()
            }],
            TypeRef::Named("Decision".into()),
        );
        let cb = resolve_callback(&m, "Ctx", "Decision").expect("should resolve");
        assert!(cb.params.is_empty(), "no non-context params expected");
    }

    #[test]
    fn callback_uses_first_doc_line() {
        let mut m = tracer_method(
            "inspect",
            vec![ParamDef {
                name: "ctx".into(),
                ty: TypeRef::Named("Ctx".into()),
                ..Default::default()
            }],
            TypeRef::Named("Decision".into()),
        );
        m.doc = "First line.\nSecond line.".into();
        let cb = resolve_callback(&m, "Ctx", "Decision").expect("should resolve");
        assert_eq!(cb.doc, "First line.");
    }

    // ── prim_ret ───────────────────────────────────────────────────────

    #[test]
    fn prim_ret_bool() {
        assert_eq!(prim_ret(&PrimitiveType::Bool), "Int32");
    }

    #[test]
    fn prim_ret_u32() {
        assert_eq!(prim_ret(&PrimitiveType::U32), "UInt32");
    }

    #[test]
    fn prim_ret_f64() {
        assert_eq!(prim_ret(&PrimitiveType::F64), "Float64");
    }

    #[test]
    fn prim_ret_usize() {
        assert_eq!(prim_ret(&PrimitiveType::Usize), "LibC::SizeT");
    }

    // ── plugin_vtable_return ────────────────────────────────────────────

    #[test]
    fn vtable_return_unit_no_error() {
        let (ret, out) = plugin_vtable_return(&TypeRef::Unit, false);
        assert_eq!(ret, "Void");
        assert!(out.is_empty());
    }

    #[test]
    fn vtable_return_unit_with_error() {
        let (ret, out) = plugin_vtable_return(&TypeRef::Unit, true);
        assert_eq!(ret, "Int32");
        assert_eq!(out, vec!["LibC::Char**"]);
    }

    #[test]
    fn vtable_return_primitive_bool() {
        let (ret, out) = plugin_vtable_return(&TypeRef::Primitive(PrimitiveType::Bool), false);
        assert_eq!(ret, "Int32");
        assert!(out.is_empty());
    }

    #[test]
    fn vtable_return_duration() {
        let (ret, out) = plugin_vtable_return(&TypeRef::Duration, false);
        assert_eq!(ret, "UInt64");
        assert!(out.is_empty());
    }

    #[test]
    fn vtable_return_string_json_channel() {
        let (ret, out) = plugin_vtable_return(&TypeRef::String, false);
        assert_eq!(ret, "Int32");
        assert_eq!(out, vec!["LibC::Char**", "LibC::Char**"]);
    }

    #[test]
    fn vtable_return_named_json_channel() {
        let (ret, out) = plugin_vtable_return(&TypeRef::Named("Foo".into()), false);
        assert_eq!(ret, "Int32");
        assert_eq!(out, vec!["LibC::Char**", "LibC::Char**"]);
    }

    #[test]
    fn vtable_return_bytes_json_channel() {
        let (ret, out) = plugin_vtable_return(&TypeRef::Bytes, false);
        assert_eq!(ret, "Int32");
        assert_eq!(out, vec!["LibC::Char**", "LibC::Char**"]);
    }

    #[test]
    fn vtable_return_optional_named_falls_back_to_out_result() {
        let (ret, out) = plugin_vtable_return(&TypeRef::Optional(Box::new(TypeRef::Named("Foo".into()))), false);
        assert_eq!(ret, "Int32");
        assert_eq!(out, vec!["LibC::Char**", "LibC::Char**"]);
    }

    // ── is_supported_plugin_bridge ─────────────────────────────────────

    fn plugin_bridge(trait_name: &str, register_fn: Option<&str>) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            register_fn: register_fn.map(|s| s.to_string()),
            unregister_fn: Some("unregister".to_string()),
            ..TraitBridgeConfig::default()
        }
    }

    fn plugin_api(trait_type: Option<TypeDef>) -> ApiSurface {
        let mut types = vec![];
        if let Some(t) = trait_type {
            types.push(t);
        }
        ApiSurface {
            crate_name: "test".into(),
            version: "0.1.0".into(),
            types,
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: std::collections::HashMap::new(),
            excluded_trait_names: std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    #[test]
    fn plugin_bridge_missing_register_fn_not_supported() {
        let bridge = plugin_bridge("Worker", None);
        let api = plugin_api(Some(TypeDef {
            name: "Worker".into(),
            is_trait: true,
            methods: vec![MethodDef {
                name: "do_work".into(),
                params: vec![ParamDef {
                    name: "input".into(),
                    ty: TypeRef::String,
                    ..ParamDef::default()
                }],
                return_type: TypeRef::String,
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        }));
        assert!(!is_supported_plugin_bridge(&api, &bridge));
    }

    #[test]
    fn plugin_bridge_missing_trait_not_supported() {
        let bridge = plugin_bridge("Worker", Some("register_worker"));
        let api = plugin_api(None);
        assert!(!is_supported_plugin_bridge(&api, &bridge));
    }

    #[test]
    fn plugin_bridge_with_unsupported_param_not_supported() {
        let bridge = plugin_bridge("Worker", Some("register_worker"));
        let api = plugin_api(Some(TypeDef {
            name: "Worker".into(),
            is_trait: true,
            methods: vec![MethodDef {
                name: "do_work".into(),
                params: vec![ParamDef {
                    name: "input".into(),
                    ty: TypeRef::Unit,
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Unit,
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        }));
        assert!(!is_supported_plugin_bridge(&api, &bridge));
    }

    #[test]
    fn plugin_bridge_with_all_supported_methods_is_supported() {
        let bridge = plugin_bridge("Worker", Some("register_worker"));
        let api = plugin_api(Some(TypeDef {
            name: "Worker".into(),
            is_trait: true,
            methods: vec![
                MethodDef {
                    name: "greet".into(),
                    params: vec![ParamDef {
                        name: "name".into(),
                        ty: TypeRef::String,
                        ..ParamDef::default()
                    }],
                    return_type: TypeRef::String,
                    ..MethodDef::default()
                },
                MethodDef {
                    name: "ping".into(),
                    params: vec![],
                    return_type: TypeRef::Unit,
                    ..MethodDef::default()
                },
            ],
            ..TypeDef::default()
        }));
        assert!(is_supported_plugin_bridge(&api, &bridge));
    }
}
