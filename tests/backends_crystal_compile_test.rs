//! End-to-end check that the Crystal backend emits source the Crystal compiler
//! accepts. Generates bindings from an in-memory `ApiSurface`, writes them to a
//! temp file, and runs `crystal build --no-codegen` (semantic analysis without
//! linking) to type-check the output.
//!
//! The test is skipped gracefully when the `crystal` compiler is not on PATH,
//! so it never fails CI on machines without a Crystal toolchain.

use alef::backends::crystal::CrystalBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef,
    PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};
use std::process::Command;

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
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        params,
        return_type,
        is_async: false,
        error_type: error_type.map(|s| s.to_string()),
        doc: format!("Demo function {name}."),
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

fn make_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.into(),
        ty,
        optional: false,
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

/// A self-contained surface with structs, a unit enum, an error type, and functions
/// that use them — so the generated module is fully self-consistent and every
/// `from_json` / `to_json` path is exercised by the compiler.
fn rich_api() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Config".into(),
            rust_path: "demo::Config".into(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("value", TypeRef::Primitive(PrimitiveType::I32)),
                make_field("label", TypeRef::String),
                FieldDef {
                    optional: true,
                    ..make_field("tag", TypeRef::Optional(Box::new(TypeRef::String)))
                },
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "A demo configuration struct.".into(),
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
        }],
        functions: vec![
            make_fn(
                "add",
                vec![
                    make_param("a", TypeRef::Primitive(PrimitiveType::I32)),
                    make_param("b", TypeRef::Primitive(PrimitiveType::I32)),
                ],
                TypeRef::Primitive(PrimitiveType::I32),
                None,
            ),
            make_fn(
                "convert",
                vec![make_param("input", TypeRef::String)],
                TypeRef::String,
                Some("DemoError"),
            ),
            // DTO round-trip: Config in, Config out — exercises Config.from_json + .to_json.
            make_fn(
                "build",
                vec![make_param("config", TypeRef::Named("Config".into()))],
                TypeRef::Named("Config".into()),
                Some("DemoError"),
            ),
            make_fn("reset", vec![], TypeRef::Unit, None),
        ],
        enums: vec![EnumDef {
            name: "Status".into(),
            rust_path: "demo::Status".into(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Active".into(),
                    fields: vec![],
                    doc: "Active state.".into(),
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
                    name: "Inactive".into(),
                    fields: vec![],
                    doc: "Inactive state.".into(),
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
            doc: "Processing status.".into(),
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
            name: "DemoError".into(),
            rust_path: "demo::DemoError".into(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "InvalidInput".into(),
                    message_template: Some("invalid input".into()),
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                    doc: "Input validation failed.".into(),
                },
                ErrorVariant {
                    name: "ProcessingFailed".into(),
                    message_template: Some("processing failed".into()),
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                    doc: "Processing encountered an error.".into(),
                },
            ],
            doc: "Errors from demo operations.".into(),
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

#[test]
fn crystal_bindings_emit_lib_and_module() {
    let files = CrystalBackend.generate_bindings(&rich_api(), &make_config()).unwrap();
    assert_eq!(files.len(), 1, "expected a single .cr source file");
    let content = &files[0].content;

    // Low-level C-ABI binding.
    assert!(content.contains("lib LibDemo"), "missing lib block: {content}");
    assert!(
        content.contains("fun free_string = demo_free_string"),
        "missing free_string: {content}"
    );
    assert!(
        content.contains("fun add = demo_add(a : Int32, b : Int32) : Int32"),
        "missing add fun: {content}"
    );
    // High-level Ruby-style module with snake_case methods.
    assert!(content.contains("module Demo"), "missing module: {content}");
    assert!(
        content.contains("def self.add(a : Int32, b : Int32) : Int32"),
        "missing add method: {content}"
    );
    assert!(
        content.contains("def self.convert(input : String) : String"),
        "missing convert method: {content}"
    );
    // Emitted type definitions.
    assert!(content.contains("class Config"), "missing struct: {content}");
    assert!(
        content.contains("include JSON::Serializable"),
        "struct should be JSON-serializable: {content}"
    );
    assert!(
        content.contains("getter value : Int32"),
        "missing struct field: {content}"
    );
    assert!(
        content.contains("getter tag : String?"),
        "optional field should be nilable: {content}"
    );
    assert!(content.contains("enum Status"), "missing enum: {content}");
    assert!(
        content.contains("class DemoError < Exception"),
        "missing error class: {content}"
    );
}

#[test]
fn crystal_bindings_typecheck_with_compiler() {
    // Skip gracefully when the Crystal toolchain is unavailable.
    if Command::new("crystal").arg("--version").output().is_err() {
        eprintln!("skipping: `crystal` compiler not found on PATH");
        return;
    }

    let files = CrystalBackend.generate_bindings(&rich_api(), &make_config()).unwrap();
    let content = &files[0].content;

    // Crystal only type-checks a method body when the method is instantiated
    // (called). Append top-level call sites so `crystal build --no-codegen`
    // enforces each body against its declared return type — including the
    // `Config.from_json` / `.to_json` round-trip in `build`. The `LibDemo`
    // externs are only referenced (not linked) under `--no-codegen`.
    let harness = format!(
        "{content}\n\n\
         Demo.add(1_i32, 2_i32)\n\
         Demo.convert(\"x\")\n\
         Demo.reset\n\
         cfg = Demo::Config.from_json(%({{\"value\":1,\"label\":\"a\"}}))\n\
         Demo.build(cfg)\n\
         status = Demo::Status::Active\n"
    );

    let dir = std::env::temp_dir().join(format!("alef_crystal_compile_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("demo.cr");
    std::fs::write(&src, &harness).expect("write generated source");

    // `--no-codegen` performs full semantic analysis (parsing + type checking)
    // without emitting an object file or linking the (absent) native library.
    let output = Command::new("crystal")
        .arg("build")
        .arg("--no-codegen")
        .arg(&src)
        .output()
        .expect("run crystal build");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "generated Crystal failed to type-check:\n--- source ---\n{harness}\n--- crystal stderr ---\n{stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

fn make_ctx_param(name: &str, ty: TypeRef) -> ParamDef {
    make_param(name, ty)
}

/// A trait-bridge surface: a `Renderer` visitor trait (context + result), plus a
/// config that bridges it. Exercises the visitor-callback host-side codegen.
fn visitor_bridge_api() -> ApiSurface {
    let renderer_trait = TypeDef {
        name: "Renderer".into(),
        rust_path: "demo::Renderer".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![MethodDef {
            name: "visit_text".into(),
            params: vec![
                make_ctx_param("ctx", TypeRef::Named("SyntaxContext".into())),
                make_ctx_param("text", TypeRef::String),
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
    };
    let context = TypeDef {
        name: "SyntaxContext".into(),
        rust_path: "demo::SyntaxContext".into(),
        original_rust_path: String::new(),
        fields: vec![
            make_field("depth", TypeRef::Primitive(PrimitiveType::I32)),
            make_field("tag_name", TypeRef::String),
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
    };
    let unit_variant = |name: &str, is_default: bool| EnumVariant {
        name: name.into(),
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
    let decision = EnumDef {
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
    };
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![renderer_trait, context],
        functions: vec![],
        enums: vec![decision],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn visitor_bridge_config() -> ResolvedCrateConfig {
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
fn visitor_bridge_emits_callback_layer() {
    let files = CrystalBackend
        .generate_bindings(&visitor_bridge_api(), &visitor_bridge_config())
        .unwrap();
    let visitor = files
        .iter()
        .find(|f| f.path.display().to_string().ends_with("_renderer_visitor.cr"))
        .expect("expected a generated visitor.cr");
    let c = &visitor.content;
    // lib layer: callbacks struct + create/free/options_set with the FFI symbols.
    assert!(
        c.contains("struct RendererVisitorCallbacks"),
        "missing callbacks struct: {c}"
    );
    assert!(
        c.contains("user_data : Void*"),
        "callbacks must lead with user_data: {c}"
    );
    assert!(
        c.contains("fun renderer_visitor_create = demo_visitor_create"),
        "missing visitor_create symbol: {c}"
    );
    assert!(
        c.contains("= demo_options_set_visitor_handle"),
        "missing options_set_visitor_handle symbol: {c}"
    );
    // high-level: abstract visitor with the overridable method + result mapping.
    assert!(
        c.contains("abstract class RendererVisitor"),
        "missing abstract visitor: {c}"
    );
    assert!(
        c.contains("def visit_text(ctx : RendererVisitorContext, text : String) : WalkDecision"),
        "missing visitor method: {c}"
    );
    assert!(
        c.contains("register_renderer_visitor"),
        "missing registration helper: {c}"
    );
}

#[test]
fn visitor_bridge_typechecks_with_compiler() {
    if Command::new("crystal").arg("--version").output().is_err() {
        eprintln!("skipping: `crystal` compiler not found on PATH");
        return;
    }

    let files = CrystalBackend
        .generate_bindings(&visitor_bridge_api(), &visitor_bridge_config())
        .unwrap();
    // Concatenate the main binding + the visitor bridge (both reopen `lib LibDemo`),
    // then define a concrete visitor and register it — forcing the compiler to
    // type-check the trampolines, Box round-trip, context decode, and result map.
    let mut src = String::new();
    for f in &files {
        src.push_str(&f.content);
        src.push_str("\n\n");
    }
    src.push_str(
        "class MyRenderer < Demo::RendererVisitor\n\
         \x20 def visit_text(ctx : Demo::RendererVisitorContext, text : String) : Demo::WalkDecision\n\
         \x20   ctx.depth > 3 ? Demo::WalkDecision::Stop : Demo::WalkDecision::Continue\n\
         \x20 end\n\
         end\n\n\
         handle = Demo.register_renderer_visitor(MyRenderer.new)\n\
         Demo.free_renderer_visitor(handle)\n",
    );

    let dir = std::env::temp_dir().join(format!("alef_crystal_visitor_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("demo.cr");
    std::fs::write(&path, &src).expect("write source");

    let output = Command::new("crystal")
        .arg("build")
        .arg("--no-codegen")
        .arg(&path)
        .output()
        .expect("run crystal build");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "generated visitor bridge failed to type-check:\n--- source ---\n{src}\n--- crystal stderr ---\n{stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A surface exercising an externally-tagged data enum (unit + newtype variants)
/// used as a function return type, to verify the abstract-class hierarchy and its
/// JSON round-trip type-check.
fn data_enum_api() -> ApiSurface {
    let unit = |name: &str, d: bool| EnumVariant {
        name: name.into(),
        fields: vec![],
        doc: String::new(),
        is_default: d,
        serde_rename: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: false,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    };
    let newtype = |name: &str, ty: TypeRef| EnumVariant {
        name: name.into(),
        fields: vec![FieldDef {
            name: "0".into(),
            ty,
            optional: false,
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
        }],
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
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![make_fn(
            "classify",
            vec![make_param("input", TypeRef::String)],
            TypeRef::Named("Outcome".into()),
            Some("E"),
        )],
        enums: vec![EnumDef {
            name: "Outcome".into(),
            rust_path: "demo::Outcome".into(),
            original_rust_path: String::new(),
            variants: vec![
                unit("Pending", true),
                newtype("Message", TypeRef::String),
                newtype("Code", TypeRef::Primitive(PrimitiveType::I32)),
            ],
            methods: vec![],
            doc: "Classification outcome.".into(),
            cfg: None,
            is_copy: false,
            has_serde: true,
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

#[test]
fn data_enum_emits_tagged_union() {
    let content = &CrystalBackend
        .generate_bindings(&data_enum_api(), &make_config())
        .unwrap()[0]
        .content;
    assert!(
        content.contains("abstract class Outcome"),
        "missing abstract base: {content}"
    );
    assert!(
        content.contains("class Outcome::Pending < Outcome"),
        "missing unit subclass: {content}"
    );
    assert!(
        content.contains("class Outcome::Message < Outcome"),
        "missing newtype subclass: {content}"
    );
    assert!(
        content.contains("getter value : String"),
        "newtype should expose payload: {content}"
    );
    assert!(
        content.contains("getter value : Int32"),
        "int newtype payload: {content}"
    );
}

#[test]
fn data_enum_typechecks_with_compiler() {
    if Command::new("crystal").arg("--version").output().is_err() {
        eprintln!("skipping: `crystal` compiler not found on PATH");
        return;
    }
    let content = &CrystalBackend
        .generate_bindings(&data_enum_api(), &make_config())
        .unwrap()[0]
        .content;
    let harness = format!(
        "{content}\n\n\
         o1 = Demo::Outcome.from_json(%(\"Pending\"))\n\
         o2 = Demo::Outcome.from_json(%({{\"Message\":\"hi\"}}))\n\
         o3 = Demo::Outcome.from_json(%({{\"Code\":7}}))\n\
         puts o1.to_json\n\
         puts o2.to_json\n\
         puts o3.to_json\n\
         Demo.classify(\"x\")\n"
    );
    let dir = std::env::temp_dir().join(format!("alef_crystal_dataenum_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("demo.cr");
    std::fs::write(&path, &harness).expect("write source");
    let output = Command::new("crystal")
        .arg("build")
        .arg("--no-codegen")
        .arg(&path)
        .output()
        .expect("run crystal build");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "generated data enum failed to type-check:\n--- source ---\n{harness}\n--- crystal stderr ---\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// An externally-tagged enum with unit, newtype, tuple(N), and struct variants —
/// verifies the full tagged-union round-trip through the Crystal compiler.
fn rich_enum_api() -> ApiSurface {
    let field = |name: &str, ty: TypeRef| FieldDef {
        name: name.into(),
        ty,
        optional: false,
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
    };
    let variant = |name: &str, fields: Vec<FieldDef>, is_tuple: bool, is_default: bool| EnumVariant {
        name: name.into(),
        originally_had_data_fields: !fields.is_empty(),
        fields,
        doc: String::new(),
        is_default,
        serde_rename: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple,
        cfg: None,
        version: Default::default(),
    };
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![make_fn("run", vec![], TypeRef::Named("Shape".into()), None)],
        enums: vec![EnumDef {
            name: "Shape".into(),
            rust_path: "demo::Shape".into(),
            original_rust_path: String::new(),
            variants: vec![
                variant("Empty", vec![], false, true),
                variant("Named", vec![field("0", TypeRef::String)], true, false),
                variant(
                    "Pair",
                    vec![
                        field("0", TypeRef::Primitive(PrimitiveType::U32)),
                        field("1", TypeRef::Primitive(PrimitiveType::U32)),
                    ],
                    true,
                    false,
                ),
                variant(
                    "Rect",
                    vec![
                        field("w", TypeRef::Primitive(PrimitiveType::U32)),
                        field("h", TypeRef::Primitive(PrimitiveType::U32)),
                    ],
                    false,
                    false,
                ),
            ],
            methods: vec![],
            doc: "A shape.".into(),
            cfg: None,
            is_copy: false,
            has_serde: true,
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

#[test]
fn rich_enum_typechecks_and_roundtrips() {
    if Command::new("crystal").arg("--version").output().is_err() {
        eprintln!("skipping: `crystal` compiler not found on PATH");
        return;
    }
    let content = &CrystalBackend
        .generate_bindings(&rich_enum_api(), &make_config())
        .unwrap()[0]
        .content;
    assert!(
        content.contains("class Shape::Rect < Shape"),
        "struct variant missing: {content}"
    );
    assert!(
        content.contains("class Shape::Pair < Shape"),
        "tuple variant missing: {content}"
    );
    assert!(
        content.contains("getter field0 : UInt32"),
        "tuple getters missing: {content}"
    );
    assert!(
        content.contains("getter w : UInt32"),
        "struct getters missing: {content}"
    );

    let harness = format!(
        "{content}\n\n\
         [%(\"Empty\"), %({{\"Named\":\"x\"}}), %({{\"Pair\":[1,2]}}), %({{\"Rect\":{{\"w\":3,\"h\":4}}}})].each do |j|\n\
         \x20 shape = Demo::Shape.from_json(j)\n\
         \x20 raise \"roundtrip mismatch\" unless Demo::Shape.from_json(shape.to_json).to_json == shape.to_json\n\
         end\n"
    );
    let dir = std::env::temp_dir().join(format!("alef_crystal_richenum_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("demo.cr");
    std::fs::write(&path, &harness).expect("write source");
    let output = Command::new("crystal")
        .arg("build")
        .arg("--no-codegen")
        .arg(&path)
        .output()
        .expect("run crystal build");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "rich enum failed to type-check:\n--- source ---\n{harness}\n--- crystal stderr ---\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// An internally-tagged enum (`#[serde(tag = "type")]`) with a unit and a struct
/// variant — verifies `use_json_discriminator` dispatch + tag re-emission round-trip.
fn internally_tagged_api() -> ApiSurface {
    let field = |name: &str, ty: TypeRef| FieldDef {
        name: name.into(),
        ty,
        optional: false,
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
    };
    let variant = |name: &str, fields: Vec<FieldDef>, is_default: bool| EnumVariant {
        name: name.into(),
        originally_had_data_fields: !fields.is_empty(),
        fields,
        doc: String::new(),
        is_default,
        serde_rename: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: false,
        cfg: None,
        version: Default::default(),
    };
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![make_fn("shape", vec![], TypeRef::Named("Node".into()), None)],
        enums: vec![EnumDef {
            name: "Node".into(),
            rust_path: "demo::Node".into(),
            original_rust_path: String::new(),
            variants: vec![
                variant("Leaf", vec![], true),
                variant(
                    "Branch",
                    vec![field("width", TypeRef::Primitive(PrimitiveType::U32))],
                    false,
                ),
            ],
            methods: vec![],
            doc: "A tree node.".into(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            has_default: true,
            serde_tag: Some("type".into()),
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

#[test]
fn internally_tagged_enum_typechecks_and_roundtrips() {
    if Command::new("crystal").arg("--version").output().is_err() {
        eprintln!("skipping: `crystal` compiler not found on PATH");
        return;
    }
    let content = &CrystalBackend
        .generate_bindings(&internally_tagged_api(), &make_config())
        .unwrap()[0]
        .content;
    assert!(
        content.contains("use_json_discriminator \"type\""),
        "missing discriminator: {content}"
    );
    assert!(
        content.contains("class Node::Branch < Node"),
        "missing struct variant: {content}"
    );

    let harness = format!(
        "{content}\n\n\
         leaf = Demo::Node.from_json(%({{\"type\":\"Leaf\"}}))\n\
         branch = Demo::Node.from_json(%({{\"type\":\"Branch\",\"width\":5}}))\n\
         raise \"leaf\" unless leaf.is_a?(Demo::Node::Leaf)\n\
         raise \"branch\" unless branch.is_a?(Demo::Node::Branch)\n\
         raise \"roundtrip\" unless Demo::Node.from_json(branch.to_json).to_json == branch.to_json\n\
         raise \"tag\" unless branch.to_json.includes?(%(\"type\":\"Branch\"))\n"
    );
    let dir = std::env::temp_dir().join(format!("alef_crystal_internal_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("demo.cr");
    std::fs::write(&path, &harness).expect("write source");
    let output = Command::new("crystal")
        .arg("build")
        .arg("--no-codegen")
        .arg(&path)
        .output()
        .expect("run crystal build");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "internally-tagged enum failed to type-check:\n--- source ---\n{harness}\n--- crystal stderr ---\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// An untagged enum (`#[serde(untagged)]`) with two newtype variants — serializes
/// as a bare payload, deserializes by trying each variant in order.
fn untagged_enum_api() -> ApiSurface {
    let newtype = |name: &str, ty: TypeRef| EnumVariant {
        name: name.into(),
        fields: vec![FieldDef {
            name: "0".into(),
            ty,
            optional: false,
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
        }],
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
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![make_fn("get", vec![], TypeRef::Named("Value".into()), None)],
        enums: vec![EnumDef {
            name: "Value".into(),
            rust_path: "demo::Value".into(),
            original_rust_path: String::new(),
            variants: vec![
                newtype("Number", TypeRef::Primitive(PrimitiveType::I64)),
                newtype("Text", TypeRef::String),
            ],
            methods: vec![],
            doc: "A number or a string.".into(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            has_default: false,
            serde_tag: None,
            serde_untagged: true,
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

#[test]
fn untagged_enum_typechecks_and_roundtrips() {
    if Command::new("crystal").arg("--version").output().is_err() {
        eprintln!("skipping: `crystal` compiler not found on PATH");
        return;
    }
    let content = &CrystalBackend
        .generate_bindings(&untagged_enum_api(), &make_config())
        .unwrap()[0]
        .content;
    assert!(
        content.contains("abstract class Value"),
        "untagged enum should emit a class hierarchy: {content}"
    );
    assert!(
        content.contains("class Value::Number < Value"),
        "missing Number variant: {content}"
    );
    assert!(
        content.contains("class Value::Text < Value"),
        "missing Text variant: {content}"
    );

    let harness = format!(
        "{content}\n\n\
         n = Demo::Value.from_json(%(42))\n\
         t = Demo::Value.from_json(%(\"hello\"))\n\
         raise \"number\" unless n.is_a?(Demo::Value::Number)\n\
         raise \"text\" unless t.is_a?(Demo::Value::Text)\n\
         raise \"n roundtrip\" unless Demo::Value.from_json(n.to_json).to_json == n.to_json\n\
         raise \"t roundtrip\" unless Demo::Value.from_json(t.to_json).to_json == t.to_json\n\
         raise \"bare\" unless n.to_json == \"42\"\n"
    );
    let dir = std::env::temp_dir().join(format!("alef_crystal_untagged_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("demo.cr");
    std::fs::write(&path, &harness).expect("write source");
    let output = Command::new("crystal")
        .arg("build")
        .arg("--no-codegen")
        .arg(&path)
        .output()
        .expect("run crystal build");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "untagged enum failed to type-check:\n--- source ---\n{harness}\n--- crystal stderr ---\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A visitor bridge whose result enum carries a string payload (`Custom(String)`)
/// alongside unit variants — the callback must return the variant code AND write
/// the payload to `out_custom`.
fn visitor_string_payload_api() -> ApiSurface {
    let unit = |name: &str, d: bool| EnumVariant {
        name: name.into(),
        fields: vec![],
        doc: String::new(),
        is_default: d,
        serde_rename: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: false,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    };
    let custom = EnumVariant {
        name: "Custom".into(),
        fields: vec![make_field("0", TypeRef::String)],
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
            },
            TypeDef {
                name: "SyntaxContext".into(),
                rust_path: "demo::SyntaxContext".into(),
                original_rust_path: String::new(),
                fields: vec![make_field("depth", TypeRef::Primitive(PrimitiveType::I32))],
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
            },
        ],
        functions: vec![],
        enums: vec![EnumDef {
            name: "WalkDecision".into(),
            rust_path: "demo::WalkDecision".into(),
            original_rust_path: String::new(),
            variants: vec![unit("Continue", true), custom, unit("Stop", false)],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
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

#[test]
fn visitor_string_payload_result_typechecks() {
    let files = CrystalBackend
        .generate_bindings(&visitor_string_payload_api(), &visitor_bridge_config())
        .expect("string-payload visitor result should be supported");
    let visitor = files
        .iter()
        .find(|f| f.path.display().to_string().ends_with("_renderer_visitor.cr"))
        .expect("expected a generated visitor.cr");
    assert!(
        visitor.content.contains("out_custom.value"),
        "string-payload variant must write out_custom: {}",
        visitor.content
    );

    if Command::new("crystal").arg("--version").output().is_err() {
        return;
    }
    let mut src = String::new();
    for f in &files {
        src.push_str(&f.content);
        src.push_str("\n\n");
    }
    src.push_str(
        "class MyRenderer < Demo::RendererVisitor\n\
         \x20 def visit_text(ctx : Demo::RendererVisitorContext, text : String) : Demo::WalkDecision\n\
         \x20   Demo::WalkDecision::Custom.new(\"replacement\")\n\
         \x20 end\n\
         end\n\n\
         h = Demo.register_renderer_visitor(MyRenderer.new)\n\
         Demo.free_renderer_visitor(h)\n",
    );
    let dir = std::env::temp_dir().join(format!("alef_crystal_vpayload_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("demo.cr");
    std::fs::write(&path, &src).expect("write source");
    let output = Command::new("crystal")
        .arg("build")
        .arg("--no-codegen")
        .arg(&path)
        .output()
        .expect("run crystal build");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "string-payload visitor failed to type-check:\n--- source ---\n{src}\n--- crystal stderr ---\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A self-referential DTO (`LinkedNode { value, next: Option<LinkedNode> }`).
/// Emitting this as a Crystal `struct` is an infinitely-sized recursive value
/// type that the compiler rejects — DTOs must be reference types (`class`).
fn recursive_type_api() -> ApiSurface {
    let field = |name: &str, ty: TypeRef, optional: bool| FieldDef {
        name: name.into(),
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
    };
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "LinkedNode".into(),
            rust_path: "demo::LinkedNode".into(),
            original_rust_path: String::new(),
            fields: vec![
                field("value", TypeRef::Primitive(PrimitiveType::I32), false),
                field(
                    "next",
                    TypeRef::Optional(Box::new(TypeRef::Named("LinkedNode".into()))),
                    true,
                ),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "A singly-linked list node.".into(),
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
        }],
        functions: vec![make_fn("head", vec![], TypeRef::Named("LinkedNode".into()), None)],
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
fn recursive_dto_typechecks() {
    if Command::new("crystal").arg("--version").output().is_err() {
        eprintln!("skipping: `crystal` compiler not found on PATH");
        return;
    }
    let content = &CrystalBackend
        .generate_bindings(&recursive_type_api(), &make_config())
        .unwrap()[0]
        .content;
    let harness = format!(
        "{content}\n\n\
         n = Demo::LinkedNode.from_json(%({{\"value\":1,\"next\":{{\"value\":2}}}}))\n\
         puts n.to_json\n"
    );
    let dir = std::env::temp_dir().join(format!("alef_crystal_recursive_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("demo.cr");
    std::fs::write(&path, &harness).expect("write source");
    let output = Command::new("crystal")
        .arg("build")
        .arg("--no-codegen")
        .arg(&path)
        .output()
        .expect("run crystal build");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "recursive DTO failed to type-check:\n--- source ---\n{harness}\n--- crystal stderr ---\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// An opaque handle type (`Engine`) plus functions that return and consume it.
/// The opaque type must become a Crystal class wrapping the FFI pointer (with a
/// `finalize` freeing it), and functions must pass/wrap the handle — not JSON.
fn opaque_type_api() -> ApiSurface {
    let opaque = TypeDef {
        name: "Engine".into(),
        rust_path: "demo::Engine".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: "An opaque engine handle.".into(),
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
    };
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![opaque],
        functions: vec![
            make_fn("engine_new", vec![], TypeRef::Named("Engine".into()), None),
            make_fn(
                "engine_run",
                vec![
                    make_param("engine", TypeRef::Named("Engine".into())),
                    make_param("input", TypeRef::String),
                ],
                TypeRef::String,
                Some("E"),
            ),
        ],
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
fn opaque_type_emits_handle_wrapper() {
    let content = &CrystalBackend
        .generate_bindings(&opaque_type_api(), &make_config())
        .unwrap()[0]
        .content;
    assert!(
        content.contains("class Engine"),
        "opaque type should emit a wrapper class: {content}"
    );
    assert!(
        content.contains("def to_unsafe"),
        "opaque wrapper needs to_unsafe: {content}"
    );
    assert!(
        content.contains("def finalize"),
        "opaque wrapper needs a finalizer: {content}"
    );
    assert!(
        content.contains("fun engine_free = demo_engine_free"),
        "missing free binding: {content}"
    );
    // The lib fun for engine_new returns a handle pointer, not a JSON string.
    assert!(
        content.contains("fun engine_new = demo_engine_new() : Void*"),
        "opaque return should be a pointer: {content}"
    );
    // engine_run passes the handle, not JSON.
    assert!(
        content.contains("engine.to_unsafe"),
        "opaque param should pass the handle: {content}"
    );
}

#[test]
fn opaque_type_typechecks() {
    if Command::new("crystal").arg("--version").output().is_err() {
        eprintln!("skipping: `crystal` compiler not found on PATH");
        return;
    }
    let content = &CrystalBackend
        .generate_bindings(&opaque_type_api(), &make_config())
        .unwrap()[0]
        .content;
    let harness = format!(
        "{content}\n\n\
         e = Demo.engine_new\n\
         puts Demo.engine_run(e, \"hi\")\n"
    );
    let dir = std::env::temp_dir().join(format!("alef_crystal_opaque_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("demo.cr");
    std::fs::write(&path, &harness).expect("write source");
    let output = Command::new("crystal")
        .arg("build")
        .arg("--no-codegen")
        .arg(&path)
        .output()
        .expect("run crystal build");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "opaque type failed to type-check:\n--- source ---\n{harness}\n--- crystal stderr ---\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// An opaque type with an instance method — the wrapper class should expose it,
/// calling the `{prefix}_{type}_{method}` FFI symbol with the handle as receiver.
fn opaque_method_api() -> ApiSurface {
    let method = MethodDef {
        name: "process".into(),
        params: vec![make_param("input", TypeRef::String)],
        return_type: TypeRef::String,
        is_async: false,
        is_static: false,
        error_type: Some("E".into()),
        doc: "Process input through the engine.".into(),
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
    let opaque = TypeDef {
        name: "Engine".into(),
        rust_path: "demo::Engine".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![method],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: "An opaque engine handle.".into(),
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
    };
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![opaque],
        functions: vec![make_fn("engine_new", vec![], TypeRef::Named("Engine".into()), None)],
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
fn opaque_instance_method_emitted() {
    let content = &CrystalBackend
        .generate_bindings(&opaque_method_api(), &make_config())
        .unwrap()[0]
        .content;
    assert!(
        content.contains("fun engine_process = demo_engine_process(handle : Void*, input : LibC::Char*) : LibC::Char*"),
        "missing method lib binding: {content}"
    );
    assert!(
        content.contains("def process(input : String) : String"),
        "missing instance method: {content}"
    );
    assert!(
        content.contains("LibDemo.engine_process(@handle, input)"),
        "method should pass @handle: {content}"
    );
}

#[test]
fn opaque_method_typechecks() {
    if Command::new("crystal").arg("--version").output().is_err() {
        eprintln!("skipping: `crystal` compiler not found on PATH");
        return;
    }
    let content = &CrystalBackend
        .generate_bindings(&opaque_method_api(), &make_config())
        .unwrap()[0]
        .content;
    let harness = format!("{content}\n\ne = Demo.engine_new\nputs e.process(\"hi\")\n");
    let dir = std::env::temp_dir().join(format!("alef_crystal_opqm_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("demo.cr");
    std::fs::write(&path, &harness).expect("write source");
    let output = Command::new("crystal")
        .arg("build")
        .arg("--no-codegen")
        .arg(&path)
        .output()
        .expect("run crystal build");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "opaque method failed to type-check:\n{harness}\n---\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A streaming method on an opaque handle (`Engine::tokens -> stream of Token`),
/// configured via `[[crates.adapters]] pattern = "streaming"`. Should emit a
/// fiber-fed `Channel(Token)` using the iterator `_start`/`_next`/`_free` ABI.
fn streaming_api() -> ApiSurface {
    let token = TypeDef {
        name: "Token".into(),
        rust_path: "demo::Token".into(),
        original_rust_path: String::new(),
        fields: vec![make_field("text", TypeRef::String)],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: "A streamed token.".into(),
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
    let engine = TypeDef {
        name: "Engine".into(),
        rust_path: "demo::Engine".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: "An opaque engine.".into(),
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
    };
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![engine, token],
        functions: vec![make_fn("engine_new", vec![], TypeRef::Named("Engine".into()), None)],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn streaming_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["crystal"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[[crates.adapters]]
name = "tokens"
pattern = "streaming"
core_path = "demo::Engine::tokens"
owner_type = "Engine"
item_type = "Token"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

#[test]
fn streaming_method_emits_channel() {
    let content = &CrystalBackend
        .generate_bindings(&streaming_api(), &streaming_config())
        .unwrap()[0]
        .content;
    assert!(
        content.contains("def tokens : Channel(Token)"),
        "missing streaming method: {content}"
    );
    assert!(
        content.contains("fun engine_tokens_start"),
        "missing _start binding: {content}"
    );
    assert!(
        content.contains("fun engine_tokens_next"),
        "missing _next binding: {content}"
    );
    assert!(
        content.contains("fun engine_tokens_free"),
        "missing iterator _free binding: {content}"
    );
    assert!(
        content.contains("fun token_to_json"),
        "missing item to_json binding: {content}"
    );
    assert!(content.contains("spawn"), "streaming should use a fiber: {content}");
    assert!(content.contains(".send("), "streaming should feed a channel: {content}");
}

#[test]
fn streaming_method_typechecks() {
    if Command::new("crystal").arg("--version").output().is_err() {
        eprintln!("skipping: `crystal` compiler not found on PATH");
        return;
    }
    let content = &CrystalBackend
        .generate_bindings(&streaming_api(), &streaming_config())
        .unwrap()[0]
        .content;
    let harness = format!(
        "{content}\n\n\
         e = Demo.engine_new\n\
         ch = e.tokens\n\
         spawn do\n\
         \x20 while tok = ch.receive?\n\
         \x20   puts tok.text\n\
         \x20 end\n\
         end\n"
    );
    let dir = std::env::temp_dir().join(format!("alef_crystal_stream_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("demo.cr");
    std::fs::write(&path, &harness).expect("write source");
    let output = Command::new("crystal")
        .arg("build")
        .arg("--no-codegen")
        .arg(&path)
        .output()
        .expect("run crystal build");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "streaming failed to type-check:\n{harness}\n---\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn streaming_with_params_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["crystal"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[[crates.adapters]]
name = "tokens"
pattern = "streaming"
core_path = "demo::Engine::tokens"
owner_type = "Engine"
item_type = "Token"
params = [{ name = "prompt", type = "String" }, { name = "limit", type = "u32" }]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

#[test]
fn streaming_method_with_params_emits_signature() {
    let content = &CrystalBackend
        .generate_bindings(&streaming_api(), &streaming_with_params_config())
        .unwrap()[0]
        .content;
    assert!(
        content.contains("def tokens(prompt : String, limit : UInt32) : Channel(Token)"),
        "streaming method should carry its params: {content}"
    );
    assert!(
        content.contains(
            "fun engine_tokens_start = demo_engine_tokens_start(handle : Void*, prompt : LibC::Char*, limit : UInt32)"
        ),
        "start binding should carry params: {content}"
    );
    assert!(
        content.contains("_start(@handle, prompt, limit)"),
        "start call should marshal params: {content}"
    );
}

#[test]
fn streaming_method_with_params_typechecks() {
    if Command::new("crystal").arg("--version").output().is_err() {
        eprintln!("skipping: `crystal` compiler not found on PATH");
        return;
    }
    let content = &CrystalBackend
        .generate_bindings(&streaming_api(), &streaming_with_params_config())
        .unwrap()[0]
        .content;
    let harness = format!(
        "{content}\n\n\
         e = Demo.engine_new\n\
         ch = e.tokens(\"hi\", 5_u32)\n\
         spawn do\n\
         \x20 while t = ch.receive?\n\
         \x20   puts t.text\n\
         \x20 end\n\
         end\n"
    );
    let dir = std::env::temp_dir().join(format!("alef_crystal_streamp_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("demo.cr");
    std::fs::write(&path, &harness).expect("write source");
    let output = Command::new("crystal")
        .arg("build")
        .arg("--no-codegen")
        .arg(&path)
        .output()
        .expect("run crystal build");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "streaming w/ params failed to type-check:\n{harness}\n---\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A free-function stream (owner is not an opaque handle) → a module-level
/// `self.<method>` returning a Channel, with no receiver handle in `_start`.
fn free_stream_api() -> ApiSurface {
    let event = TypeDef {
        name: "Event".into(),
        rust_path: "demo::Event".into(),
        original_rust_path: String::new(),
        fields: vec![make_field("kind", TypeRef::String)],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: "A streamed event.".into(),
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
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![event],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn free_stream_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["crystal"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[[crates.adapters]]
name = "poll"
pattern = "streaming"
core_path = "demo::Feed::poll"
owner_type = "Feed"
item_type = "Event"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

#[test]
fn free_function_stream_emits_module_method() {
    let content = &CrystalBackend
        .generate_bindings(&free_stream_api(), &free_stream_config())
        .unwrap()[0]
        .content;
    assert!(
        content.contains("def self.poll : Channel(Event)"),
        "free stream should be a module method: {content}"
    );
    assert!(
        content.contains("fun feed_poll_start = demo_feed_poll_start() : Void*"),
        "free stream _start takes no handle: {content}"
    );
}

#[test]
fn free_function_stream_typechecks() {
    if Command::new("crystal").arg("--version").output().is_err() {
        eprintln!("skipping: `crystal` compiler not found on PATH");
        return;
    }
    let content = &CrystalBackend
        .generate_bindings(&free_stream_api(), &free_stream_config())
        .unwrap()[0]
        .content;
    let harness = format!(
        "{content}\n\n\
         ch = Demo.poll\n\
         spawn do\n\
         \x20 while ev = ch.receive?\n\
         \x20   puts ev.kind\n\
         \x20 end\n\
         end\n"
    );
    let dir = std::env::temp_dir().join(format!("alef_crystal_freestream_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("demo.cr");
    std::fs::write(&path, &harness).expect("write source");
    let output = Command::new("crystal")
        .arg("build")
        .arg("--no-codegen")
        .arg(&path)
        .output()
        .expect("run crystal build");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "free stream failed to type-check:\n{harness}\n---\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// End-to-end link + run: build a C shared library implementing the exact FFI
/// symbols the generated Crystal binding expects, then `crystal build` (full link)
/// the binding against it and run it. This validates that the emitted `lib` symbol
/// names, C signatures, and runtime marshalling (scalar pass-through, String↔char*,
/// free_string ownership) are genuinely ABI-correct and linkable — not just that
/// the Crystal type-checks.
fn link_api() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![
            make_fn(
                "add",
                vec![
                    make_param("a", TypeRef::Primitive(PrimitiveType::I32)),
                    make_param("b", TypeRef::Primitive(PrimitiveType::I32)),
                ],
                TypeRef::Primitive(PrimitiveType::I32),
                None,
            ),
            make_fn(
                "greet",
                vec![make_param("name", TypeRef::String)],
                TypeRef::String,
                None,
            ),
        ],
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
fn ffi_link_and_run_against_c_oracle() {
    let have_cc = Command::new("cc").arg("--version").output().is_ok();
    let have_crystal = Command::new("crystal").arg("--version").output().is_ok();
    if !have_cc || !have_crystal {
        eprintln!("skipping: need both `cc` and `crystal` on PATH");
        return;
    }

    let content = &CrystalBackend.generate_bindings(&link_api(), &make_config()).unwrap()[0].content;

    let dir = std::env::temp_dir().join(format!("alef_crystal_link_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");

    // C oracle implementing the documented ABI contract.
    let shim = r#"#include <stdint.h>
#include <stdlib.h>
#include <string.h>
int32_t demo_add(int32_t a, int32_t b) { return a + b; }
/* `name` arrives JSON-encoded (e.g. "\"world\""); return an owned greeting. */
char* demo_greet(const char* name) {
    const char* msg = "hello";
    char* out = (char*) malloc(strlen(msg) + 1);
    strcpy(out, msg);
    return out;
}
void demo_free_string(char* ptr) { free(ptr); }
"#;
    std::fs::write(dir.join("shim.c"), shim).expect("write shim.c");

    // Build the shared library (matching `@[Link(ldflags: "-ldemo_ffi")]`).
    let lib_file = if cfg!(target_os = "macos") {
        "libdemo_ffi.dylib"
    } else {
        "libdemo_ffi.so"
    };
    let mut cc = Command::new("cc");
    cc.current_dir(&dir);
    if cfg!(target_os = "macos") {
        cc.args(["-dynamiclib", "-o", lib_file, "shim.c"]);
    } else {
        cc.args(["-shared", "-fPIC", "-o", lib_file, "shim.c"]);
    }
    let cc_out = cc.output().expect("run cc");
    assert!(
        cc_out.status.success(),
        "cc failed: {}",
        String::from_utf8_lossy(&cc_out.stderr)
    );

    // Generated binding + a main that asserts scalar and string round-trips.
    let program = format!(
        "{content}\n\nabort(\"add\") unless Demo.add(2_i32, 3_i32) == 5\nabort(\"greet\") unless Demo.greet(\"world\") == \"hello\"\nputs \"OK\"\n"
    );
    std::fs::write(dir.join("demo.cr"), &program).expect("write demo.cr");

    let dir_str = dir.to_string_lossy().to_string();
    let exe = dir.join("demo_prog");
    let build = Command::new("crystal")
        .current_dir(&dir)
        .args(["build", "demo.cr", "-o"])
        .arg(&exe)
        .arg("--link-flags")
        .arg(format!("-L{dir_str} -Wl,-rpath,{dir_str}"))
        .output()
        .expect("run crystal build");
    assert!(
        build.status.success(),
        "crystal build (link) failed:\n--- program ---\n{program}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(&exe)
        .env("DYLD_LIBRARY_PATH", &dir_str)
        .env("LD_LIBRARY_PATH", &dir_str)
        .output()
        .expect("run generated program");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success() && stdout.trim() == "OK",
        "linked program did not run cleanly: status={:?} stdout={stdout:?} stderr={:?}",
        run.status,
        String::from_utf8_lossy(&run.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A plugin-style trait bridge (`register_fn` + `Plugin` super-trait): a Crystal
/// object registered into a Rust registry, implementing a Rust trait through a
/// `#[repr(C)]` vtable. This is the registry pattern (distinct from visitor bridges).
fn plugin_bridge_api() -> ApiSurface {
    let store = TypeDef {
        name: "Store".into(),
        rust_path: "demo::Store".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![MethodDef {
            name: "fetch".into(),
            params: vec![make_param("key", TypeRef::String)],
            return_type: TypeRef::String,
            is_async: false,
            is_static: false,
            error_type: Some("StoreError".into()),
            doc: "Fetch a value by key.".into(),
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
        }],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        doc: "A pluggable key-value store.".into(),
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
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![store],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn plugin_bridge_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["crystal"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[[crates.trait_bridges]]
trait_name = "Store"
super_trait = "Plugin"
register_fn = "register_store"
unregister_fn = "unregister_store"
registry_getter = "store_registry"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

#[test]
fn plugin_bridge_emits_registry_api() {
    let files = CrystalBackend
        .generate_bindings(&plugin_bridge_api(), &plugin_bridge_config())
        .expect("plugin bridge should be supported");
    let content: String = files.iter().map(|f| f.content.clone()).collect::<Vec<_>>().join("\n");
    let content = &content;
    // VTable lib struct with the fetch fn-pointer + free_string/free_user_data.
    assert!(
        content.contains("struct StoreVTable"),
        "missing vtable struct: {content}"
    );
    assert!(content.contains("free_string :"), "vtable needs free_string: {content}");
    assert!(
        content.contains("free_user_data :"),
        "vtable needs free_user_data: {content}"
    );
    // register / unregister lib symbols.
    assert!(
        content.contains("fun register_store = demo_register_store"),
        "missing register symbol: {content}"
    );
    assert!(
        content.contains("= demo_unregister_store"),
        "missing unregister symbol: {content}"
    );
    // High-level abstract class + registry API.
    assert!(
        content.contains("abstract class Store"),
        "missing abstract class: {content}"
    );
    assert!(
        content.contains("def fetch(key : String) : String"),
        "missing trait method: {content}"
    );
    assert!(
        content.contains("def self.register_store(name : String, impl : Store)"),
        "missing register API: {content}"
    );
    assert!(
        content.contains("def self.unregister_store(name : String)"),
        "missing unregister API: {content}"
    );
}

#[test]
fn plugin_bridge_typechecks() {
    if Command::new("crystal").arg("--version").output().is_err() {
        eprintln!("skipping: `crystal` compiler not found on PATH");
        return;
    }
    let files = CrystalBackend
        .generate_bindings(&plugin_bridge_api(), &plugin_bridge_config())
        .unwrap();
    let mut src = String::new();
    for f in &files {
        src.push_str(&f.content);
        src.push_str("\n\n");
    }
    src.push_str(
        "class MyStore < Demo::Store\n\
         \x20 def fetch(key : String) : String\n\
         \x20   \"value-for-\" + key\n\
         \x20 end\n\
         end\n\n\
         Demo.register_store(\"test\", MyStore.new)\n\
         Demo.unregister_store(\"test\")\n",
    );
    let dir = std::env::temp_dir().join(format!("alef_crystal_plugin_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("demo.cr");
    std::fs::write(&path, &src).expect("write source");
    let output = Command::new("crystal")
        .arg("build")
        .arg("--no-codegen")
        .arg(&path)
        .output()
        .expect("run crystal build");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "plugin bridge failed to type-check:\n{src}\n---\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plugin_bridge_link_and_run_against_c_oracle() {
    let have_cc = Command::new("cc").arg("--version").output().is_ok();
    let have_crystal = Command::new("crystal").arg("--version").output().is_ok();
    if !have_cc || !have_crystal {
        eprintln!("skipping: need both `cc` and `crystal`");
        return;
    }

    // Generate all binding files (main + plugin bridge) and concatenate them.
    let files = CrystalBackend
        .generate_bindings(&plugin_bridge_api(), &plugin_bridge_config())
        .unwrap();
    let mut binding = String::new();
    for f in &files {
        binding.push_str(&f.content);
        binding.push_str("\n\n");
    }

    let dir = std::env::temp_dir().join(format!("alef_crystal_plink_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");

    // C oracle: a registry that stores the vtable + user_data, plus probes that
    // invoke the registered plugin's callbacks (simulating Rust calling in).
    // The StoreVTable layout MUST match the generated `#[repr(C)]` order.
    let shim = r#"#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
typedef struct {
    int32_t (*name_fn)(const void*, char**, char**);
    int32_t (*version_fn)(const void*, char**, char**);
    int32_t (*initialize_fn)(const void*, char**);
    int32_t (*shutdown_fn)(const void*, char**);
    int32_t (*fetch)(const void*, const char*, char**, char**);
    void (*free_string)(char*);
    void (*free_user_data)(void*);
} StoreVTable;
static StoreVTable g_vt;
static const void* g_ud = 0;
static int g_reg = 0;
int32_t demo_register_store(const char* name, const StoreVTable* vt, const void* ud, char** out_err) {
    (void)name; (void)out_err; g_vt = *vt; g_ud = ud; g_reg = 1; return 0;
}
int32_t demo_unregister_store(const char* name, char** out_err) { (void)name; (void)out_err; g_reg = 0; return 0; }
void demo_free_string(char* p) { free(p); }
static void copy_out(char* out, size_t cap, char* result) {
    size_t i = 0;
    if (result) { for (; result[i] && i + 1 < cap; i++) out[i] = result[i]; }
    out[i] = 0;
}
int32_t demo_probe_fetch(const char* key, char* out, size_t cap) {
    if (!g_reg || !g_vt.fetch) return -1;
    char* result = 0; char* err = 0;
    int32_t st = g_vt.fetch(g_ud, key, &result, &err);
    if (st != 0) { if (err && g_vt.free_string) g_vt.free_string(err); return st; }
    copy_out(out, cap, result);
    if (result && g_vt.free_string) g_vt.free_string(result);
    return 0;
}
int32_t demo_probe_name(char* out, size_t cap) {
    if (!g_reg || !g_vt.name_fn) return -1;
    char* result = 0; char* err = 0;
    int32_t st = g_vt.name_fn(g_ud, &result, &err);
    if (st != 0) return st;
    copy_out(out, cap, result);
    if (result && g_vt.free_string) g_vt.free_string(result);
    return 0;
}
"#;
    std::fs::write(dir.join("shim.c"), shim).expect("write shim.c");

    let lib_file = if cfg!(target_os = "macos") {
        "libdemo_ffi.dylib"
    } else {
        "libdemo_ffi.so"
    };
    let mut cc = Command::new("cc");
    cc.current_dir(&dir);
    if cfg!(target_os = "macos") {
        cc.args(["-dynamiclib", "-o", lib_file, "shim.c"]);
    } else {
        cc.args(["-shared", "-fPIC", "-o", lib_file, "shim.c"]);
    }
    let cc_out = cc.output().expect("run cc");
    assert!(
        cc_out.status.success(),
        "cc failed: {}",
        String::from_utf8_lossy(&cc_out.stderr)
    );

    // Program: register a Crystal Store impl, then probe it through the vtable.
    let program = format!(
        "{binding}\n\n\
         lib LibDemo\n\
         \x20 fun probe_fetch = demo_probe_fetch(key : LibC::Char*, out : LibC::Char*, cap : LibC::SizeT) : Int32\n\
         \x20 fun probe_name = demo_probe_name(out : LibC::Char*, cap : LibC::SizeT) : Int32\n\
         end\n\n\
         class MyStore < Demo::Store\n\
         \x20 def name : String\n\
         \x20   \"test-store\"\n\
         \x20 end\n\
         \x20 def fetch(key : String) : String\n\
         \x20   \"value:\" + key\n\
         \x20 end\n\
         end\n\n\
         raise \"register failed\" unless Demo.register_store(\"test\", MyStore.new)\n\
         buf = Bytes.new(256)\n\
         st = LibDemo.probe_fetch(\"hello\", buf.to_unsafe.as(LibC::Char*), LibC::SizeT.new(256))\n\
         raise \"fetch status #{{st}}\" unless st == 0\n\
         got = String.new(buf.to_unsafe)\n\
         raise \"fetch got #{{got}}\" unless got == \"value:hello\"\n\
         nbuf = Bytes.new(64)\n\
         LibDemo.probe_name(nbuf.to_unsafe.as(LibC::Char*), LibC::SizeT.new(64))\n\
         raise \"name got #{{String.new(nbuf.to_unsafe)}}\" unless String.new(nbuf.to_unsafe) == \"test-store\"\n\
         raise \"unregister failed\" unless Demo.unregister_store(\"test\")\n\
         puts \"OK\"\n"
    );
    std::fs::write(dir.join("demo.cr"), &program).expect("write demo.cr");

    let dir_str = dir.to_string_lossy().to_string();
    let exe = dir.join("plugin_prog");
    let build = Command::new("crystal")
        .current_dir(&dir)
        .args(["build", "demo.cr", "-o"])
        .arg(&exe)
        .arg("--link-flags")
        .arg(format!("-L{dir_str} -Wl,-rpath,{dir_str}"))
        .output()
        .expect("run crystal build");
    assert!(
        build.status.success(),
        "crystal build (plugin link) failed:\n--- program ---\n{program}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(&exe)
        .env("DYLD_LIBRARY_PATH", &dir_str)
        .env("LD_LIBRARY_PATH", &dir_str)
        .output()
        .expect("run plugin program");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success() && stdout.trim() == "OK",
        "plugin program failed: status={:?} stdout={stdout:?} stderr={:?}",
        run.status,
        String::from_utf8_lossy(&run.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Full e2e against a real Rust-compiled cdylib (rustc `--crate-type=cdylib`),
/// exercising Rust's actual `CString` allocator/ABI interplay with the Crystal
/// binding's `free_string` — a step beyond the C oracle. Covers regular function
/// returns (Rust `CString::into_raw`) and the plugin bridge (Crystal-malloc'd
/// `out_result` freed by the Crystal `free_string` the Rust side invokes).
#[test]
fn full_e2e_against_real_rust_cdylib() {
    let have_rustc = Command::new("rustc").arg("--version").output().is_ok();
    let have_crystal = Command::new("crystal").arg("--version").output().is_ok();
    if !have_rustc || !have_crystal {
        eprintln!("skipping: need both `rustc` and `crystal`");
        return;
    }

    // API: a function (greet) + a plugin Store trait.
    let mut api = plugin_bridge_api();
    api.functions = vec![make_fn(
        "greet",
        vec![make_param("name", TypeRef::String)],
        TypeRef::String,
        None,
    )];
    let files = CrystalBackend.generate_bindings(&api, &plugin_bridge_config()).unwrap();
    let mut binding = String::new();
    for f in &files {
        binding.push_str(&f.content);
        binding.push_str("\n\n");
    }

    let dir = std::env::temp_dir().join(format!("alef_crystal_rustffi_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");

    // Real Rust FFI cdylib: greet returns a CString (Rust allocator); the plugin
    // registry stores the vtable and a probe invokes fetch through it.
    let rust = r#"
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

#[unsafe(no_mangle)]
pub extern "C" fn demo_greet(name: *const c_char) -> *mut c_char {
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
    CString::new(format!("hi {}", name)).unwrap().into_raw()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn demo_free_string(p: *mut c_char) {
    if !p.is_null() { drop(unsafe { CString::from_raw(p) }); }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct StoreVTable {
    name_fn: Option<unsafe extern "C" fn(*const c_void, *mut *mut c_char, *mut *mut c_char) -> i32>,
    version_fn: Option<unsafe extern "C" fn(*const c_void, *mut *mut c_char, *mut *mut c_char) -> i32>,
    initialize_fn: Option<unsafe extern "C" fn(*const c_void, *mut *mut c_char) -> i32>,
    shutdown_fn: Option<unsafe extern "C" fn(*const c_void, *mut *mut c_char) -> i32>,
    fetch: Option<unsafe extern "C" fn(*const c_void, *const c_char, *mut *mut c_char, *mut *mut c_char) -> i32>,
    free_string: Option<unsafe extern "C" fn(*mut c_char)>,
    free_user_data: Option<unsafe extern "C" fn(*mut c_void)>,
}

static mut G_VT: Option<StoreVTable> = None;
static mut G_UD: *const c_void = std::ptr::null();

#[unsafe(no_mangle)]
pub unsafe extern "C" fn demo_register_store(_name: *const c_char, vt: *const StoreVTable, ud: *const c_void, _e: *mut *mut c_char) -> i32 {
    unsafe { G_VT = Some(*vt); G_UD = ud; }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn demo_unregister_store(_name: *const c_char, _e: *mut *mut c_char) -> i32 {
    unsafe { G_VT = None; }
    0
}

/// Invoke the registered plugin's fetch and return the result as a Rust CString.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn demo_probe_fetch(key: *const c_char) -> *mut c_char {
    let (vt, ud) = unsafe { (G_VT, G_UD) };
    let vt = match vt { Some(v) => v, None => return std::ptr::null_mut() };
    let fetch = match vt.fetch { Some(f) => f, None => return std::ptr::null_mut() };
    let mut out_result: *mut c_char = std::ptr::null_mut();
    let mut out_error: *mut c_char = std::ptr::null_mut();
    let status = unsafe { fetch(ud, key, &mut out_result, &mut out_error) };
    if status != 0 || out_result.is_null() { return std::ptr::null_mut(); }
    // Read the (Crystal-malloc'd) string, copy into a Rust CString, then free via the vtable.
    let s = unsafe { CStr::from_ptr(out_result) }.to_string_lossy().into_owned();
    if let Some(free) = vt.free_string { unsafe { free(out_result); } }
    CString::new(s).unwrap().into_raw()
}
"#;
    std::fs::write(dir.join("shim.rs"), rust).expect("write shim.rs");

    let lib_file = if cfg!(target_os = "macos") {
        "libdemo_ffi.dylib"
    } else {
        "libdemo_ffi.so"
    };
    let rc = Command::new("rustc")
        .current_dir(&dir)
        .args(["--crate-type=cdylib", "--edition=2024", "-O", "shim.rs", "-o", lib_file])
        .output()
        .expect("run rustc");
    assert!(
        rc.status.success(),
        "rustc failed: {}",
        String::from_utf8_lossy(&rc.stderr)
    );

    let program = format!(
        "{binding}\n\n\
         lib LibDemo\n\
         \x20 fun probe_fetch = demo_probe_fetch(key : LibC::Char*) : LibC::Char*\n\
         end\n\n\
         raise \"greet\" unless Demo.greet(\"world\") == \"hi world\"\n\
         class MyStore < Demo::Store\n\
         \x20 def fetch(key : String) : String\n\
         \x20   \"got:\" + key\n\
         \x20 end\n\
         end\n\
         raise \"register\" unless Demo.register_store(\"test\", MyStore.new)\n\
         rp = LibDemo.probe_fetch(\"k1\")\n\
         raise \"probe null\" if rp.null?\n\
         got = String.new(rp)\n\
         LibDemo.free_string(rp)\n\
         raise \"probe got #{{got}}\" unless got == \"got:k1\"\n\
         Demo.unregister_store(\"test\")\n\
         puts \"OK\"\n"
    );
    std::fs::write(dir.join("demo.cr"), &program).expect("write demo.cr");

    let dir_str = dir.to_string_lossy().to_string();
    let exe = dir.join("e2e_prog");
    let build = Command::new("crystal")
        .current_dir(&dir)
        .args(["build", "demo.cr", "-o"])
        .arg(&exe)
        .arg("--link-flags")
        .arg(format!("-L{dir_str} -Wl,-rpath,{dir_str}"))
        .output()
        .expect("run crystal build");
    assert!(
        build.status.success(),
        "crystal build failed:\n{program}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(&exe)
        .env("DYLD_LIBRARY_PATH", &dir_str)
        .env("LD_LIBRARY_PATH", &dir_str)
        .output()
        .expect("run e2e program");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success() && stdout.trim() == "OK",
        "e2e program failed: status={:?} stdout={stdout:?} stderr={:?}",
        run.status,
        String::from_utf8_lossy(&run.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}
