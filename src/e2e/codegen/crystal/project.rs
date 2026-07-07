//! Crystal e2e project file rendering: shard.yml, spec_helper, and specs.

use crate::core::config::TraitBridgeConfig;
use crate::core::config::e2e::CallConfig;
use crate::core::ir::{MethodDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

use super::http;
use super::stubs::{MODULE_PLACEHOLDER, emit_test_backend};

/// Render the e2e `shard.yml` with a path dependency on the generated binding.
pub(super) fn render_shard_yml(shard_name: &str, pkg_path: &str) -> String {
    format!(
        r#"name: {shard_name}_e2e
version: 0.0.0

dependencies:
  {shard_name}:
    path: {pkg_path}

crystal: ">= 1.0.0"
"#
    )
}

/// Render `spec/spec_helper.cr` — requires spec and the generated binding.
pub(super) fn render_spec_helper(shard_name: &str) -> String {
    format!("require \"spec\"\nrequire \"{shard_name}\"\n")
}

/// Render a smoke spec that links the binding and checks its VERSION.
pub(super) fn render_smoke_spec(module_name: &str) -> String {
    format!(
        r#"require "./spec_helper"

describe {module_name} do
  it "links the generated binding" do
    {module_name}::VERSION.should_not be_empty
  end
end
"#
    )
}

/// Render a per-category spec file. Fixtures with assertions become real
/// examples that call the configured function and assert on the result; fixtures
/// without assertions stay as `pending` placeholders.
pub(super) fn render_category_spec(
    category: &str,
    fixtures: &[&Fixture],
    module_name: &str,
    e2e_config: &E2eConfig,
    trait_bridges: &[TraitBridgeConfig],
    type_defs: &[TypeDef],
) -> String {
    let mut out = String::from("require \"./spec_helper\"\n\n");
    out.push_str(&format!("describe {module_name} do\n"));
    out.push_str(&format!("  describe {category:?} do\n"));
    for fixture in fixtures {
        let desc = if fixture.description.is_empty() {
            &fixture.id
        } else {
            &fixture.description
        };

        let call_config = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            category,
            &fixture.tags,
            &fixture.input,
        );

        let is_http_fixture = fixture.mock_response.is_some() || fixture.http.is_some();

        if fixture.assertions.is_empty() && !is_http_fixture {
            out.push_str(&format!("    pending {desc:?}\n"));
            continue;
        }

        if fixture.http.is_some() {
            if http::render_http_test_spec(&mut out, fixture) {
                continue;
            }
            out.push_str(&format!("    pending {desc:?}\n"));
            continue;
        }

        if is_http_fixture {
            out.push_str(&format!("    pending {desc:?}\n"));
            continue;
        }

        let function_name = call_config
            .overrides
            .get("crystal")
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call_config.function.clone());

        let base_options_type = call_config.options_type.as_deref();
        let options_type = call_config
            .overrides
            .get("crystal")
            .and_then(|o| o.options_type.as_deref())
            .or(base_options_type);

        let result_var = if call_config.result_var.is_empty() || call_config.result_var == "result" {
            "__result"
        } else {
            call_config.result_var.as_str()
        };

        let (setup_lines, call_args_str, teardown_lines) = build_args_and_setup(
            fixture,
            call_config,
            module_name,
            trait_bridges,
            type_defs,
            options_type,
        );

        out.push_str(&format!("    it {desc:?} do\n"));

        for line in &setup_lines {
            for l in line.lines() {
                if l.is_empty() {
                    out.push('\n');
                } else {
                    out.push_str(&format!("      {l}\n"));
                }
            }
        }

        let call = format!("{module_name}.{function_name}({call_args_str})");
        let returns_void = call_config.returns_void;

        if fixture.assertions.iter().any(|a| a.assertion_type == "error") {
            out.push_str("      expect_raises(Exception) do\n");
            out.push_str(&format!("        {call}\n"));
            out.push_str("      end\n");
        } else if returns_void {
            out.push_str(&format!("      {call}\n"));
            for a in &fixture.assertions {
                out.push_str(&render_void_assertion(a));
            }
        } else {
            out.push_str(&format!("      {result_var} = {call}\n"));
            for a in &fixture.assertions {
                out.push_str(&render_assertion(a, result_var));
            }
        }

        for line in &teardown_lines {
            for l in line.lines() {
                if l.is_empty() {
                    out.push('\n');
                } else {
                    out.push_str(&format!("      {l}\n"));
                }
            }
        }

        out.push_str("    end\n");
    }
    out.push_str("  end\nend\n");
    out
}

/// Build Crystal argument expressions and setup/teardown lines from a
/// fixture's input using `CallConfig.args`. Returns `(setup_lines,
/// call_args_str, teardown_lines)`.
fn build_args_and_setup(
    fixture: &Fixture,
    call_config: &CallConfig,
    module_name: &str,
    trait_bridges: &[TraitBridgeConfig],
    type_defs: &[TypeDef],
    options_type: Option<&str>,
) -> (Vec<String>, String, Vec<String>) {
    let args = fixture.resolved_args(call_config);

    let mut setup_lines: Vec<String> = Vec::new();
    let mut call_parts: Vec<String> = Vec::new();
    let mut teardown_lines: Vec<String> = Vec::new();

    if args.is_empty() {
        let arg = call_args_fallback(&fixture.input);
        if !arg.is_empty() {
            call_parts.push(arg);
        }
        return (setup_lines, call_parts.join(", "), teardown_lines);
    }

    for arg in args {
        let value = resolve_json_field(&fixture.input, &arg.field);

        match arg.arg_type.as_str() {
            "test_backend" => {
                if let Some(trait_name) = &arg.trait_name {
                    if let Some(trait_bridge) = trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                        let methods: Vec<&MethodDef> = type_defs
                            .iter()
                            .find(|t| t.name == *trait_name)
                            .map(|t| t.methods.iter().collect())
                            .unwrap_or_default();

                        let emission = emit_test_backend(trait_bridge, &methods, fixture);

                        let setup = emission.setup_block.replace(MODULE_PLACEHOLDER, module_name);
                        let teardown = emission.teardown_block.replace(MODULE_PLACEHOLDER, module_name);

                        if !setup.trim().is_empty() {
                            setup_lines.push(setup);
                        }

                        if let Some(register_fn) = &trait_bridge.register_fn {
                            let reg_args = emission.arg_expr.replace(MODULE_PLACEHOLDER, module_name);
                            setup_lines.push(format!("{module_name}.{register_fn}({reg_args})"));
                        }

                        if !teardown.trim().is_empty() {
                            teardown_lines.push(teardown);
                        }
                    }
                }
            }
            "json_object" => {
                let json_str = serde_json::to_string(&value).unwrap_or_default();
                let escaped = escape_crystal_string(&json_str);
                let ctor_type = if arg.name == "config" {
                    options_type.or(arg.element_type.as_deref())
                } else {
                    arg.element_type.as_deref().or(options_type)
                };
                if let Some(type_name) = ctor_type {
                    call_parts.push(format!("{module_name}::{type_name}.from_json(\"{escaped}\")"));
                } else {
                    call_parts.push(format!("\"{escaped}\""));
                }
            }
            _ => {
                if value.is_null() && arg.optional {
                    call_parts.push("nil".to_string());
                } else {
                    call_parts.push(crystal_lit(value));
                }
            }
        }
    }

    (setup_lines, call_parts.join(", "), teardown_lines)
}

/// Resolve a JSON field path (dot-separated) from the fixture input.
fn resolve_json_field<'a>(value: &'a serde_json::Value, path: &str) -> &'a serde_json::Value {
    let segments: Vec<&str> = path.split('.').filter(|s| !s.is_empty()).collect();
    let mut current = value;
    for seg in segments {
        match current {
            serde_json::Value::Object(obj) => match obj.get(seg) {
                Some(v) => current = v,
                None => return &serde_json::Value::Null,
            },
            _ => return &serde_json::Value::Null,
        }
    }
    current
}

/// Fallback arg builder: original single-arg behavior when no `args` are
/// configured.  A bare string input becomes a single positional string arg;
/// anything else is passed as-is (object/array as JSON string for `from_json`
/// based DTO args), and null becomes no args.
fn call_args_fallback(input: &serde_json::Value) -> String {
    match input {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => crystal_lit(&serde_json::Value::String(s.clone())),
        other => crystal_lit(other),
    }
}

/// Render one assertion as a Crystal `should` expectation on the result var.
fn render_assertion(a: &crate::e2e::fixture::Assertion, result_var: &str) -> String {
    let acc = field_accessor(a.field.as_deref(), result_var);
    match a.assertion_type.as_str() {
        "equals" => match &a.value {
            Some(v) => format!("      {acc}.should eq({})\n", crystal_lit(v)),
            None => "      # equals assertion missing value\n".to_string(),
        },
        "not_empty" => format!("      {acc}.to_s.should_not be_empty\n"),
        "contains" => match &a.value {
            Some(serde_json::Value::String(s)) => format!("      {acc}.to_s.should contain({})\n", string_lit(s)),
            _ => "      # contains assertion requires a string value\n".to_string(),
        },
        "error" => String::new(),
        "contains_all" => match &a.values {
            Some(values) if !values.is_empty() => values
                .iter()
                .map(|v| format!("      {acc}.should contain({})\n", crystal_lit(v)))
                .collect(),
            _ => "      # contains_all assertion requires values\n".to_string(),
        },
        "contains_any" => match &a.values {
            Some(values) if !values.is_empty() => {
                let checks = values
                    .iter()
                    .map(|v| format!("{acc}.includes?({})", crystal_lit(v)))
                    .collect::<Vec<_>>()
                    .join(" || ");
                format!("      ({checks}).should be_true\n")
            }
            _ => "      # contains_any assertion requires values\n".to_string(),
        },
        "is_empty" => format!("      {acc}.to_s.should be_empty\n"),
        "starts_with" => {
            let val = a
                .value
                .as_ref()
                .map(crystal_lit)
                .unwrap_or_else(|| "\"\"".into());
            format!("      {acc}.to_s.should start_with({val})\n")
        }
        "ends_with" => {
            let val = a
                .value
                .as_ref()
                .map(crystal_lit)
                .unwrap_or_else(|| "\"\"".into());
            format!("      {acc}.to_s.should end_with({val})\n")
        }
        "matches_regex" => {
            let val = a
                .value
                .as_ref()
                .map(crystal_lit)
                .unwrap_or_else(|| "\"\"".into());
            format!("      {acc}.to_s.should match({val})\n")
        }
        "greater_than" => {
            let val = a.value.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "0".into());
            format!("      {acc}.should be > {val}\n")
        }
        "less_than" => {
            let val = a.value.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "0".into());
            format!("      {acc}.should be < {val}\n")
        }
        "min_length" => {
            let val = a.value.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "0".into());
            format!("      {acc}.size.should be >= {val}\n")
        }
        "max_length" => {
            let val = a.value.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "0".into());
            format!("      {acc}.size.should be <= {val}\n")
        }
        "count_equals" => {
            let val = a.value.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "0".into());
            format!("      {acc}.size.should eq({val})\n")
        }
        "count_min" => {
            let val = a.value.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "0".into());
            format!("      {acc}.size.should be >= {val}\n")
        }
        "is_true" => format!("      {acc}.should be_true\n"),
        "is_false" => format!("      {acc}.should be_false\n"),
        "not_contains" => match &a.value {
            Some(serde_json::Value::String(s)) => format!("      {acc}.to_s.should_not contain({})\n", string_lit(s)),
            _ => "      # not_contains assertion requires a string value\n".to_string(),
        },
        "method_result" => {
            let method = a.method.as_deref().unwrap_or("(missing_method)");
            let method_args = build_method_args(a.args.as_ref());
            let call = format!("{acc}.{method}{method_args}");
            match a.check.as_deref() {
                Some("equals") => {
                    let val = a.value.as_ref().map(crystal_lit).unwrap_or_else(|| "nil".into());
                    format!("      {call}.should eq({val})\n")
                }
                Some("is_true") => format!("      {call}.should be_true\n"),
                Some("is_false") => format!("      {call}.should be_false\n"),
                Some("greater_than_or_equal") => {
                    let val = a.value.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "0".into());
                    format!("      {call}.should be >= {val}\n")
                }
                Some("count_min") => {
                    let val = a.value.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "0".into());
                    format!("      {call}.size.should be >= {val}\n")
                }
                _ => format!(
                    "      # TODO: unsupported method_result check `{}`\n",
                    a.check.as_deref().unwrap_or("(none)")
                ),
            }
        }
        other => format!("      # TODO: unsupported assertion `{other}`\n"),
    }
}

/// Render an assertion for a void-returning function call. Since there is no
/// result variable, emit the assertion as a Crystal comment noting what
/// would be checked.
fn render_void_assertion(a: &crate::e2e::fixture::Assertion) -> String {
    let msg = match a.assertion_type.as_str() {
        "error" => String::new(),
        "equals" => format!(
            "expects {} to eq {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "not_empty" => format!("expects {} not to be empty", a.field.as_deref().unwrap_or("(result)")),
        "is_empty" => format!("expects {} to be empty", a.field.as_deref().unwrap_or("(result)")),
        "contains" => format!(
            "expects {} to contain {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "starts_with" => format!(
            "expects {} to start with {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "ends_with" => format!(
            "expects {} to end with {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "matches_regex" => format!(
            "expects {} to match {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "greater_than" => format!(
            "expects {} to be > {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "less_than" => format!(
            "expects {} to be < {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "min_length" => format!(
            "expects {} size >= {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "max_length" => format!(
            "expects {} size <= {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "count_equals" => format!(
            "expects {} size == {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "count_min" => format!(
            "expects {} size >= {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "is_true" => format!("expects {} to be true", a.field.as_deref().unwrap_or("(result)")),
        "is_false" => format!("expects {} to be false", a.field.as_deref().unwrap_or("(result)")),
        "not_contains" => format!(
            "expects {} not to contain {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        other => format!("assertion type `{other}`"),
    };
    if msg.is_empty() {
        String::new()
    } else {
        format!("      # void-return: {msg}\n")
    }
}

/// Build the Crystal accessor for an assertion's optional dot-path field.
/// `None` → `{result_var}`; `"meta.title"` → `{result_var}.meta.title` (snake_cased).
fn field_accessor(field: Option<&str>, result_var: &str) -> String {
    use heck::ToSnakeCase;
    match field {
        None => result_var.to_string(),
        Some(path) => {
            let mut acc = result_var.to_string();
            for seg in path.split('.').filter(|s| !s.is_empty()) {
                acc.push('.');
                acc.push_str(&seg.to_snake_case());
            }
            acc
        }
    }
}

/// Render a JSON value as a Crystal literal (string/number/bool/null only).
fn crystal_lit(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => string_lit(s),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "nil".to_string(),
        other => string_lit(&other.to_string()),
    }
}

/// Build a parenthesised Crystal method-call argument list from a JSON array,
/// or empty string for no/non-array args.
fn build_method_args(args: Option<&serde_json::Value>) -> String {
    match args {
        Some(serde_json::Value::Array(arr)) if !arr.is_empty() => {
            let rendered: Vec<String> = arr.iter().map(crystal_lit).collect();
            format!("({})", rendered.join(", "))
        }
        _ => String::new(),
    }
}

fn string_lit(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Escape a string for use inside a Crystal double-quoted string literal.
/// Crystal uses the same escape sequences as C: `\"` for double-quote,
/// `\\` for backslash, `\n` for newline, `\t` for tab.
fn escape_crystal_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── resolve_json_field ─────────────────────────────────────────────

    #[test]
    fn resolve_simple_field() {
        let input = serde_json::json!({"name": "alice"});
        let v = resolve_json_field(&input, "name");
        assert_eq!(v, &serde_json::json!("alice"));
    }

    #[test]
    fn resolve_nested_field() {
        let input = serde_json::json!({"meta": {"title": "hello"}});
        let v = resolve_json_field(&input, "meta.title");
        assert_eq!(v, &serde_json::json!("hello"));
    }

    #[test]
    fn resolve_deeply_nested_field() {
        let input = serde_json::json!({"a": {"b": {"c": 42}}});
        let v = resolve_json_field(&input, "a.b.c");
        assert_eq!(v, &serde_json::json!(42));
    }

    #[test]
    fn resolve_missing_field_returns_null() {
        let input = serde_json::json!({"x": 1});
        let v = resolve_json_field(&input, "y");
        assert_eq!(v, &serde_json::Value::Null);
    }

    #[test]
    fn resolve_path_through_non_object_returns_null() {
        let input = serde_json::json!({"x": "hello"});
        let v = resolve_json_field(&input, "x.y");
        assert_eq!(v, &serde_json::Value::Null);
    }

    #[test]
    fn resolve_empty_path_returns_input() {
        let input = serde_json::json!({"x": 1});
        let v = resolve_json_field(&input, "");
        assert_eq!(v, &input);
    }

    // ── escape_crystal_string ──────────────────────────────────────────

    #[test]
    fn escape_plain_string() {
        assert_eq!(escape_crystal_string("hello"), "hello");
    }

    #[test]
    fn escape_quote() {
        assert_eq!(escape_crystal_string("a\"b"), "a\\\"b");
    }

    #[test]
    fn escape_backslash() {
        assert_eq!(escape_crystal_string("a\\b"), "a\\\\b");
    }

    #[test]
    fn escape_newline() {
        assert_eq!(escape_crystal_string("a\nb"), "a\\nb");
    }

    #[test]
    fn escape_tab() {
        assert_eq!(escape_crystal_string("a\tb"), "a\\tb");
    }

    #[test]
    fn escape_carriage_return() {
        assert_eq!(escape_crystal_string("a\rb"), "a\\rb");
    }

    #[test]
    fn escape_all_specials() {
        assert_eq!(escape_crystal_string("\"\\\n\t\r"), "\\\"\\\\\\n\\t\\r");
    }

    // ── string_lit ─────────────────────────────────────────────────────

    #[test]
    fn string_lit_wraps_in_quotes() {
        assert_eq!(string_lit("hi"), "\"hi\"");
    }

    #[test]
    fn string_lit_escapes_specials() {
        assert_eq!(string_lit("a\"b"), "\"a\\\"b\"");
    }

    // ── crystal_lit ────────────────────────────────────────────────────

    #[test]
    fn crystal_lit_null() {
        assert_eq!(crystal_lit(&serde_json::json!(null)), "nil");
    }

    #[test]
    fn crystal_lit_bool_true() {
        assert_eq!(crystal_lit(&serde_json::json!(true)), "true");
    }

    #[test]
    fn crystal_lit_bool_false() {
        assert_eq!(crystal_lit(&serde_json::json!(false)), "false");
    }

    #[test]
    fn crystal_lit_integer() {
        assert_eq!(crystal_lit(&serde_json::json!(42)), "42");
    }

    #[test]
    fn crystal_lit_float() {
        assert_eq!(crystal_lit(&serde_json::json!(3.14)), "3.14");
    }

    #[test]
    fn crystal_lit_string() {
        assert_eq!(crystal_lit(&serde_json::json!("hello")), "\"hello\"");
    }

    #[test]
    fn crystal_lit_object_falls_back_to_json_string() {
        let lit = crystal_lit(&serde_json::json!({"a": 1}));
        assert!(lit.starts_with('"'));
        assert!(lit.ends_with('"'));
        assert!(lit.contains("\\\"a\\\""));
    }

    #[test]
    fn crystal_lit_array_falls_back_to_json_string() {
        let lit = crystal_lit(&serde_json::json!([1, 2, 3]));
        assert!(lit.starts_with('"'));
        assert!(lit.ends_with('"'));
        assert!(lit.contains("1"));
    }

    // ── field_accessor ─────────────────────────────────────────────────

    #[test]
    fn field_accessor_no_field_uses_result_var() {
        assert_eq!(field_accessor(None, "r"), "r");
    }

    #[test]
    fn field_accessor_single_field() {
        assert_eq!(field_accessor(Some("name"), "res"), "res.name");
    }

    #[test]
    fn field_accessor_nested_path() {
        assert_eq!(field_accessor(Some("meta.title"), "__result"), "__result.meta.title");
    }

    #[test]
    fn field_accessor_pascal_case_to_snake_case() {
        assert_eq!(field_accessor(Some("UserProfile"), "r"), "r.user_profile");
    }

    #[test]
    fn field_accessor_trailing_dot_ignored() {
        assert_eq!(field_accessor(Some("a."), "r"), "r.a");
    }

    #[test]
    fn field_accessor_empty_segments_ignored() {
        assert_eq!(field_accessor(Some("a..b"), "r"), "r.a.b");
    }

    // ── call_args_fallback ─────────────────────────────────────────────

    #[test]
    fn call_args_fallback_null_returns_empty() {
        assert_eq!(call_args_fallback(&serde_json::Value::Null), "");
    }

    #[test]
    fn call_args_fallback_string() {
        assert_eq!(call_args_fallback(&serde_json::json!("hi")), "\"hi\"");
    }

    #[test]
    fn call_args_fallback_number_renders_as_string_literal() {
        let result = call_args_fallback(&serde_json::json!(42));
        assert!(!result.is_empty(), "should not be empty for number input");
    }

    #[test]
    fn call_args_fallback_bool_renders_as_string_literal() {
        let result = call_args_fallback(&serde_json::json!(true));
        assert!(!result.is_empty(), "should not be empty for bool input");
    }

    #[test]
    fn call_args_fallback_array_renders_as_json_string() {
        let result = call_args_fallback(&serde_json::json!([1, 2]));
        assert!(result.starts_with('"'), "result: {result}");
    }

    #[test]
    fn call_args_fallback_object_renders_as_json_string() {
        let result = call_args_fallback(&serde_json::json!({"x": 1}));
        assert!(result.starts_with('"'), "result: {result}");
    }

    // ── build_method_args ──────────────────────────────────────────────

    #[test]
    fn build_method_args_none_returns_empty() {
        assert_eq!(build_method_args(None), "");
    }

    #[test]
    fn build_method_args_empty_array_returns_empty() {
        let arr = serde_json::json!([]);
        assert_eq!(build_method_args(Some(&arr)), "");
    }

    #[test]
    fn build_method_args_single_element() {
        let arr = serde_json::json!([0]);
        assert_eq!(build_method_args(Some(&arr)), "(0)");
    }

    #[test]
    fn build_method_args_multiple_elements() {
        let arr = serde_json::json!([1, "two", true]);
        assert_eq!(build_method_args(Some(&arr)), "(1, \"two\", true)");
    }
}
