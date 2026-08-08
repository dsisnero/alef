//! Crystal e2e test generator (spec-based).
//!
//! Generates a runnable Crystal spec project under `e2e/crystal/` from JSON
//! fixtures: a `shard.yml` with a path dependency on the generated
//! `packages/crystal` binding, a `spec/spec_helper.cr`, and per-category spec
//! files. A smoke spec always links the generated binding so the harness proves
//! the binding compiles and loads.
//!
//! Trait-bridge (plugin) test backends are emitted via [`emit_test_backend`],
//! mirroring the other C-FFI e2e generators.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use heck::{ToPascalCase, ToSnakeCase};
use std::path::PathBuf;

use super::E2eCodegen;

mod http;
mod project;
mod stubs;
#[cfg(test)]
mod tests;

pub use stubs::emit_test_backend;

/// Crystal e2e code generator.
pub struct CrystalE2eCodegen;

impl E2eCodegen for CrystalE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        _type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        let shard_name = config.name.to_snake_case();
        let module_name = config
            .crystal
            .as_ref()
            .and_then(|c| c.module_name.clone())
            .unwrap_or_else(|| config.name.to_pascal_case());

        // Resolve the path to the generated Crystal binding package.
        let pkg = e2e_config.resolve_package("crystal");
        let pkg_path = pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/crystal".to_string());

        // Determine if any active fixture needs the mock server (mock_url /
        // mock_url_list arg types, an input whose value embeds the `$mock_url`
        // placeholder, or a `mock_response` served from the local server).
        // If none do, skip spawning the mock-server binary.
        let needs_mock_server = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
            let cc = e2e_config.resolve_call_for_fixture(
                f.call.as_deref(),
                &f.id,
                &f.resolved_category(),
                &f.tags,
                &f.input,
            );
            cc.args.iter().any(|a| a.arg_type == "mock_url" || a.arg_type == "mock_url_list")
                || super::value_contains_mock_url_placeholder(&f.input)
                || f.mock_response.is_some()
        });

        let mut files = vec![
            GeneratedFile {
                path: output_base.join("shard.yml"),
                content: project::render_shard_yml(&shard_name, &pkg_path),
                generated_header: false,
            },
            GeneratedFile {
                path: output_base.join("spec").join("spec_helper.cr"),
                content: project::render_spec_helper(&shard_name, &e2e_config.env, needs_mock_server),
                generated_header: false,
            },
        ];

        // Per-category spec files for fixtures that resolve for Crystal.
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .collect();
            if active.is_empty() {
                continue;
            }
            let filename = format!("{}_spec.cr", sanitize_filename(&group.category));
            files.push(GeneratedFile {
                path: output_base.join("spec").join(filename),
                content: project::render_category_spec(
                    &group.category,
                    &active,
                    &module_name,
                    e2e_config,
                    &config.trait_bridges,
                    _type_defs,
                ),
                generated_header: true,
            });
        }

        // Always emit a smoke spec that links the generated binding.
        files.push(GeneratedFile {
            path: output_base.join("spec").join("binding_smoke_spec.cr"),
            content: project::render_smoke_spec(&module_name),
            generated_header: true,
        });

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "crystal"
    }
}

