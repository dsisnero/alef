# Plan: Crystal alef-e2e harness + plugin-style trait bridges

Status: **DONE** (all phases A–D landed + verified). Decisions: **(c) both
verification paths**, **both e2e scopes**, **e2e harness first**.

## Outcome

- Phase A ✅ `CrystalE2eCodegen` harness scaffold (shard.yml + spec_helper +
  per-category specs + binding smoke spec), registered, `crystal spec` e2e cmd.
- Phase B ✅ plugin-style (registry) trait bridge codegen — `#[repr(C)]` vtable,
  register/unregister, per-method trampolines, abstract class + register API.
- Phase C ✅ e2e `emit_test_backend` for Crystal (stub subclass + register call).
- Phase D1 ✅ in-repo C-oracle plugin-bridge **link + run** test.
- Phase D2 ✅ full e2e against a real `rustc`-compiled cdylib (Rust CString
  allocator/ABI ↔ Crystal `free_string`, plugin round-trip). **Surfaced and
  fixed a real bug**: String/Char/Path params must pass raw `char*`, not JSON.

Remaining (future): rich fixture→assertion engine in the harness (Phase A emits
`pending` placeholders); wiring `emit_test_backend` into generated specs; HTTP
`TestClientRenderer` path; plugin methods with `Bytes`/`Optional`-complex params.

## Goal

1. A `CrystalE2eCodegen` harness so `alef e2e` generates a runnable Crystal spec
   project (`e2e/crystal/`) from JSON fixtures, and it can be built + run.
2. Plugin-style trait bridges in the Crystal backend (`register_fn`/registry
   pattern), so a foreign Crystal object can implement a Rust trait across the
   C ABI — completing the trait-bridge story (visitor-style already shipped).
3. End-to-end verification that the generated Crystal actually links + runs
   against the real generated FFI (closing the ABI link-parity gap).

## Reference ABI (from `src/backends/ffi/trait_bridge/`)

Plugin bridge, prefix `p`, trait `T` (PascalCase prefix `Pp`):

- **VTable** `#[repr(C)] {Pp}{T}VTable`:
  - super-trait (Plugin) method fn-pointers: `name`, `version`, `initialize`, `shutdown`
  - one `Option<extern "C" fn(...)>` per own trait method
  - `free_user_data: Option<extern "C" fn(*const c_void)>`
  - `free_string: Option<extern "C" fn(*mut c_char)>`  (register() null-checks this)
- **Method thunk**: `fn(user_data: *const c_void, <params>, out_result?: *mut *mut c_char, out_error?: *mut *mut c_char) -> i32`
  - params: scalars by value; String/Named/Vec/Map/Json → `*const c_char` (JSON); `Bytes` → `*const u8` + `_len: usize`
  - `out_result` present for complex/String returns; `out_error` for fallible or complex; returns i32 status (0 = ok)
- **Constructor/registration** (all `{p}_` prefixed):
  - `{p}_{bridge_snake}_new(vtable: *const VTable, user_data: *const c_void) -> *mut Bridge`  (bridge_snake = snake(`{T}Bridge`))
  - `{p}_{bridge_snake}_free(*mut Bridge)`
  - `{p}_{register_fn}(name: *const c_char, vtable: *const VTable, user_data: *const c_void, out_error: *mut *mut c_char) -> i32`
  - `{p}_unregister_{trait_snake}(name: *const c_char) -> i32` (confirm sig)
- `ffi_set_out_error` writes a malloc'd error string; caller frees via `free_string`.

Crystal mapping: real C function pointers via closure-free `->(...){}`, `Box(T)`
for `user_data`, malloc'd copies for `out_result`/`out_error`, JSON marshalling
for complex params/returns. Struct **layout** parity is what matters (Crystal lib
struct field order must match the `#[repr(C)]` vtable); `fun` symbol names reuse
the FFI `{p}_...` formulas verbatim.

## alef-e2e reference (from `src/e2e/`)

- `E2eCodegen` trait (`codegen/mod.rs`): `generate(groups, e2e_config, config, type_defs, enums)` + `language_name()`; registered in `all_generators()`, filtered by `default_e2e_languages()`.
- Trait-bridge test stubs: `emit_test_backend(language, trait_bridge, methods, fixture) -> TestBackendEmission{setup_block, arg_expr, teardown_block}` (Crystal currently → `unimplemented`).
- Closest template: `codegen/zig/{mod,build,test_file,stubs,visitor}.rs`.
- HTTP DRY path: `codegen/client::{TestClientRenderer, http_call::render_http_test}`.
- Crystal `TestConfig` (`core/config/test_defaults.rs:262`) currently sets `e2e: None` → must add the e2e run command (`crystal spec` in the e2e dir).

## Phases (execution order: e2e first)

### Phase A — CrystalE2eCodegen harness (`src/e2e/codegen/crystal/`)
- `mod.rs`: `CrystalE2eCodegen` impl `E2eCodegen::generate` →
  - `e2e/crystal/shard.yml` (path dep on `../../packages/crystal`), `spec/spec_helper.cr`.
  - `spec/<crate>_spec.cr`: per-fixture Crystal specs.
    - function-call fixtures → call the generated wrapper, assert on result (`assertion_recipes`, field access).
    - HTTP/mock fixtures → implement `TestClientRenderer` for Crystal (`client.rs`) and drive `render_http_test` (HTTP client via `crest`/stdlib `HTTP::Client`, hitting `MOCK_SERVER_URL`).
- Register `CrystalE2eCodegen` in `all_generators()`; ensure `default_e2e_languages` maps `Language::Crystal` (it does, via `other => other.to_string()`).
- Add `e2e: Some("cd {dir} && crystal spec")` to Crystal `TestConfig`.
- Tests: unit test asserting generated spec structure; snapshot.

### Phase B — Backend plugin-style trait bridge (`src/backends/crystal/`)
- `trait_bridge.rs::gen_plugin_bridge`: emit VTable lib struct (super-trait + per-method fn-pointers + free_user_data + free_string), `{bridge_snake}_new/_free`, `register`/`unregister` lib funs; per-method trampolines (Box user_data, JSON/scalar decode, out_result/out_error malloc, i32 status); high-level abstract `{T}` class + `register_<trait>(name, impl)` / `unregister_<trait>(name)`.
- `gen_bindings/mod.rs`: replace the plugin-bridge rejection with generation (keep guards for unsupported shapes: e.g. `Bytes` params deferred).
- Tests: gen_bindings assertions + `crystal build --no-codegen` typecheck (compile suite) + snapshot.

### Phase C — E2e emit_test_backend for Crystal (`src/e2e/codegen/crystal/stubs.rs`)
- Mirror `zig/stubs.rs`: stub class implementing all methods (super-trait `name`→`"test"`, `version`→`"0.0.1"`), build vtable, `register_<trait>("test", stub)` `arg_expr` + `unregister` teardown. Wire into `emit_test_backend` dispatch in `codegen/mod.rs`.

### Phase D — Verification (both)
- **D1 (in-repo, no cargo):** extend `tests/backends_crystal_compile_test.rs` with a plugin-bridge C-oracle test — a C shim implementing `demo_register_<trait>`/`_new`/`_free` that invokes vtable fn-pointers; `crystal build` (full link) + run; assert the Crystal impl's method is called through the vtable.
- **D2 (CI-grade, gated):** an integration test that runs `alef` end-to-end on a tiny core crate → generate FFI + Crystal → `cargo build` the ffi cdylib → `crystal spec` links + runs a registered plugin bridge. Gated on `cargo`+`crystal`; skips otherwise.

## Verification gates (every phase)
`crystal build --no-codegen` typecheck; `cargo test --test backends_crystal_*`;
`cargo clippy --lib`; `cargo fmt`. Commit per phase (conventional commits).

## Open confirmations during impl
- Exact `unregister_{trait_snake}` signature + whether it takes `name`.
- Full vtable field set/order (confirm `free_string` + `free_user_data` positions from `vtable_*` jinja) — layout parity is load-bearing.
- HTTP e2e client lib choice for Crystal (stdlib `HTTP::Client` vs a shard).
