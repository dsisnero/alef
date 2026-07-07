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
