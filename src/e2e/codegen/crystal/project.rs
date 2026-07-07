//! Crystal e2e project file rendering: shard.yml, spec_helper, and specs.

use crate::e2e::fixture::Fixture;

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
    function_name: &str,
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
        if fixture.assertions.is_empty() {
            out.push_str(&format!("    pending {desc:?}\n"));
            continue;
        }
        out.push_str(&format!("    it {desc:?} do\n"));
        let args = call_args(&fixture.input);
        let call = format!("{module_name}.{function_name}({args})");
        // A fixture with an `error` assertion expects the call itself to raise.
        if fixture.assertions.iter().any(|a| a.assertion_type == "error") {
            out.push_str("      expect_raises(Exception) do\n");
            out.push_str(&format!("        {call}\n"));
            out.push_str("      end\n");
        } else {
            out.push_str(&format!("      __result = {call}\n"));
            for a in &fixture.assertions {
                out.push_str(&render_assertion(a));
            }
        }
        out.push_str("    end\n");
    }
    out.push_str("  end\nend\n");
    out
}

/// Build the Crystal call-argument list from a fixture's input. A bare string
/// input becomes a single positional string arg; anything else is passed as-is
/// (object/array as JSON via the DTO's `from_json`), and null becomes no args.
fn call_args(input: &serde_json::Value) -> String {
    match input {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => crystal_lit(&serde_json::Value::String(s.clone())),
        other => crystal_lit(other),
    }
}

/// Render one assertion as a Crystal `should` expectation on `__result`.
fn render_assertion(a: &crate::e2e::fixture::Assertion) -> String {
    let acc = field_accessor(a.field.as_deref());
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
        // `error` is handled at the block level (expect_raises).
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
        "method_result" => {
            let method = a.method.as_deref().unwrap_or("(missing_method)");
            let method_args = build_method_args(a.args.as_ref());
            let call = format!("{acc}.{method}{method_args}");
            match a.check.as_deref() {
                Some("equals") => {
                    let val = a.value.as_ref().map(|v| crystal_lit(v)).unwrap_or_else(|| "nil".into());
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

/// Build the Crystal accessor for an assertion's optional dot-path field.
/// `None` → `__result`; `"meta.title"` → `__result.meta.title` (snake_cased).
fn field_accessor(field: Option<&str>) -> String {
    use heck::ToSnakeCase;
    match field {
        None => "__result".to_string(),
        Some(path) => {
            let mut acc = String::from("__result");
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
        // Arrays/objects are passed as their JSON text for `from_json`-based DTO args.
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
