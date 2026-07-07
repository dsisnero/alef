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

/// Render a per-category spec file. Each fixture becomes an `it` example. Until
/// the full fixture→assertion engine lands, examples are pending placeholders
/// that document the fixture; the smoke spec guarantees the suite links + runs.
pub(super) fn render_category_spec(category: &str, fixtures: &[&Fixture], module_name: &str) -> String {
    let mut out = String::from("require \"./spec_helper\"\n\n");
    out.push_str(&format!("describe {module_name} do\n"));
    out.push_str(&format!("  describe {category:?} do\n"));
    for fixture in fixtures {
        let desc = if fixture.description.is_empty() {
            &fixture.id
        } else {
            &fixture.description
        };
        out.push_str(&format!("    pending {desc:?}\n"));
    }
    out.push_str("  end\nend\n");
    out
}
