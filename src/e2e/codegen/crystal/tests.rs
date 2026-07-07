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
