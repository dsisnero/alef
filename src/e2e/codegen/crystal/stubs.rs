//! Crystal e2e trait-bridge (plugin) test-backend stub emission.
//!
//! Produces a Crystal class that subclasses the generated abstract trait class
//! and registers it via the backend's `register_<trait>` API, mirroring
//! `zig/stubs.rs`. The binding module is referenced through the
//! [`MODULE_PLACEHOLDER`] token which the spec generator substitutes for the
//! real module name (the same pattern Zig uses for its `lib.` placeholder).

use crate::e2e::codegen::TestBackendEmission;
use heck::{ToPascalCase, ToSnakeCase};
use std::fmt::Write as _;

/// Placeholder for the generated binding module, substituted by the spec
/// generator once it knows the crate's module name.
pub const MODULE_PLACEHOLDER: &str = "__ALEF_MODULE__";

/// Default Crystal literal for a stubbed method return type.
fn default_value(ty: &crate::core::ir::TypeRef) -> String {
    use crate::codegen::type_mapper::TypeMapper;
    use crate::core::ir::{PrimitiveType, TypeRef};
    let mapper = crate::backends::crystal::type_map::CrystalMapper;
    match ty {
        TypeRef::Unit => "nil".to_string(),
        TypeRef::String | TypeRef::Char | TypeRef::Path => "\"\"".to_string(),
        TypeRef::Json => "JSON::Any.new(nil)".to_string(),
        TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
        TypeRef::Primitive(PrimitiveType::F32 | PrimitiveType::F64) => "0.0".to_string(),
        TypeRef::Primitive(_) | TypeRef::Duration => "0".to_string(),
        TypeRef::Optional(_) => "nil".to_string(),
        TypeRef::Vec(_) => "[] of typeof(nil)".to_string(),
        TypeRef::Bytes => "Bytes.empty".to_string(),
        TypeRef::Map(k, v) => format!("{{}} of {} => {}", mapper.map_type(k), mapper.map_type(v)),
        _ => format!("raise \"stub\" /* {:?} */", ty),
    }
}

/// Emit a Crystal test-backend stub for a plugin trait bridge fixture.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> TestBackendEmission {
    use crate::codegen::type_mapper::TypeMapper;
    let mapper = crate::backends::crystal::type_map::CrystalMapper;

    let id_snake = crate::e2e::escape::sanitize_ident(&fixture.id.to_snake_case());
    let stub_class = format!("TestStub{}", id_snake.to_pascal_case());
    let stub_var = format!("stub_{id_snake}");
    let trait_name = trait_bridge.trait_name.to_pascal_case();
    let trait_snake = trait_bridge.trait_name.to_snake_case();
    let m = MODULE_PLACEHOLDER;

    let mut setup = String::new();
    let _ = writeln!(setup, "class {stub_class} < {m}::{trait_name}");
    if trait_bridge.super_trait.is_some() {
        let _ = writeln!(setup, "  def name : String");
        let _ = writeln!(setup, "    \"test\"");
        let _ = writeln!(setup, "  end");
        let _ = writeln!(setup, "  def version : String");
        let _ = writeln!(setup, "    \"0.0.1\"");
        let _ = writeln!(setup, "  end");
    }
    for method in methods {
        // Skip super-trait methods (handled above via name/version).
        if trait_bridge
            .super_trait
            .as_deref()
            .is_some_and(|st| method.trait_source.as_deref() == Some(st))
        {
            continue;
        }
        let method_snake = method.name.to_snake_case();
        let params = method
            .params
            .iter()
            .map(|p| format!("{} : {}", p.name.to_snake_case(), mapper.map_type(&p.ty)))
            .collect::<Vec<_>>()
            .join(", ");
        let ret = if matches!(method.return_type, crate::core::ir::TypeRef::Unit) {
            "Nil".to_string()
        } else {
            mapper.map_type(&method.return_type)
        };
        let sig = if params.is_empty() {
            format!("  def {method_snake} : {ret}")
        } else {
            format!("  def {method_snake}({params}) : {ret}")
        };
        let _ = writeln!(setup, "{sig}");
        let _ = writeln!(setup, "    {}", default_value(&method.return_type));
        let _ = writeln!(setup, "  end");
    }
    let _ = writeln!(setup, "end");
    let _ = writeln!(setup, "{stub_var} = {stub_class}.new");

    // arg_expr expands into `{module}.{register_fn}(<arg_expr>)`.
    let arg_expr = format!("\"test\", {stub_var}");
    let teardown = format!("{m}.unregister_{trait_snake}(\"test\")\n");

    TestBackendEmission {
        setup_block: setup,
        arg_expr,
        type_imports: Vec::new(),
        teardown_block: teardown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{PrimitiveType, TypeRef};

    #[test]
    fn default_unit_returns_nil() {
        assert_eq!(default_value(&TypeRef::Unit), "nil");
    }

    #[test]
    fn default_string_returns_empty_quote() {
        assert_eq!(default_value(&TypeRef::String), "\"\"");
    }

    #[test]
    fn default_char_returns_empty_quote() {
        assert_eq!(default_value(&TypeRef::Char), "\"\"");
    }

    #[test]
    fn default_path_returns_empty_quote() {
        assert_eq!(default_value(&TypeRef::Path), "\"\"");
    }

    #[test]
    fn default_json_returns_any_new_nil() {
        assert_eq!(default_value(&TypeRef::Json), "JSON::Any.new(nil)");
    }

    #[test]
    fn default_bool_returns_false() {
        assert_eq!(default_value(&TypeRef::Primitive(PrimitiveType::Bool)), "false");
    }

    #[test]
    fn default_f32_returns_zero_point_zero() {
        assert_eq!(default_value(&TypeRef::Primitive(PrimitiveType::F32)), "0.0");
    }

    #[test]
    fn default_f64_returns_zero_point_zero() {
        assert_eq!(default_value(&TypeRef::Primitive(PrimitiveType::F64)), "0.0");
    }

    #[test]
    fn default_i32_returns_zero() {
        assert_eq!(default_value(&TypeRef::Primitive(PrimitiveType::I32)), "0");
    }

    #[test]
    fn default_duration_returns_zero() {
        assert_eq!(default_value(&TypeRef::Duration), "0");
    }

    #[test]
    fn default_optional_returns_nil() {
        assert_eq!(default_value(&TypeRef::Optional(Box::new(TypeRef::String))), "nil");
    }

    #[test]
    fn default_vec_returns_empty_array() {
        assert_eq!(
            default_value(&TypeRef::Vec(Box::new(TypeRef::String))),
            "[] of typeof(nil)"
        );
    }

    #[test]
    fn default_bytes_returns_bytes_empty() {
        assert_eq!(default_value(&TypeRef::Bytes), "Bytes.empty");
    }

    #[test]
    fn default_map_returns_empty_hash() {
        let map = TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Primitive(PrimitiveType::I32)),
        );
        assert_eq!(default_value(&map), "{} of String => Int32");
    }
}
