//! Snapshot coverage for the Crystal backend, mirroring
//! `backends_gleam_snapshot_test.rs` / `backends_zig_snapshot_test.rs`.
//!
//! Captures the full generated `.cr` binding and `shard.yml` scaffold for a
//! representative API surface (struct + function + named-type function + enum +
//! error) so any change in emitted Crystal shows up as a reviewable diff.

use alef::backends::crystal::CrystalBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef,
    PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};

fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn make_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
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
        core_wrapper: alef::core::ir::CoreWrapper::None,
    }
}

fn make_fn(
    name: &str,
    params: Vec<ParamDef>,
    return_type: TypeRef,
    error_type: Option<&str>,
    doc: &str,
) -> FunctionDef {
    FunctionDef {
        name: name.into(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        params,
        return_type,
        is_async: false,
        error_type: error_type.map(|s| s.to_string()),
        doc: doc.to_string(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_basic_api() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "demo::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("value", TypeRef::Primitive(PrimitiveType::I32), false),
                make_field("label", TypeRef::String, false),
                make_field("tag", TypeRef::Optional(Box::new(TypeRef::String)), true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "A demo configuration struct.".to_string(),
            cfg: None,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![
            // String-in / string-out, fallible (JSON-string ABI).
            make_fn(
                "process",
                vec![
                    make_param("input", TypeRef::String),
                    make_param("count", TypeRef::Primitive(PrimitiveType::U32)),
                ],
                TypeRef::String,
                Some("DemoError"),
                "Process input and return a result.",
            ),
            // Named-type param and return — exercises `.to_json` / `Config.from_json`.
            make_fn(
                "build",
                vec![make_param("config", TypeRef::Named("Config".to_string()))],
                TypeRef::Named("Config".to_string()),
                Some("DemoError"),
                "Build a config from another config.",
            ),
            // Infallible scalar path.
            make_fn(
                "add",
                vec![
                    make_param("a", TypeRef::Primitive(PrimitiveType::I32)),
                    make_param("b", TypeRef::Primitive(PrimitiveType::I32)),
                ],
                TypeRef::Primitive(PrimitiveType::I32),
                None,
                "Add two integers.",
            ),
        ],
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "demo::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![],
                    doc: "Active state.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Inactive".to_string(),
                    fields: vec![],
                    doc: "Inactive state.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: "Processing status.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![ErrorDef {
            name: "DemoError".to_string(),
            rust_path: "demo::DemoError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "InvalidInput".to_string(),
                    message_template: Some("invalid input provided".to_string()),
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                    doc: "Input validation failed.".to_string(),
                },
                ErrorVariant {
                    name: "ProcessingFailed".to_string(),
                    message_template: Some("processing failed".to_string()),
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                    doc: "Processing encountered an error.".to_string(),
                },
            ],
            doc: "Errors emitted by demo operations.".to_string(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn make_basic_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["crystal"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

#[test]
fn snapshot_basic_bindings() {
    let api = make_basic_api();
    let config = make_basic_config();
    let files = CrystalBackend.generate_bindings(&api, &config).unwrap();
    for file in &files {
        insta::assert_snapshot!(
            format!("snapshot_basic__{}", file.path.display().to_string().replace('/', "__")),
            &file.content
        );
    }
}

#[test]
fn snapshot_basic_scaffold() {
    let api = make_basic_api();
    let config = make_basic_config();
    let files = CrystalBackend.generate_scaffold(&api, &config).unwrap();
    for file in &files {
        insta::assert_snapshot!(
            format!("snapshot_basic__{}", file.path.display().to_string().replace('/', "__")),
            &file.content
        );
    }
}

/// A visitor-style trait bridge surface: `Renderer` trait (context + result enum)
/// plus a config that bridges it — regression-locks the generated visitor.cr.
fn make_visitor_api() -> ApiSurface {
    let unit_variant = |name: &str, is_default: bool| EnumVariant {
        name: name.to_string(),
        fields: vec![],
        doc: String::new(),
        is_default,
        serde_rename: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: false,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    };
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![
            TypeDef {
                name: "Renderer".into(),
                rust_path: "demo::Renderer".into(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![MethodDef {
                    name: "visit_text".into(),
                    params: vec![
                        make_param("ctx", TypeRef::Named("SyntaxContext".into())),
                        make_param("text", TypeRef::String),
                    ],
                    return_type: TypeRef::Named("WalkDecision".into()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Visit a text node.".into(),
                    receiver: Some(ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: true,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                }],
                is_opaque: false,
                is_clone: false,
                is_copy: false,
                doc: "A syntax-tree renderer.".into(),
                cfg: None,
                is_trait: true,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
                version: Default::default(),
            },
            TypeDef {
                name: "SyntaxContext".into(),
                rust_path: "demo::SyntaxContext".into(),
                original_rust_path: String::new(),
                fields: vec![
                    make_field("depth", TypeRef::Primitive(PrimitiveType::I32), false),
                    make_field("tag_name", TypeRef::String, false),
                ],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: "Context for a visit callback.".into(),
                cfg: None,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![EnumDef {
            name: "WalkDecision".into(),
            rust_path: "demo::WalkDecision".into(),
            original_rust_path: String::new(),
            variants: vec![
                unit_variant("Continue", true),
                unit_variant("SkipChildren", false),
                unit_variant("Stop", false),
            ],
            methods: vec![],
            doc: "What to do after visiting a node.".into(),
            cfg: None,
            is_copy: true,
            has_serde: false,
            has_default: true,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn make_visitor_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["crystal"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[[crates.trait_bridges]]
trait_name = "Renderer"
bind_via = "options_field"
options_type = "ParseOptions"
options_field = "renderer"
context_type = "SyntaxContext"
result_type = "WalkDecision"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

#[test]
fn snapshot_visitor_bridge() {
    let api = make_visitor_api();
    let config = make_visitor_config();
    let files = CrystalBackend.generate_bindings(&api, &config).unwrap();
    // Snapshot only the generated visitor bridge file (the main binding is covered
    // by `snapshot_basic_bindings`).
    let visitor = files
        .iter()
        .find(|f| f.path.display().to_string().ends_with("_renderer_visitor.cr"))
        .expect("expected a generated visitor.cr");
    insta::assert_snapshot!(
        format!("visitor__{}", visitor.path.display().to_string().replace('/', "__")),
        &visitor.content
    );
}
