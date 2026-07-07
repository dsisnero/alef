//! Crystal e2e trait-bridge (plugin) test-backend stub emission.
//!
//! Filled in by Phase C once the Crystal backend emits the plugin-bridge
//! `register_<trait>` API. Until then, return an unimplemented marker so the
//! e2e dispatch is wired but produces a clear placeholder.

use crate::e2e::codegen::TestBackendEmission;

/// Emit a Crystal test backend stub for a trait bridge (plugin) fixture.
pub fn emit_test_backend(
    _trait_bridge: &crate::core::config::TraitBridgeConfig,
    _methods: &[&crate::core::ir::MethodDef],
    _fixture: &crate::e2e::fixture::Fixture,
) -> TestBackendEmission {
    TestBackendEmission::unimplemented("crystal")
}
