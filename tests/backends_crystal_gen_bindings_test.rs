//! Targeted assertions for the Crystal backend, mirroring
//! `backends_gleam_gen_bindings_test.rs` / `backends_zig_gen_bindings_test.rs`.

use alef::backends::crystal::CrystalBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{ApiSurface, FunctionDef, ParamDef, PrimitiveType, TypeRef};

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

fn make_fn(name: &str, params: Vec<ParamDef>, return_type: TypeRef, error_type: Option<&str>) -> FunctionDef {
    FunctionDef {
        name: name.into(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        params,
        return_type,
        is_async: false,
        error_type: error_type.map(|s| s.to_string()),
        doc: String::new(),
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

fn api_with(functions: Vec<FunctionDef>) -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions,
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn make_config() -> ResolvedCrateConfig {
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
fn emits_source_file_and_shard_yml() {
    let api = api_with(vec![make_fn("noop", vec![], TypeRef::Unit, None)]);
    let files = CrystalBackend.generate_bindings(&api, &make_config()).unwrap();
    assert_eq!(files.len(), 2, "expected main .cr + shard.yml, got {files:#?}");

    let main = files.iter().find(|f| f.path.display().to_string().ends_with(".cr"))
        .expect("expected a .cr binding file");
    assert!(main.generated_header, "binding file should carry the generated header");

    let shard = files.iter().find(|f| f.path.display().to_string().ends_with("shard.yml"))
        .expect("expected a shard.yml");
    assert!(!shard.generated_header, "shard.yml should NOT carry the generated header");
    assert!(shard.content.contains("name: demo"), "shard.yml should contain the package name");
}

#[test]
fn lib_block_binds_prefixed_c_symbols() {
    let api = api_with(vec![make_fn(
        "add",
        vec![
            make_param("a", TypeRef::Primitive(PrimitiveType::I32)),
            make_param("b", TypeRef::Primitive(PrimitiveType::I32)),
        ],
        TypeRef::Primitive(PrimitiveType::I32),
        None,
    )]);
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    assert!(content.contains("lib LibDemo"), "missing lib block: {content}");
    assert!(
        content.contains("fun free_string = demo_free_string"),
        "missing free_string: {content}"
    );
    // Scalars pass by value; the exported C symbol is prefixed with the FFI prefix.
    assert!(
        content.contains("fun add = demo_add(a : Int32, b : Int32) : Int32"),
        "missing add fun: {content}"
    );
}

#[test]
fn module_uses_ruby_style_snake_case_methods() {
    let api = api_with(vec![make_fn(
        "parseDocument",
        vec![make_param("rawInput", TypeRef::String)],
        TypeRef::String,
        Some("DemoError"),
    )]);
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    assert!(content.contains("module Demo"), "missing module: {content}");
    // Ruby-style naming: snake_case method + snake_case params.
    assert!(
        content.contains("def self.parse_document(raw_input : String) : String"),
        "missing snake_case method: {content}"
    );
}

#[test]
fn complex_params_are_json_encoded_scalars_pass_through() {
    let api = api_with(vec![make_fn(
        "mix",
        vec![
            make_param("text", TypeRef::String),
            make_param("n", TypeRef::Primitive(PrimitiveType::U32)),
        ],
        TypeRef::Unit,
        None,
    )]);
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    // String arg passes raw (the FFI expects raw char*); the scalar passes through.
    assert!(
        content.contains("LibDemo.mix(text, n)"),
        "string param should pass raw, scalar through: {content}"
    );
}

#[test]
fn fallible_non_nilable_return_raises_on_null() {
    let api = api_with(vec![make_fn(
        "convert",
        vec![make_param("input", TypeRef::String)],
        TypeRef::String,
        Some("E"),
    )]);
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    assert!(
        content.contains("raise \"LibDemo.convert returned a null pointer\""),
        "non-nilable fallible return should raise on null, not return nil: {content}"
    );
    assert!(
        !content.contains("return nil"),
        "non-nilable return must not `return nil`: {content}"
    );
}

#[test]
fn optional_return_is_nilable_and_returns_nil_on_null() {
    let api = api_with(vec![make_fn(
        "maybe",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        None,
    )]);
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    assert!(
        content.contains("def self.maybe() : String?"),
        "optional return should be nilable: {content}"
    );
    assert!(
        content.contains("return nil if __ptr.null?"),
        "nilable return should map null to nil: {content}"
    );
}

#[test]
fn json_return_parses_result() {
    let api = api_with(vec![make_fn("describe", vec![], TypeRef::Json, None)]);
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    assert!(
        content.contains("def self.describe() : JSON::Any"),
        "missing JSON return type: {content}"
    );
    assert!(
        content.contains("JSON.parse(__json)"),
        "JSON return should be parsed: {content}"
    );
}

#[test]
fn binding_excluded_functions_are_skipped() {
    let mut f = make_fn("hidden", vec![], TypeRef::Unit, None);
    f.binding_excluded = true;
    let visible = make_fn("shown", vec![], TypeRef::Unit, None);
    let content = &CrystalBackend
        .generate_bindings(&api_with(vec![f, visible]), &make_config())
        .unwrap()[0]
        .content;

    assert!(
        !content.contains("demo_hidden"),
        "excluded fn should not be bound: {content}"
    );
    assert!(content.contains("demo_shown"), "visible fn should be bound: {content}");
}

#[test]
fn configured_trait_bridge_fails_loudly_instead_of_mis_generating() {
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
register_fn = "register_renderer"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    let config = cfg.resolve().expect("test config must resolve").remove(0);

    let api = api_with(vec![make_fn("noop", vec![], TypeRef::Unit, None)]);
    let err = CrystalBackend
        .generate_bindings(&api, &config)
        .expect_err("configured plugin-style trait bridge should be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("not plugin-style/registry bridges"),
        "unexpected error: {msg}"
    );
    assert!(
        msg.contains("Renderer"),
        "error should name the configured trait: {msg}"
    );
}

#[test]
fn supported_plugin_bridge_generates_plugin_file() {
    use alef::core::config::TraitBridgeConfig;
    use alef::core::ir::{MethodDef, TypeDef};

    let renderer = TypeDef {
        name: "Renderer".into(),
        is_trait: true,
        methods: vec![MethodDef {
            name: "render".into(),
            params: vec![make_param("text", TypeRef::String)],
            return_type: TypeRef::String,
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };

    let mut config = make_config();
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "Renderer".to_string(),
        register_fn: Some("register_renderer".to_string()),
        unregister_fn: Some("unregister_renderer".to_string()),
        ..TraitBridgeConfig::default()
    }];

    let mut api = api_with(vec![make_fn("noop", vec![], TypeRef::Unit, None)]);
    api.types = vec![renderer];

    let files = CrystalBackend
        .generate_bindings(&api, &config)
        .expect("supported plugin bridge should generate successfully");

    let plugin_filename = files
        .iter()
        .map(|f| f.path.display().to_string())
        .find(|p| p.contains("plugin.cr"))
        .expect("plugin.cr file should be emitted for supported plugin bridge");

    assert!(
        plugin_filename.contains("demo_renderer_plugin.cr"),
        "expected plugin file, got: {plugin_filename}"
    );

    let plugin_content = files
        .iter()
        .find(|f| f.path.display().to_string().contains("plugin.cr"))
        .map(|f| &f.content)
        .expect("plugin content");

    assert!(
        plugin_content.contains("struct RendererVTable"),
        "should contain vtable struct: {plugin_content}"
    );
    assert!(
        plugin_content.contains("def self.register"),
        "should contain register method: {plugin_content}"
    );
    assert!(
        plugin_content.contains("fun register_renderer = demo_register_renderer"),
        "should contain register fun: {plugin_content}"
    );

    let main_content = files
        .iter()
        .find(|f| f.path.display().to_string().ends_with("src/demo.cr"))
        .map(|f| &f.content)
        .expect("main binding file");

    assert!(
        main_content.contains("require \"./demo_renderer_plugin\""),
        "main file should require plugin bridge file: {main_content}"
    );
}

#[test]
fn plugin_bridge_with_unsupported_method_still_rejected() {
    use alef::core::config::TraitBridgeConfig;
    use alef::core::ir::{MethodDef, TypeDef};

    let renderer = TypeDef {
        name: "Renderer".into(),
        is_trait: true,
        methods: vec![MethodDef {
            name: "render".into(),
            params: vec![make_param("text", TypeRef::Unit)], // Unit params unsupported
            return_type: TypeRef::Unit,
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };

    let mut config = make_config();
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "Renderer".to_string(),
        register_fn: Some("register_renderer".to_string()),
        ..TraitBridgeConfig::default()
    }];

    let mut api = api_with(vec![make_fn("noop", vec![], TypeRef::Unit, None)]);
    api.types = vec![renderer];

    let err = CrystalBackend
        .generate_bindings(&api, &config)
        .expect_err("unsupported plugin bridge should be rejected");
    assert!(
        err.to_string().contains("Renderer"),
        "error should name the trait: {err}"
    );
}

#[test]
fn visitor_bridge_with_data_carrying_result_enum_is_rejected() {
    use alef::core::ir::{EnumDef, EnumVariant, MethodDef, ReceiverKind, TypeDef};

    // A visitor result enum with a non-`String` payload variant (`Custom(i32)`) is
    // not representable in the visitor callback protocol (only unit + `String`
    // payloads are) — generation must fail loudly rather than emit a broken binding.
    let renderer = TypeDef {
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
            doc: String::new(),
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
        doc: String::new(),
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
    };
    let ctx = TypeDef {
        name: "SyntaxContext".into(),
        rust_path: "demo::SyntaxContext".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
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
    };
    let data_variant = EnumVariant {
        name: "Custom".into(),
        fields: vec![make_field_string()],
        doc: String::new(),
        is_default: false,
        serde_rename: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: true,
        originally_had_data_fields: true,
        cfg: None,
        version: Default::default(),
    };
    let decision = EnumDef {
        name: "WalkDecision".into(),
        rust_path: "demo::WalkDecision".into(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Continue".into(),
                fields: vec![],
                doc: String::new(),
                is_default: true,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            data_variant,
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        has_default: true,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };
    let mut api = api_with(vec![]);
    api.types = vec![renderer, ctx];
    api.enums = vec![decision];

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
    let config = cfg.resolve().expect("test config must resolve").remove(0);

    let err = CrystalBackend
        .generate_bindings(&api, &config)
        .expect_err("data-carrying result enum should be rejected");
    assert!(
        err.to_string().contains("data-carrying result enums"),
        "unexpected error: {err}"
    );
}

fn make_field_string() -> alef::core::ir::FieldDef {
    alef::core::ir::FieldDef {
        name: "0".into(),
        ty: TypeRef::Primitive(PrimitiveType::I32),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: alef::core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

#[test]
fn duration_params_and_returns_pass_as_u64_scalar() {
    let api = api_with(vec![make_fn(
        "delay",
        vec![make_param("ms", TypeRef::Duration)],
        TypeRef::Duration,
        None,
    )]);
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    assert!(content.contains("lib LibDemo"), "missing lib block: {content}");
    assert!(
        content.contains("fun delay = demo_delay(ms : UInt64) : UInt64"),
        "Duration param and return must use UInt64 in lib fun, got: {content}"
    );
    assert!(
        content.contains("def self.delay(ms : Int64) : Int64"),
        "wrapper must use Int64 for Duration, got: {content}"
    );
    assert!(
        content.contains("LibDemo.delay(ms)"),
        "Duration param must pass directly (not JSON-encoded), got: {content}"
    );
}

#[test]
fn duration_with_error_still_returns_c_string() {
    let api = api_with(vec![make_fn(
        "risky_delay",
        vec![make_param("ms", TypeRef::Duration)],
        TypeRef::Duration,
        Some("E"),
    )]);
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    assert!(
        content.contains("fun risky_delay = demo_risky_delay(ms : UInt64) : UInt64"),
        "fallible scalar return is by-value + last_error_code, got: {content}"
    );
    assert!(
        content.contains("last_error_code"),
        "fallible scalar return must check last_error_code: {content}"
    );
}

#[test]
fn bytes_passes_as_raw_string_not_json() {
    let api = api_with(vec![make_fn(
        "get_data",
        vec![make_param("payload", TypeRef::Bytes)],
        TypeRef::Bytes,
        None,
    )]);
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    assert!(content.contains("lib LibDemo"), "missing lib block: {content}");
    assert!(
        content.contains("fun get_data = demo_get_data(payload : LibC::Char*) : LibC::Char*"),
        "Bytes param/return must use LibC::Char* in lib fun: {content}"
    );
    assert!(
        content.contains("def self.get_data(payload : Bytes) : Bytes"),
        "wrapper must use Bytes for Bytes, got: {content}"
    );
    assert!(
        content.contains("LibDemo.get_data(payload.to_a.to_json)"),
        "Bytes param must be JSON-serialized via to_a.to_json, got: {content}"
    );
    assert!(
        content.contains("Array(UInt8).from_json(__json)"),
        "Bytes return must parse JSON via Array(UInt8), got: {content}"
    );
    assert!(
        content.contains("Bytes.new(__arr.size)"),
        "Bytes return must copy via Bytes.new, got: {content}"
    );
}

#[test]
fn bytes_fallible_returns_raw_c_string() {
    let api = api_with(vec![make_fn(
        "fetch_bytes",
        vec![make_param("key", TypeRef::String)],
        TypeRef::Bytes,
        Some("E"),
    )]);
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    assert!(
        content.contains("fun fetch_bytes = demo_fetch_bytes(key : LibC::Char*) : LibC::Char*"),
        "fallible Bytes return still uses LibC::Char*: {content}"
    );
}

// ── F12: struct-return code path in gen_call_body ────────────────────────

fn make_type(name: &str, fields: Vec<alef::core::ir::FieldDef>, is_opaque: bool) -> alef::core::ir::TypeDef {
    alef::core::ir::TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
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
    }
}

fn make_field(name: &str, ty: TypeRef) -> alef::core::ir::FieldDef {
    alef::core::ir::FieldDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: alef::core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn api_with_types(functions: Vec<FunctionDef>, types: Vec<alef::core::ir::TypeDef>) -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types,
        functions,
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

#[test]
fn struct_return_uses_to_json_free_pattern() {
    let config_ty = make_type("Config", vec![make_field("value", TypeRef::Primitive(PrimitiveType::I32))], false);
    let api = api_with_types(
        vec![make_fn(
            "build",
            vec![make_param("cfg", TypeRef::Named("Config".to_string()))],
            TypeRef::Named("Config".to_string()),
            Some("DemoError"),
        )],
        vec![config_ty],
    );
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    // The lib fun should use Config* (struct pointer) not LibC::Char*
    assert!(
        content.contains("fun build = demo_build(cfg : Config*) : Config*"),
        "struct param/return should use Config* in lib fun, got: {content}"
    );

    // The wrapper should contain to_json/free for the return path
    assert!(
        content.contains("__json_ptr = LibDemo.config_to_json(__ptr)"),
        "return should use config_to_json helper: {content}"
    );
    assert!(
        content.contains("LibDemo.config_free(__ptr)"),
        "return should free the struct pointer: {content}"
    );
    assert!(
        content.contains("LibDemo.free_string(__json_ptr)"),
        "return should free the JSON string: {content}"
    );
    assert!(
        content.contains("Config.from_json(__json)"),
        "return should parse JSON into Config: {content}"
    );

    // The wrapper param setup should use from_json/free
    assert!(
        content.contains("__handle_cfg = LibDemo.config_from_json(cfg.to_json)"),
        "struct param should use from_json helper: {content}"
    );
    assert!(
        content.contains("LibDemo.config_free(__handle_cfg)"),
        "struct param should free after call: {content}"
    );
}

#[test]
fn struct_return_without_error_still_uses_to_json() {
    let config_ty = make_type("Config", vec![make_field("value", TypeRef::Primitive(PrimitiveType::I32))], false);
    let api = api_with_types(
        vec![make_fn(
            "build",
            vec![make_param("cfg", TypeRef::Named("Config".to_string()))],
            TypeRef::Named("Config".to_string()),
            None,
        )],
        vec![config_ty],
    );
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    // Infallible struct return: the lib fun returns Config* directly (no error C string)
    assert!(
        content.contains("fun build = demo_build(cfg : Config*) : Config*"),
        "infallible struct return should use Config* in lib fun: {content}"
    );

    // Should still use to_json/free pattern
    assert!(
        content.contains("__json_ptr = LibDemo.config_to_json"),
        "infallible return should still use config_to_json: {content}"
    );
}

#[test]
fn struct_param_only_no_struct_return_uses_from_json() {
    let config_ty = make_type("Config", vec![make_field("value", TypeRef::Primitive(PrimitiveType::I32))], false);
    let api = api_with_types(
        vec![make_fn(
            "apply",
            vec![make_param("cfg", TypeRef::Named("Config".to_string()))],
            TypeRef::String,
            None,
        )],
        vec![config_ty],
    );
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    // The lib fun: struct param is Config*, return is String (via LibC::Char*)
    assert!(
        content.contains("fun apply = demo_apply(cfg : Config*) : LibC::Char*"),
        "struct param with string return: {content}"
    );

    // Param setup uses from_json/free
    assert!(
        content.contains("__handle_cfg = LibDemo.config_from_json(cfg.to_json)"),
        "struct param should use from_json: {content}"
    );
    assert!(
        content.contains("LibDemo.config_free(__handle_cfg)"),
        "struct param should free: {content}"
    );

    // The lib call should pass the handle, not cfg directly
    assert!(
        content.contains("LibDemo.apply(__handle_cfg)"),
        "call should pass handle variable: {content}"
    );
}

// ── F13: struct-param code path in gen_wrapper_body ──────────────────────

#[test]
fn struct_param_uses_from_json_in_wrapper_body() {
    let config_ty = make_type("Config", vec![make_field("value", TypeRef::Primitive(PrimitiveType::I32))], false);
    let api = api_with_types(
        vec![make_fn(
            "process",
            vec![make_param("config", TypeRef::Named("Config".to_string()))],
            TypeRef::Unit,
            Some("E"),
        )],
        vec![config_ty],
    );
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    assert!(
        content.contains("fun process = demo_process(config : Config*) : Void"),
        "struct param with fallible unit return (by-value + last_error_code): {content}"
    );

    // Verify from_json/free setup
    assert!(
        content.contains("__handle_config = LibDemo.config_from_json(config.to_json)"),
        "struct param from_json: {content}"
    );
    assert!(
        content.contains("LibDemo.config_free(__handle_config)"),
        "struct param free: {content}"
    );
}

#[test]
fn multiple_struct_params_each_get_from_json() {
    let config_ty = make_type("Config", vec![make_field("value", TypeRef::Primitive(PrimitiveType::I32))], false);
    let opts_ty = make_type("Options", vec![make_field("verbose", TypeRef::Primitive(PrimitiveType::Bool))], false);
    let api = api_with_types(
        vec![make_fn(
            "run",
            vec![
                make_param("cfg", TypeRef::Named("Config".to_string())),
                make_param("opts", TypeRef::Named("Options".to_string())),
            ],
            TypeRef::Unit,
            None,
        )],
        vec![config_ty, opts_ty],
    );
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    // lib fun uses both struct pointers
    assert!(
        content.contains("fun run = demo_run(cfg : Config*, opts : Options*)"),
        "multiple struct params: {content}"
    );

    // Each param gets its own handle
    assert!(
        content.contains("__handle_cfg = LibDemo.config_from_json(cfg.to_json)"),
        "first struct param from_json: {content}"
    );
    assert!(
        content.contains("__handle_opts = LibDemo.options_from_json(opts.to_json)"),
        "second struct param from_json: {content}"
    );
    assert!(
        content.contains("LibDemo.config_free(__handle_cfg)"),
        "first struct param free: {content}"
    );
    assert!(
        content.contains("LibDemo.options_free(__handle_opts)"),
        "second struct param free: {content}"
    );

    // The lib call passes both handles
    assert!(
        content.contains("LibDemo.run(__handle_cfg, __handle_opts)"),
        "call passes both handles: {content}"
    );
}

#[test]
fn optional_named_param_still_uses_json_string_abi() {
    let config_ty = make_type("Config", vec![make_field("value", TypeRef::Primitive(PrimitiveType::I32))], false);
    let api = api_with_types(
        vec![make_fn(
            "build",
            vec![make_param("cfg", TypeRef::Optional(Box::new(TypeRef::Named("Config".to_string()))))],
            TypeRef::Named("Config".to_string()),
            Some("E"),
        )],
        vec![config_ty],
    );
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    // Optional(Named(...)) is NOT matched by the direct TypeRef::Named pattern,
    // so it should fall through to JSON-string ABI for the param: cfg.to_json
    // is passed directly, not via a __handle_cfg from_json/free pattern.
    assert!(
        !content.contains("__handle_cfg = LibDemo.config_from_json"),
        "Optional(Config) param should NOT use __handle_cfg from_json (F3): {content}"
    );
    assert!(
        content.contains("LibDemo.build(cfg.to_json)"),
        "Optional(Config) param should remain JSON-encoded: {content}"
  );
}

// ── F3: Optional(Named(...)) struct return ──────────────────────────────

#[test]
fn fallible_optional_struct_return_uses_json_string_abi() {
    // Fallible + Optional(Named) cannot use struct-pointer ABI because
    // null could mean error or None — indistinguishable. Must use JSON-string ABI.
    let config_ty = make_type("Config", vec![make_field("value", TypeRef::Primitive(PrimitiveType::I32))], false);
    let api = api_with_types(
        vec![make_fn(
            "find",
            vec![make_param("key", TypeRef::String)],
            TypeRef::Optional(Box::new(TypeRef::Named("Config".to_string()))),
            Some("E"),
        )],
        vec![config_ty],
    );
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    // Fallible optional struct return uses LibC::Char* (JSON-string ABI)
    assert!(
        content.contains("fun find = demo_find(key : LibC::Char*) : LibC::Char*"),
        "fallible optional struct return should use LibC::Char* in lib fun: {content}"
    );
    // config_to_json/config_free ARE declared as helper fun in the lib block,
    // but the wrapper method body must NOT use the struct-pointer ABI pattern.
    // Verify the method uses JSON-string ABI: free_string, not config_free.
    assert!(
        content.contains("LibDemo.free_string(__ptr)"),
        "fallible optional struct return should use free_string (JSON ABI): {content}"
    );
    assert!(
        !content.contains("LibDemo.find(__handle"),
        "fallible optional struct return should NOT use __handle pattern: {content}"
    );
}

#[test]
fn infallible_optional_struct_return_uses_struct_pointer_abi() {
    // Infallible + Optional(Named) CAN use struct-pointer ABI because
    // null only means None (no error ambiguity).
    let config_ty = make_type("Config", vec![make_field("value", TypeRef::Primitive(PrimitiveType::I32))], false);
    let api = api_with_types(
        vec![make_fn(
            "find",
            vec![make_param("key", TypeRef::String)],
            TypeRef::Optional(Box::new(TypeRef::Named("Config".to_string()))),
            None,
        )],
        vec![config_ty],
    );
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    // Infallible optional struct return: lib fun uses Config* (nullable struct pointer)
    assert!(
        content.contains("fun find = demo_find(key : LibC::Char*) : Config*"),
        "infallible optional struct return should use Config*: {content}"
    );

    // Should return nil on null, to_json/free for non-null
    assert!(
        content.contains("return nil if __ptr.null?"),
        "should return nil on null: {content}"
    );
    assert!(
        content.contains("__json_ptr = LibDemo.config_to_json(__ptr)"),
        "should use config_to_json: {content}"
    );
    assert!(
        content.contains("LibDemo.config_free(__ptr)"),
        "should free the struct pointer: {content}"
    );
    assert!(
        content.contains("Config.from_json(__json)"),
        "should parse Config from JSON: {content}"
    );
}

// ── F5: duplicate getter lines ──────────────────────────────────────────

#[test]
fn struct_getter_appears_once_per_field() {
    let config_ty = make_type("Config", vec![make_field("value", TypeRef::Primitive(PrimitiveType::I32))], false);
    let api = api_with_types(
        vec![make_fn("noop", vec![], TypeRef::Unit, None)],
        vec![config_ty],
    );
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    // Each getter should appear exactly once (no duplicates).
    let getter_value_count = content.matches("getter value : Int32").count();
    assert_eq!(
        getter_value_count, 1,
        "getter value : Int32 should appear exactly once, not {getter_value_count}: {content}"
    );
}

#[test]
fn struct_multiple_fields_each_getter_appears_once() {
    let config_ty = make_type(
        "Config",
        vec![
            make_field("value", TypeRef::Primitive(PrimitiveType::I32)),
            make_field("label", TypeRef::String),
            make_field("tag", TypeRef::Optional(Box::new(TypeRef::String))),
        ],
        false,
    );
    let api = api_with_types(
        vec![make_fn("noop", vec![], TypeRef::Unit, None)],
        vec![config_ty],
    );
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    assert_eq!(
        content.matches("getter value : Int32").count(),
        1,
        "getter value appears once: {content}"
    );
    assert_eq!(
        content.matches("getter label : String").count(),
        1,
        "getter label appears once: {content}"
    );
    assert_eq!(
        content.matches("getter tag : String?").count(),
        1,
        "getter tag appears once: {content}"
    );
}

// ── F4: default values for partial JSON ────────────────────────────────

use alef::core::ir::DefaultValue;

fn make_field_with_default(name: &str, ty: TypeRef, typed_default: Option<DefaultValue>) -> alef::core::ir::FieldDef {
    let mut f = make_field(name, ty);
    f.typed_default = typed_default;
    f
}

#[test]
fn struct_fields_with_defaults_emit_json_field_default_annotation() {
    let config_ty = make_type(
        "Config",
        vec![
            make_field_with_default("retries", TypeRef::Primitive(PrimitiveType::U32), Some(DefaultValue::IntLiteral(3))),
            make_field_with_default("verbose", TypeRef::Primitive(PrimitiveType::Bool), Some(DefaultValue::BoolLiteral(false))),
            make_field_with_default("name", TypeRef::String, Some(DefaultValue::StringLiteral("default".into()))),
            make_field_with_default("rate", TypeRef::Primitive(PrimitiveType::F64), Some(DefaultValue::FloatLiteral(1.5))),
            make_field("required_field", TypeRef::String), // no default
        ],
        false,
    );
    let api = api_with_types(
        vec![make_fn("test", vec![], TypeRef::Unit, None)],
        vec![config_ty],
    );
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    // Getter-initializer defaults (the working mechanism for partial JSON in
    // Crystal 1.19 — `@[JSON::Field(default:)]` does not work with from_json).
    assert!(content.contains("getter retries : UInt32 = 3"), "int default: {content}");
    assert!(content.contains("getter verbose : Bool = false"), "bool default: {content}");
    assert!(content.contains(r#"getter name : String = "default""#), "string default: {content}");
    assert!(content.contains("getter rate : Float64 = 1.5"), "float default: {content}");
    // Field without an explicit default still gets a type-based fallback
    // (`String` → `""`) so partial JSON never leaves a non-nilable field unset.
    assert!(content.contains("getter required_field : String = \"\""), "no-default field: {content}");
}

#[test]
fn struct_field_without_default_does_not_emit_annotation() {
    let config_ty = make_type(
        "Config",
        vec![make_field("value", TypeRef::Primitive(PrimitiveType::I32))],
        false,
    );
    let api = api_with_types(
        vec![make_fn("test", vec![], TypeRef::Unit, None)],
        vec![config_ty],
    );
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    assert!(
        !content.contains("@[JSON::Field(default:"),
        "field without default should not emit annotation: {content}"
    );
}

#[test]
fn optional_field_with_default_does_not_emit_annotation() {
    // Optional fields are already nilable (nil means absent), so no default needed.
    let mut f = make_field("tag", TypeRef::Optional(Box::new(TypeRef::String)));
    f.typed_default = Some(DefaultValue::None);
    let config_ty = make_type("Config", vec![f], false);
    let api = api_with_types(
        vec![make_fn("test", vec![], TypeRef::Unit, None)],
        vec![config_ty],
    );
    let content = &CrystalBackend.generate_bindings(&api, &make_config()).unwrap()[0].content;

    assert!(
        !content.contains("@[JSON::Field(default:"),
        "optional field with None default should not emit annotation: {content}"
    );
}
