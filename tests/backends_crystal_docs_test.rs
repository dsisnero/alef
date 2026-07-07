//! Doc-comment emission for the Crystal backend, mirroring
//! `backends_gleam_docs_test.rs`.
//!
//! Verifies that IR doc strings surface as Crystal `#` comments in the
//! generated binding.

use alef::backends::crystal::CrystalBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{ApiSurface, FunctionDef, TypeRef};

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

fn api_with_doc(doc: &str) -> ApiSurface {
    ApiSurface {
        crate_name: "demo".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "test_func".to_string(),
            rust_path: "demo::test_func".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
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
        }],
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
fn function_doc_appears_in_binding() {
    let api = api_with_doc("This is a test function.");
    let files = CrystalBackend.generate_bindings(&api, &make_config()).unwrap();
    assert!(!files.is_empty());
    let content = &files[0].content;
    assert!(
        content.contains("# This is a test function."),
        "Crystal doc comment should appear in output: {content}"
    );
}

#[test]
fn only_first_doc_line_is_emitted_as_summary() {
    let api = api_with_doc("Summary line.\nSecond paragraph that should not appear inline.");
    let files = CrystalBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(content.contains("# Summary line."), "summary line missing: {content}");
    assert!(
        !content.contains("Second paragraph"),
        "only the summary line should be emitted inline: {content}"
    );
}
