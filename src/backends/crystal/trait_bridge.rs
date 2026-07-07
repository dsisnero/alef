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

use super::type_map::crystal_type_name;

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
fn context_field(field_ty: &TypeRef, raw_field: &str, api: &ApiSurface) -> Option<(&'static str, String, String)> {
    Some(match field_ty {
        TypeRef::String => ("LibC::Char*", "String".into(), format!("String.new(raw.{raw_field})")),
        TypeRef::Primitive(PrimitiveType::Bool) => ("Int32", "Bool".into(), format!("raw.{raw_field} != 0")),
        TypeRef::Primitive(p) => {
            let (c, hi) = crystal_scalar(p);
            (c, hi.into(), format!("raw.{raw_field}"))
        }
        TypeRef::Named(name) if api.enums.iter().any(|e| e.name == *name) => {
            let en = crystal_type_name(name);
            ("Int32", en.clone(), format!("{en}.from_value(raw.{raw_field})"))
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
            (TypeRef::String, false) => ("LibC::Char*", "String".into(), format!("String.new({raw})")),
            (TypeRef::String, true) => (
                "LibC::Char*",
                "String?".into(),
                format!("{raw}.null? ? nil : String.new({raw})"),
            ),
            (TypeRef::Primitive(PrimitiveType::Bool), false) => ("Int32", "Bool".into(), format!("{raw} != 0")),
            (TypeRef::Primitive(PrimitiveType::U32), false) => ("UInt32", "UInt32".into(), raw.clone()),
            (TypeRef::Primitive(PrimitiveType::Usize), false) => ("LibC::SizeT", "LibC::SizeT".into(), raw.clone()),
            _ => return None, // unsupported param shape → skip whole method
        };
        params.push(CbParam {
            name: raw,
            c_type,
            hi_type,
            decode,
        });
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
        "  fun {trait_snake}_options_set_visitor = {ffi_prefix}_options_set_visitor_handle(options : Void*, handle : {trait_name}VisitorHandle) : Void\n"
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
        out.push_str(&format!("      visitor = Box({trait_name}Visitor).unbox(user_data)\n"));
        out.push_str("      raw = ctx.value\n");
        // Build high-level context.
        let ctx_args = ctx_fields
            .iter()
            .map(|f| f.decode.clone())
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("      context = {ctx_hi}.new({ctx_args})\n"));
        // Decode extras.
        let mut call_args = String::from("context");
        for p in &cb.params {
            out.push_str(&format!("      {}_value = {}\n", p.name, p.decode));
            call_args.push_str(&format!(", {}_value", p.name));
        }
        out.push_str(&format!("      decision = visitor.{}({call_args})\n", cb.method));
        // Map the decision to its FFI result code; string-payload variants also
        // write a malloc'd copy of the payload into `out_custom` (Rust takes
        // ownership via `CString::from_raw`, so the allocator must be libc malloc).
        out.push_str("      case decision\n");
        for v in &result_meta.unit_variants {
            out.push_str(&format!(
                "      when {result_hi}::{} then {}\n",
                crystal_type_name(&v.name),
                v.code
            ));
        }
        for v in &result_meta.string_payload_variants {
            out.push_str(&format!("      when {result_hi}::{}\n", crystal_type_name(&v.name)));
            out.push_str("        __payload = decision.value.to_slice\n");
            out.push_str("        __buf = LibC.malloc(__payload.size + 1).as(UInt8*)\n");
            out.push_str("        __buf.copy_from(__payload.to_unsafe, __payload.size)\n");
            out.push_str("        __buf[__payload.size] = 0_u8\n");
            out.push_str("        out_custom.value = __buf\n");
            out.push_str("        out_len.value = LibC::SizeT.new(__payload.size)\n");
            out.push_str(&format!("        {}\n", v.code));
        }
        out.push_str(&format!("      else {default_code}\n"));
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
