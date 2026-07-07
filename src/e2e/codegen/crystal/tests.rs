#![cfg(test)]

use super::CrystalE2eCodegen;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::new_config::NewAlefConfig;
use crate::e2e::codegen::E2eCodegen;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureGroup};

fn e2e_config() -> E2eConfig {
    let toml = r#"
fixtures = "fixtures"
output = "e2e"

[call]
function = "convert"
module = "demo"
"#;
    toml::from_str(toml).expect("e2e config parses")
}

fn crate_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["crystal"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("crate config parses");
    cfg.resolve().expect("crate config resolves").remove(0)
}

fn fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: format!("fixture {id}"),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        source: String::new(),
        http: None,
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

fn file(files: &[crate::core::backend::GeneratedFile], suffix: &str) -> Option<String> {
    files
        .iter()
        .find(|f| f.path.display().to_string().ends_with(suffix))
        .map(|f| f.content.clone())
}

#[test]
fn generates_runnable_crystal_spec_project() {
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fixture("does_a_thing")],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .expect("generate");

    // shard.yml with a path dep on the generated binding.
    let shard = file(&files, "e2e/crystal/shard.yml").expect("shard.yml emitted");
    assert!(shard.contains("name: demo_e2e"), "shard: {shard}");
    assert!(shard.contains("path: ../../packages/crystal"), "shard dep: {shard}");

    // spec_helper requires spec + the binding.
    let helper = file(&files, "spec/spec_helper.cr").expect("spec_helper emitted");
    assert!(helper.contains("require \"spec\""), "helper: {helper}");
    assert!(helper.contains("require \"demo\""), "helper: {helper}");

    // smoke spec links the binding.
    let smoke = file(&files, "spec/binding_smoke_spec.cr").expect("smoke spec emitted");
    assert!(smoke.contains("describe Demo"), "smoke: {smoke}");
    assert!(smoke.contains("Demo::VERSION"), "smoke: {smoke}");

    // per-category spec references the fixture.
    let cat = file(&files, "spec/smoke_spec.cr").expect("category spec emitted");
    assert!(cat.contains("pending \"fixture does_a_thing\""), "category: {cat}");
}

#[test]
fn language_name_is_crystal() {
    assert_eq!(CrystalE2eCodegen.language_name(), "crystal");
}

#[test]
fn emit_test_backend_stub_subclasses_trait_and_registers() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeRef};

    let method = MethodDef {
        name: "fetch".to_string(),
        params: vec![ParamDef {
            name: "key".to_string(),
            ty: TypeRef::String,
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
        }],
        return_type: TypeRef::String,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let bridge = TraitBridgeConfig {
        trait_name: "Store".to_string(),
        super_trait: Some("Plugin".to_string()),
        register_fn: Some("register_store".to_string()),
        ..Default::default()
    };
    let fixture = Fixture {
        id: "my_fixture".to_string(),
        category: None,
        description: "test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        source: String::new(),
        http: None,
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    };
    let methods = vec![&method];
    let em = super::emit_test_backend(&bridge, &methods, &fixture);

    assert!(
        em.setup_block
            .contains("class TestStubMyFixture < __ALEF_MODULE__::Store"),
        "{}",
        em.setup_block
    );
    assert!(
        em.setup_block.contains("def name : String"),
        "super-trait name stub: {}",
        em.setup_block
    );
    assert!(
        em.setup_block.contains("def fetch(key : String) : String"),
        "method stub: {}",
        em.setup_block
    );
    assert!(
        em.arg_expr.contains("\"test\", stub_my_fixture"),
        "arg_expr: {}",
        em.arg_expr
    );
    assert!(
        em.teardown_block.contains("__ALEF_MODULE__.unregister_store(\"test\")"),
        "teardown: {}",
        em.teardown_block
    );
}

#[test]
fn fixture_with_assertions_emits_real_example() {
    use crate::e2e::fixture::Assertion;
    let mut fx = fixture("shouts");
    fx.input = serde_json::json!("hi");
    fx.assertions = vec![
        Assertion {
            assertion_type: "equals".to_string(),
            field: None,
            value: Some(serde_json::json!("HELLO")),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        },
        Assertion {
            assertion_type: "not_empty".to_string(),
            field: None,
            value: None,
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        },
    ];
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .expect("generate");
    let spec = file(&files, "spec/smoke_spec.cr").expect("category spec emitted");

    // Real example that calls the configured function with the input arg.
    assert!(spec.contains("it \"fixture shouts\" do"), "spec: {spec}");
    assert!(spec.contains("__result = Demo.convert(\"hi\")"), "call site: {spec}");
    assert!(spec.contains("__result.should eq(\"HELLO\")"), "equals assert: {spec}");
    assert!(
        spec.contains("__result.to_s.should_not be_empty"),
        "not_empty assert: {spec}"
    );
    assert!(
        !spec.contains("pending \"fixture shouts\""),
        "should not be pending: {spec}"
    );
}

fn assertion(kind: &str, field: Option<&str>, value: Option<serde_json::Value>) -> crate::e2e::fixture::Assertion {
    crate::e2e::fixture::Assertion {
        assertion_type: kind.to_string(),
        field: field.map(|s| s.to_string()),
        value,
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    }
}

#[test]
fn field_path_assertion_accesses_nested_getter() {
    let mut fx = fixture("has_name");
    fx.input = serde_json::json!("x");
    fx.assertions = vec![
        assertion("equals", Some("name"), Some(serde_json::json!("Ada"))),
        assertion("not_empty", Some("meta.title"), None),
    ];
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();
    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(
        spec.contains("__result.name.should eq(\"Ada\")"),
        "field equals: {spec}"
    );
    assert!(
        spec.contains("__result.meta.title.to_s.should_not be_empty"),
        "nested not_empty: {spec}"
    );
}

#[test]
fn error_assertion_wraps_call_in_expect_raises() {
    let mut fx = fixture("fails");
    fx.input = serde_json::json!("bad");
    fx.assertions = vec![assertion("error", None, None)];
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();
    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(spec.contains("expect_raises(Exception) do"), "error wrap: {spec}");
    assert!(spec.contains("Demo.convert(\"bad\")"), "error call: {spec}");
    assert!(
        !spec.contains("__result ="),
        "error path must not assign result: {spec}"
    );
}

fn assertion_values(kind: &str, field: Option<&str>, values: Vec<serde_json::Value>) -> crate::e2e::fixture::Assertion {
    crate::e2e::fixture::Assertion {
        assertion_type: kind.to_string(),
        field: field.map(|s| s.to_string()),
        value: None,
        values: Some(values),
        method: None,
        check: None,
        args: None,
        return_type: None,
    }
}

#[test]
fn contains_all_and_any_assertions() {
    let mut fx = fixture("membership");
    fx.input = serde_json::json!("x");
    fx.assertions = vec![
        assertion_values(
            "contains_all",
            None,
            vec![serde_json::json!("a"), serde_json::json!("b")],
        ),
        assertion_values(
            "contains_any",
            Some("tags"),
            vec![serde_json::json!("x"), serde_json::json!("y")],
        ),
    ];
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();
    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    // contains_all → one `contain` expectation per value.
    assert!(
        spec.contains("__result.should contain(\"a\")"),
        "contains_all a: {spec}"
    );
    assert!(
        spec.contains("__result.should contain(\"b\")"),
        "contains_all b: {spec}"
    );
    // contains_any → a single boolean any-of expectation on the field.
    assert!(
        spec.contains("(__result.tags.includes?(\"x\") || __result.tags.includes?(\"y\")).should be_true"),
        "contains_any: {spec}"
    );
}

#[test]
fn method_result_assertions() {
    use crate::e2e::fixture::Assertion;
    let mk = |method: &str, check: &str, value: Option<serde_json::Value>, args: Option<serde_json::Value>| Assertion {
        assertion_type: "method_result".to_string(),
        field: None,
        value,
        values: None,
        method: Some(method.to_string()),
        check: Some(check.to_string()),
        args,
        return_type: None,
    };
    let mut fx = fixture("methods");
    fx.input = serde_json::json!("x");
    fx.assertions = vec![
        mk("name", "equals", Some(serde_json::json!("Ada")), None),
        mk("valid?", "is_true", None, None),
        mk("score", "greater_than_or_equal", Some(serde_json::json!(10)), None),
        mk("tags", "count_min", Some(serde_json::json!(2)), None),
        mk(
            "at",
            "equals",
            Some(serde_json::json!("z")),
            Some(serde_json::json!([0])),
        ),
    ];
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();
    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(spec.contains("__result.name.should eq(\"Ada\")"), "equals: {spec}");
    assert!(spec.contains("__result.valid?.should be_true"), "is_true: {spec}");
    assert!(spec.contains("__result.score.should be >= 10"), "gte: {spec}");
    assert!(spec.contains("__result.tags.size.should be >= 2"), "count_min: {spec}");
    assert!(spec.contains("__result.at(0).should eq(\"z\")"), "method args: {spec}");
}

fn make_e2e_config_with_args(fixture_args: &str) -> E2eConfig {
    let toml = format!(
        r#"
fixtures = "fixtures"
output = "e2e"

[call]
function = "convert"
module = "demo"
{}
"#,
        fixture_args
    );
    toml::from_str(&toml).expect("e2e config parses")
}

#[test]
fn multi_arg_resolution_from_call_config() {
    let args_toml = r#"
[[call.args]]
name = "url"
field = "url"
type = "string"

[[call.args]]
name = "depth"
field = "depth"
type = "i32"
"#;
    let config = make_e2e_config_with_args(args_toml);
    let mut fx = fixture("scrape");
    fx.input = serde_json::json!({"url": "https://example.com", "depth": 3});
    fx.assertions = vec![assertion("equals", None, Some(serde_json::json!("ok")))];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &config, &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(
        spec.contains("Demo.convert(\"https://example.com\", 3)"),
        "multi-arg call: {spec}"
    );
    assert!(
        !spec.contains("__result = Demo.convert()"),
        "should not be empty args: {spec}"
    );
}

#[test]
fn json_object_arg_emits_dto_constructor() {
    let args_toml = r#"
[[call.args]]
name = "options"
field = "options"
type = "json_object"
element_type = "ConversionOptions"
"#;
    let config = make_e2e_config_with_args(args_toml);
    let mut fx = fixture("convert_with_opts");
    fx.input = serde_json::json!({"options": {"format": "markdown", "source": "html"}});
    fx.assertions = vec![assertion("not_empty", None, None)];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &config, &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    let escaped = "{\\\"format\\\":\\\"markdown\\\",\\\"source\\\":\\\"html\\\"}";
    assert!(
        spec.contains(&format!("Demo::ConversionOptions.from_json(\"{escaped}\")")),
        "dto constructor: {spec}"
    );
}

#[test]
fn test_backend_arg_wires_setup_register_and_teardown() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

    let args_toml = r#"
[[call.args]]
name = "backend"
field = "backend"
type = "test_backend"
trait = "Store"
"#;
    let config = make_e2e_config_with_args(args_toml);
    let mut fx = fixture("backend_test");
    fx.input = serde_json::json!({"backend": "stub"});
    fx.assertions = vec![assertion("equals", None, Some(serde_json::json!("done")))];

    let type_defs = vec![TypeDef {
        name: "Store".to_string(),
        methods: vec![MethodDef {
            name: "fetch".to_string(),
            params: vec![ParamDef {
                name: "key".to_string(),
                ty: TypeRef::String,
                ..ParamDef::default()
            }],
            return_type: TypeRef::String,
            receiver: Some(ReceiverKind::Ref),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    }];

    let mut crate_cfg = crate_config();
    crate_cfg.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "Store".to_string(),
        super_trait: Some("Plugin".to_string()),
        register_fn: Some("register_store".to_string()),
        ..TraitBridgeConfig::default()
    }];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];

    let files = CrystalE2eCodegen
        .generate(&groups, &config, &crate_cfg, &type_defs, &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();

    assert!(
        spec.contains("class TestStubBackendTest < Demo::Store"),
        "stub class: {spec}"
    );
    assert!(spec.contains("def name : String"), "super-trait name method: {spec}");
    assert!(
        spec.contains("Demo.register_store(\"test\", stub_backend_test)"),
        "register call: {spec}"
    );
    assert!(
        spec.contains("Demo.unregister_store(\"test\")"),
        "unregister call: {spec}"
    );
    assert!(spec.contains("__result = Demo.convert()"), "function call: {spec}");
    assert!(spec.contains("__result.should eq(\"done\")"), "assertion: {spec}");
}

#[test]
fn empty_args_in_e2e_config_falls_back_to_single_input_arg() {
    let config = e2e_config();
    let mut fx = fixture("plain");
    fx.input = serde_json::json!("hello world");
    fx.assertions = vec![assertion("not_empty", None, None)];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &config, &crate_config(), &[], &[])
        .unwrap();
    let spec = file(&files, "spec/smoke_spec.cr").unwrap();

    assert!(
        spec.contains("Demo.convert(\"hello world\")"),
        "fallback single arg: {spec}"
    );
}

#[test]
fn per_fixture_call_config_resolves_different_function() {
    let args_toml = r#"
[call]
function = "convert"
module = "demo"

[calls.custom_convert]
function = "custom_convert"
module = "demo"
"#;
    let toml = format!(
        r#"
fixtures = "fixtures"
output = "e2e"
{}
"#,
        args_toml
    );
    let config: E2eConfig = toml::from_str(&toml).expect("e2e config parses");

    let mut fx = fixture("custom");
    fx.call = Some("custom_convert".to_string());
    fx.input = serde_json::json!("data");
    fx.assertions = vec![assertion("equals", None, Some(serde_json::json!("ok")))];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &config, &crate_config(), &[], &[])
        .unwrap();
    let spec = file(&files, "spec/smoke_spec.cr").unwrap();

    assert!(
        spec.contains("Demo.custom_convert(\"data\")"),
        "custom function: {spec}"
    );
}

#[test]
fn per_fixture_args_override_call_config_args() {
    use crate::core::config::e2e::ArgMapping;

    let args_toml = r#"
[[call.args]]
name = "global"
field = "global"
type = "string"
"#;
    let config = make_e2e_config_with_args(args_toml);

    let mut fx = fixture("override");
    fx.args = vec![ArgMapping {
        name: "local".to_string(),
        field: "local".to_string(),
        arg_type: "string".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    fx.input = serde_json::json!({"local": "local_value"});
    fx.assertions = vec![assertion("equals", None, Some(serde_json::json!("ok")))];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &config, &crate_config(), &[], &[])
        .unwrap();
    let spec = file(&files, "spec/smoke_spec.cr").unwrap();

    assert!(
        spec.contains("Demo.convert(\"local_value\")"),
        "fixture arg override: {spec}"
    );
    assert!(!spec.contains("global"), "should not use global arg: {spec}");
}

#[test]
fn stub_bytes_return_produces_bytes_empty() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, TypeRef};
    let bridge = TraitBridgeConfig {
        trait_name: "Store".to_string(),
        register_fn: Some("register_store".to_string()),
        ..TraitBridgeConfig::default()
    };
    let method = MethodDef {
        name: "fetch_bytes".to_string(),
        return_type: TypeRef::Bytes,
        ..MethodDef::default()
    };
    let fixture = Fixture {
        id: "bytes_test".to_string(),
        category: None,
        description: "test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        source: String::new(),
        http: None,
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    };
    let methods = vec![&method];
    let em = super::emit_test_backend(&bridge, &methods, &fixture);

    assert!(
        em.setup_block.contains("def fetch_bytes : Bytes"),
        "signature: {}",
        em.setup_block
    );
    assert!(
        em.setup_block.contains("Bytes.empty"),
        "body should not be raise stub: {}",
        em.setup_block
    );
    assert!(
        !em.setup_block.contains("raise \"stub\""),
        "should not raise stub: {}",
        em.setup_block
    );
}

#[test]
fn stub_optional_named_param_type_is_nilable() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, ParamDef, TypeRef};
    let bridge = TraitBridgeConfig {
        trait_name: "Store".to_string(),
        register_fn: Some("register_store".to_string()),
        ..TraitBridgeConfig::default()
    };
    let method = MethodDef {
        name: "set_config".to_string(),
        params: vec![ParamDef {
            name: "config".to_string(),
            ty: TypeRef::Optional(Box::new(TypeRef::Named("MyConfig".to_string()))),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
        ..MethodDef::default()
    };
    let fixture = Fixture {
        id: "opt_config".to_string(),
        category: None,
        description: "test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        source: String::new(),
        http: None,
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    };
    let methods = vec![&method];
    let em = super::emit_test_backend(&bridge, &methods, &fixture);

    assert!(
        em.setup_block.contains("def set_config(config : MyConfig?) : Nil"),
        "optional named param: {}",
        em.setup_block
    );
    assert!(em.setup_block.contains("nil"), "default nil body: {}", em.setup_block);
}

#[test]
fn stub_other_complex_types_have_idiomatic_defaults() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, TypeRef};

    let bridge = TraitBridgeConfig {
        trait_name: "Worker".to_string(),
        register_fn: Some("register_worker".to_string()),
        ..TraitBridgeConfig::default()
    };

    let methods: Vec<MethodDef> = vec![
        ("process_bytes", TypeRef::Bytes),
        ("timeout", TypeRef::Duration),
        ("path", TypeRef::Path),
        ("ch", TypeRef::Char),
        (
            "mapping",
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        ),
    ]
    .into_iter()
    .map(|(name, return_type)| MethodDef {
        name: name.to_string(),
        return_type,
        ..MethodDef::default()
    })
    .collect();
    let method_refs: Vec<&MethodDef> = methods.iter().collect();

    let fixture = Fixture {
        id: "complex".to_string(),
        category: None,
        description: "test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        source: String::new(),
        http: None,
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    };
    let em = super::emit_test_backend(&bridge, &method_refs, &fixture);

    for (name, expected_type, expected_default) in [
        ("process_bytes", "Bytes", "Bytes.empty"),
        ("timeout", "Int64", "0"),
        ("path", "String", "\"\""),
        ("ch", "String", "\"\""),
        ("mapping", "Hash(String, String)", "{} of String => String"),
    ] {
        assert!(
            em.setup_block.contains(&format!("def {name} : {expected_type}")),
            "signature for {name}: {}",
            em.setup_block
        );
        assert!(
            em.setup_block.contains(expected_default),
            "default for {name} should be `{expected_default}`: {}",
            em.setup_block
        );
    }
}

#[test]
fn json_object_config_arg_uses_options_type_when_no_element_type() {
    let toml = r#"
fixtures = "fixtures"
output = "e2e"

[call]
function = "convert"
module = "demo"
options_type = "ConversionOptions"

[[call.args]]
name = "config"
field = "config"
type = "json_object"
"#;
    let config: E2eConfig = toml::from_str(toml).expect("e2e config parses");
    let mut fx = fixture("with_config");
    fx.input = serde_json::json!({"config": {"format": "md", "source": "html"}});
    fx.assertions = vec![assertion("not_empty", None, None)];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &config, &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(
        spec.contains("Demo::ConversionOptions.from_json("),
        "should use options_type for config arg: {spec}"
    );
}

#[test]
fn json_object_non_config_arg_prefers_element_type_over_options_type() {
    let toml = r#"
fixtures = "fixtures"
output = "e2e"

[call]
function = "convert"
module = "demo"
options_type = "GlobalOptions"

[[call.args]]
name = "render_opts"
field = "render_opts"
type = "json_object"
element_type = "RenderOptions"
"#;
    let config: E2eConfig = toml::from_str(toml).expect("e2e config parses");
    let mut fx = fixture("render");
    fx.input = serde_json::json!({"render_opts": {"width": 800}});
    fx.assertions = vec![assertion("not_empty", None, None)];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &config, &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(
        spec.contains("Demo::RenderOptions.from_json("),
        "element_type should win over options_type for non-config: {spec}"
    );
    assert!(
        !spec.contains("GlobalOptions"),
        "should not use global options_type: {spec}"
    );
}

#[test]
fn http_fixtures_emit_pending() {
    use crate::e2e::fixture::MockResponse;
    let mut fx = fixture("http_test");
    fx.mock_response = Some(MockResponse {
        status: 200,
        body: Some(serde_json::json!({"ok": true})),
        stream_chunks: None,
        headers: Default::default(),
    });
    fx.assertions = vec![assertion("equals", None, Some(serde_json::json!("data")))];

    let groups = vec![FixtureGroup {
        category: "http_tests".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/http_tests_spec.cr").unwrap();
    assert!(
        spec.contains("pending \"fixture http_test\""),
        "HTTP fixture should be pending: {spec}"
    );
    assert!(
        !spec.contains("Demo.convert"),
        "HTTP fixture should not call function: {spec}"
    );
    assert!(
        !spec.contains("__result ="),
        "HTTP fixture should not have result: {spec}"
    );
}

#[test]
fn http_handler_fixture_generates_real_http_spec() {
    use crate::e2e::fixture::{HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};
    use std::collections::BTreeMap;

    let mut fx = fixture("serve_test");
    fx.http = Some(HttpFixture {
        handler: HttpHandler {
            route: "/api/test".to_string(),
            method: "GET".to_string(),
            body_schema: None,
            parameters: BTreeMap::new(),
            middleware: None,
        },
        request: HttpRequest {
            method: "GET".to_string(),
            path: "/api/test".to_string(),
            headers: BTreeMap::new(),
            query_params: BTreeMap::new(),
            cookies: BTreeMap::new(),
            body: None,
            content_type: None,
            form_data: None,
        },
        expected_response: HttpExpectedResponse {
            status_code: 200,
            body: Some(serde_json::json!({"ok": true})),
            headers: BTreeMap::new(),
            validation_errors: None,
            body_partial: None,
        },
    });

    let groups = vec![FixtureGroup {
        category: "http_tests".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/http_tests_spec.cr").unwrap();
    assert!(
        !spec.contains("pending"),
        "HTTP handler fixture should NOT be pending: {spec}"
    );
    assert!(
        spec.contains("it \"fixture serve_test\""),
        "should emit an it block for the HTTP test: {spec}"
    );
    assert!(
        spec.contains("response.status_code.should eq(200)"),
        "should emit status assertion: {spec}"
    );
}

#[test]
fn result_var_configurable_name_replaces_hardcoded_result() {
    let toml = r#"
fixtures = "fixtures"
output = "e2e"

[call]
function = "convert"
module = "demo"
result_var = "my_result"
"#;
    let config: E2eConfig = toml::from_str(toml).expect("e2e config parses");
    let mut fx = fixture("renamed");
    fx.input = serde_json::json!("data");
    fx.assertions = vec![assertion("equals", None, Some(serde_json::json!("ok")))];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &config, &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(
        spec.contains("my_result = Demo.convert(\"data\")"),
        "should use custom result var: {spec}"
    );
    assert!(
        spec.contains("my_result.should eq(\"ok\")"),
        "assertion should use custom result var: {spec}"
    );
    assert!(!spec.contains("__result"), "should not use default result var: {spec}");
}

#[test]
fn returns_void_emits_call_without_result_assignment() {
    let toml = r#"
fixtures = "fixtures"
output = "e2e"

[call]
function = "shutdown"
module = "demo"
returns_void = true
"#;
    let config: E2eConfig = toml::from_str(toml).expect("e2e config parses");
    let mut fx = fixture("cleanup");
    fx.input = serde_json::Value::Null;
    fx.assertions = vec![assertion("error", None, None)];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &config, &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(
        spec.contains("Demo.shutdown()"),
        "void call should still be emitted: {spec}"
    );
    assert!(
        !spec.contains("__result ="),
        "void call should not assign result: {spec}"
    );
    assert!(
        spec.contains("expect_raises(Exception) do"),
        "error assertion still works for void calls: {spec}"
    );
}

#[test]
fn returns_void_with_non_error_assertions_still_works() {
    let toml = r#"
fixtures = "fixtures"
output = "e2e"

[call]
function = "validate"
module = "demo"
returns_void = true
"#;
    let config: E2eConfig = toml::from_str(toml).expect("e2e config parses");
    let mut fx = fixture("validate_test");
    fx.input = serde_json::json!("data");
    fx.assertions = vec![assertion("not_empty", None, None)];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &config, &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(
        spec.contains("Demo.validate(\"data\")"),
        "void call should emit: {spec}"
    );
    assert!(
        !spec.contains("__result ="),
        "void call should not assign result: {spec}"
    );
}

#[test]
fn http_fixture_with_headers_and_body_generates_full_spec() {
    use crate::e2e::fixture::{HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};
    use std::collections::BTreeMap;

    let mut headers = BTreeMap::new();
    headers.insert("X-Custom".to_string(), "abc".to_string());

    let mut fx = fixture("post_create");
    fx.http = Some(HttpFixture {
        handler: HttpHandler {
            route: "/items".to_string(),
            method: "POST".to_string(),
            body_schema: None,
            parameters: BTreeMap::new(),
            middleware: None,
        },
        request: HttpRequest {
            method: "POST".to_string(),
            path: "/items".to_string(),
            headers,
            query_params: BTreeMap::new(),
            cookies: BTreeMap::new(),
            body: Some(serde_json::json!({"name": "widget"})),
            content_type: Some("application/json".to_string()),
            form_data: None,
        },
        expected_response: HttpExpectedResponse {
            status_code: 201,
            body: Some(serde_json::json!({"id": 1, "name": "widget"})),
            headers: BTreeMap::new(),
            validation_errors: None,
            body_partial: None,
        },
    });

    let groups = vec![FixtureGroup {
        category: "api".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/api_spec.cr").unwrap();
    assert!(spec.contains("HTTP::Client.post("), "should use POST method: {spec}");
    assert!(
        spec.contains("status_code.should eq(201)"),
        "should assert 201 status: {spec}"
    );
    assert!(
        spec.contains("name"),
        "should include body JSON with name field: {spec}"
    );
    assert!(spec.contains("X-Custom"), "should include custom header: {spec}");
}

#[test]
fn http_fixture_with_partial_body_assertion() {
    use crate::e2e::fixture::{HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};
    use std::collections::BTreeMap;

    let mut fx = fixture("partial_test");
    fx.http = Some(HttpFixture {
        handler: HttpHandler {
            route: "/info".to_string(),
            method: "GET".to_string(),
            body_schema: None,
            parameters: BTreeMap::new(),
            middleware: None,
        },
        request: HttpRequest {
            method: "GET".to_string(),
            path: "/info".to_string(),
            headers: BTreeMap::new(),
            query_params: BTreeMap::new(),
            cookies: BTreeMap::new(),
            body: None,
            content_type: None,
            form_data: None,
        },
        expected_response: HttpExpectedResponse {
            status_code: 200,
            body: None,
            headers: BTreeMap::new(),
            validation_errors: None,
            body_partial: Some(serde_json::json!({"version": "1.0"})),
        },
    });

    let groups = vec![FixtureGroup {
        category: "api".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/api_spec.cr").unwrap();
    assert!(
        spec.contains("parsed = JSON.parse(response.body)"),
        "should parse body for partial: {spec}"
    );
    assert!(
        spec.contains("parsed[\"version\""),
        "should check partial field: {spec}"
    );
}

#[test]
fn http_fixture_with_validation_errors() {
    use crate::e2e::fixture::{
        HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest, ValidationErrorExpectation,
    };
    use std::collections::BTreeMap;

    let mut fx = fixture("validation_test");
    fx.http = Some(HttpFixture {
        handler: HttpHandler {
            route: "/submit".to_string(),
            method: "POST".to_string(),
            body_schema: None,
            parameters: BTreeMap::new(),
            middleware: None,
        },
        request: HttpRequest {
            method: "POST".to_string(),
            path: "/submit".to_string(),
            headers: BTreeMap::new(),
            query_params: BTreeMap::new(),
            cookies: BTreeMap::new(),
            body: Some(serde_json::json!({})),
            content_type: None,
            form_data: None,
        },
        expected_response: HttpExpectedResponse {
            status_code: 422,
            body: None,
            headers: BTreeMap::new(),
            validation_errors: Some(vec![ValidationErrorExpectation {
                loc: vec!["name".to_string()],
                msg: "field required".to_string(),
                error_type: "missing".to_string(),
            }]),
            body_partial: None,
        },
    });

    let groups = vec![FixtureGroup {
        category: "api".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/api_spec.cr").unwrap();
    assert!(spec.contains("status_code.should eq(422)"), "should assert 422: {spec}");
    assert!(
        spec.contains("errors = JSON.parse(response.body)[\"errors\"]"),
        "should extract errors array: {spec}"
    );
    assert!(
        spec.contains("\"field required\""),
        "should check error message: {spec}"
    );
}

#[test]
fn http_fixture_with_non_crystal_skip_still_generates_output() {
    use crate::e2e::fixture::{HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest, SkipDirective};
    use std::collections::BTreeMap;

    let mut fx = fixture("other_skip");
    fx.skip = Some(SkipDirective {
        languages: vec!["go".to_string()],
        reason: Some("not yet".to_string()),
    });
    fx.http = Some(HttpFixture {
        handler: HttpHandler {
            route: "/data".to_string(),
            method: "GET".to_string(),
            body_schema: None,
            parameters: BTreeMap::new(),
            middleware: None,
        },
        request: HttpRequest {
            method: "GET".to_string(),
            path: "/data".to_string(),
            headers: BTreeMap::new(),
            query_params: BTreeMap::new(),
            cookies: BTreeMap::new(),
            body: None,
            content_type: None,
            form_data: None,
        },
        expected_response: HttpExpectedResponse {
            status_code: 200,
            body: None,
            headers: BTreeMap::new(),
            validation_errors: None,
            body_partial: None,
        },
    });

    let groups = vec![FixtureGroup {
        category: "api".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/api_spec.cr").unwrap();
    assert!(
        spec.contains("status_code.should eq(200)"),
        "fixture with non-crystal skip should still generate: {spec}"
    );
    assert!(!spec.contains("pending"), "should not be pending: {spec}");
}

#[test]
fn http_fixture_with_header_assertions() {
    use crate::e2e::fixture::{HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};
    use std::collections::BTreeMap;

    let mut resp_headers = BTreeMap::new();
    resp_headers.insert("Content-Type".to_string(), "application/json".to_string());

    let mut fx = fixture("header_test");
    fx.http = Some(HttpFixture {
        handler: HttpHandler {
            route: "/data".to_string(),
            method: "GET".to_string(),
            body_schema: None,
            parameters: BTreeMap::new(),
            middleware: None,
        },
        request: HttpRequest {
            method: "GET".to_string(),
            path: "/data".to_string(),
            headers: BTreeMap::new(),
            query_params: BTreeMap::new(),
            cookies: BTreeMap::new(),
            body: None,
            content_type: None,
            form_data: None,
        },
        expected_response: HttpExpectedResponse {
            status_code: 200,
            body: None,
            headers: resp_headers,
            validation_errors: None,
            body_partial: None,
        },
    });

    let groups = vec![FixtureGroup {
        category: "api".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/api_spec.cr").unwrap();
    assert!(spec.contains("status_code.should eq(200)"), "should assert 200: {spec}");
}

#[test]
fn http_fixture_with_uuid_header_token() {
    use crate::e2e::fixture::{HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};
    use std::collections::BTreeMap;

    let mut resp_headers = BTreeMap::new();
    resp_headers.insert("X-Request-Id".to_string(), "<<uuid>>".to_string());

    let mut fx = fixture("uuid_test");
    fx.http = Some(HttpFixture {
        handler: HttpHandler {
            route: "/track".to_string(),
            method: "GET".to_string(),
            body_schema: None,
            parameters: BTreeMap::new(),
            middleware: None,
        },
        request: HttpRequest {
            method: "GET".to_string(),
            path: "/track".to_string(),
            headers: BTreeMap::new(),
            query_params: BTreeMap::new(),
            cookies: BTreeMap::new(),
            body: None,
            content_type: None,
            form_data: None,
        },
        expected_response: HttpExpectedResponse {
            status_code: 200,
            body: None,
            headers: resp_headers,
            validation_errors: None,
            body_partial: None,
        },
    });

    let groups = vec![FixtureGroup {
        category: "api".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/api_spec.cr").unwrap();
    assert!(
        spec.contains("match(/\\A[0-9a-f]{8}"),
        "should use UUID regex assertion: {spec}"
    );
    assert!(spec.contains("x-request-id"), "should include header name: {spec}");
}

#[test]
fn is_empty_assertion() {
    let mut fx = fixture("empty_check");
    fx.input = serde_json::json!("x");
    fx.assertions = vec![assertion("is_empty", Some("items"), None)];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(
        spec.contains("__result.items.to_s.should be_empty"),
        "is_empty should emit be_empty: {spec}"
    );
}

#[test]
fn starts_with_assertion() {
    let mut fx = fixture("prefix_check");
    fx.input = serde_json::json!("x");
    fx.assertions = vec![assertion("starts_with", Some("path"), Some(serde_json::json!("/api")))];
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(
        spec.contains("should start_with("),
        "starts_with should emit start_with: {spec}"
    );
}

#[test]
fn ends_with_assertion() {
    let mut fx = fixture("suffix_check");
    fx.input = serde_json::json!("x");
    fx.assertions = vec![assertion("ends_with", Some("url"), Some(serde_json::json!(".html")))];
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(
        spec.contains("should end_with("),
        "ends_with should emit end_with: {spec}"
    );
}

#[test]
fn matches_regex_assertion() {
    let mut fx = fixture("regex_check");
    fx.input = serde_json::json!("x");
    fx.assertions = vec![assertion(
        "matches_regex",
        Some("email"),
        Some(serde_json::json!("^[a-z]+@")),
    )];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(
        spec.contains("should match("),
        "matches_regex should emit match: {spec}"
    );
}

#[test]
fn greater_than_assertion() {
    let mut fx = fixture("gt_check");
    fx.input = serde_json::json!("x");
    fx.assertions = vec![assertion("greater_than", Some("count"), Some(serde_json::json!(0)))];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(spec.contains("should be >"), "greater_than should emit be >: {spec}");
}

#[test]
fn less_than_assertion() {
    let mut fx = fixture("lt_check");
    fx.input = serde_json::json!("x");
    fx.assertions = vec![assertion("less_than", Some("price"), Some(serde_json::json!(100)))];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(spec.contains("should be <"), "less_than should emit be <: {spec}");
}

#[test]
fn min_length_assertion() {
    let mut fx = fixture("min_len");
    fx.input = serde_json::json!("x");
    fx.assertions = vec![assertion("min_length", Some("name"), Some(serde_json::json!(3)))];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(
        spec.contains("size.should be >="),
        "min_length should emit size >=: {spec}"
    );
}

#[test]
fn count_equals_assertion() {
    let mut fx = fixture("count_eq");
    fx.input = serde_json::json!("x");
    fx.assertions = vec![assertion("count_equals", Some("tags"), Some(serde_json::json!(5)))];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(
        spec.contains("size.should eq("),
        "count_equals should emit size eq: {spec}"
    );
}

#[test]
fn is_true_standalone_assertion() {
    let mut fx = fixture("truthy");
    fx.input = serde_json::json!("x");
    fx.assertions = vec![assertion("is_true", Some("valid"), None)];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(spec.contains("should be_true"), "is_true should emit be_true: {spec}");
}

#[test]
fn is_false_standalone_assertion() {
    let mut fx = fixture("falsey");
    fx.input = serde_json::json!("x");
    fx.assertions = vec![assertion("is_false", Some("active"), None)];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(
        spec.contains("should be_false"),
        "is_false should emit be_false: {spec}"
    );
}

#[test]
fn not_contains_assertion() {
    let mut fx = fixture("exclude");
    fx.input = serde_json::json!("x");
    fx.assertions = vec![assertion("not_contains", Some("name"), Some(serde_json::json!("bad")))];
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(
        spec.contains("should_not contain("),
        "not_contains should emit should_not contain: {spec}"
    );
}

#[test]
fn count_min_standalone_assertion() {
    let mut fx = fixture("min_items");
    fx.input = serde_json::json!("x");
    fx.assertions = vec![assertion("count_min", Some("items"), Some(serde_json::json!(3)))];

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/smoke_spec.cr").unwrap();
    assert!(
        spec.contains("size.should be >="),
        "count_min should emit size >=: {spec}"
    );
}

#[test]
fn http_post_without_explicit_content_type_defaults_to_json() {
    use crate::e2e::fixture::{HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};
    use std::collections::BTreeMap;

    let mut fx = fixture("post_no_ct");
    fx.http = Some(HttpFixture {
        handler: HttpHandler {
            route: "/submit".to_string(),
            method: "POST".to_string(),
            body_schema: None,
            parameters: BTreeMap::new(),
            middleware: None,
        },
        request: HttpRequest {
            method: "POST".to_string(),
            path: "/submit".to_string(),
            headers: BTreeMap::new(),
            query_params: BTreeMap::new(),
            cookies: BTreeMap::new(),
            body: Some(serde_json::json!({"x": 1})),
            content_type: None,
            form_data: None,
        },
        expected_response: HttpExpectedResponse {
            status_code: 201,
            body: None,
            headers: BTreeMap::new(),
            validation_errors: None,
            body_partial: None,
        },
    });

    let groups = vec![FixtureGroup {
        category: "api".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/api_spec.cr").unwrap();
    assert!(
        spec.contains("Content-Type"),
        "should emit Content-Type header even when not explicit: {spec}"
    );
}

#[test]
fn http_fixture_with_query_params_emits_query_string() {
    use crate::e2e::fixture::{HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};
    use std::collections::BTreeMap;

    let mut qp = BTreeMap::new();
    qp.insert("page".to_string(), serde_json::json!(1));
    qp.insert("limit".to_string(), serde_json::json!(20));

    let mut fx = fixture("query_test");
    fx.http = Some(HttpFixture {
        handler: HttpHandler {
            route: "/items".to_string(),
            method: "GET".to_string(),
            body_schema: None,
            parameters: BTreeMap::new(),
            middleware: None,
        },
        request: HttpRequest {
            method: "GET".to_string(),
            path: "/items".to_string(),
            headers: BTreeMap::new(),
            query_params: qp,
            cookies: BTreeMap::new(),
            body: None,
            content_type: None,
            form_data: None,
        },
        expected_response: HttpExpectedResponse {
            status_code: 200,
            body: None,
            headers: BTreeMap::new(),
            validation_errors: None,
            body_partial: None,
        },
    });

    let groups = vec![FixtureGroup {
        category: "api".to_string(),
        fixtures: vec![fx],
    }];
    let files = CrystalE2eCodegen
        .generate(&groups, &e2e_config(), &crate_config(), &[], &[])
        .unwrap();

    let spec = file(&files, "spec/api_spec.cr").unwrap();
    assert!(
        spec.contains("page") && spec.contains("limit"),
        "should include query params in URL: {spec}"
    );
}
