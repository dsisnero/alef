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
        out.push_str(&format!("      __result = {module_name}.{function_name}({args})\n"));
        for a in &fixture.assertions {
            out.push_str(&render_assertion(a));
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
    match a.assertion_type.as_str() {
        "equals" => match &a.value {
            Some(v) => format!("      __result.should eq({})\n", crystal_lit(v)),
            None => "      # equals assertion missing value\n".to_string(),
        },
        "not_empty" => "      __result.to_s.should_not be_empty\n".to_string(),
        "contains" => match &a.value {
            Some(serde_json::Value::String(s)) => format!("      __result.to_s.should contain({})\n", string_lit(s)),
            _ => "      # contains assertion requires a string value\n".to_string(),
        },
        other => format!("      # TODO: unsupported assertion `{other}`\n"),
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

/// Render a Crystal double-quoted string literal with minimal escaping.
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
