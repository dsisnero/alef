# Plan: Crystal alef-e2e harness + plugin-style trait bridges

Status: **DONE** (all phases A–D + e2e assertion engine landed and verified).

## Outcome

- **Phase A** ✅ `CrystalE2eCodegen` harness scaffold (shard.yml + spec_helper +
  per-category specs + binding smoke spec), registered, `crystal spec` e2e cmd.
- **Phase B** ✅ plugin-style (registry) trait bridge codegen — `#[repr(C)]` vtable,
  register/unregister, per-method trampolines, abstract class + register API.
- **Phase C** ✅ e2e `emit_test_backend` for Crystal (stub subclass + register call).
- **Phase D1** ✅ in-repo C-oracle plugin-bridge **link + run** test.
- **Phase D2** ✅ full e2e against a real `rustc`-compiled cdylib (Rust CString
  allocator/ABI ↔ Crystal `free_string`, plugin round-trip). **Surfaced and
  fixed a real bug**: String/Char/Path params must pass raw `char*`, not JSON.
- **E2e assertion engine** ✅ real function-call specs with field-path support:
  `equals`, `not_empty`, `contains`, `contains_all`, `contains_any`,
  `method_result` (is_true/is_false/equals/gte/count_min), `error`
  (expect_raises). Fixtures without assertions stay `pending`.

**Verification**: every commit type-checked via `crystal build --no-codegen`;
the plugin bridge and function return ABI validated at **runtime** against a
real C oracle and a real Rust cdylib. All 41 Crystal integration tests pass.

## Commits

| Commit | Description |
|---|---|
| `1c98956` | feat(crystal): register Crystal as a language target |
| `654bb6e` | feat(crystal): add the Crystal binding backend |
| `6627bc3` | feat(crystal): scaffold Crystal packages |
| `bc62ce4` | test(crystal): generation, snapshot, docs & compiler-verification suites |
| `230e9b4` | feat(crystal): support free-function (owner-less) streaming |
| `30bb7e7` | test(crystal): end-to-end FFI link + run against a C ABI oracle |
| `0f53953` | chore: ignore macOS AppleDouble (._*) metadata files |
| `a22373b` | docs(crystal): list Crystal in the supported targets table |
| `13add6b` | docs(crystal): plan for e2e harness + plugin-style trait bridges |
| `cd969c1` | feat(crystal): add e2e harness scaffold (CrystalE2eCodegen) |
| `d11cea7` | feat(crystal): plugin-style trait bridges (registry pattern) |
| `6501934` | feat(crystal): e2e test-backend stub for plugin trait bridges |
| `2fa636b` | test(crystal): runtime link+run validation of plugin trait bridge |
| `5ccf5da` | fix(crystal): pass String/Char/Path params raw across the FFI |
| `44a10e4` | feat(crystal): real function-call e2e specs (assertions) |
| `3b99445` | feat(crystal): field-path and error assertions in e2e specs |
| `88a67d4` | feat(crystal): contains_all/contains_any e2e assertions |
| `8c4922e` | feat(crystal): method_result e2e assertion type |
| `5aa282a` | docs(crystal): mark trait-bridge/e2e plan complete |
| `3d34af9` | docs(crystal): final plan update + summary |

## Remaining (documented, future work)

- **Multi-arg / DTO-constructor arg resolution** (full `CallConfig.args` engine).
- **Wiring `emit_test_backend`** into generated specs so trait-bridge fixtures
  produce real examples.
- **HTTP `TestClientRenderer`** path for mock-server e2e fixtures.
- **Plugin methods with `Bytes`/`Optional`-complex params**.

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
  - `{p}_{bridge_snake}_new(vtable: *const VTable, user_data: *const c_void) -> *mut Bridge`
  - `{p}_{bridge_snake}_free(*mut Bridge)`
  - `{p}_{register_fn}(name: *const c_char, vtable: *const VTable, user_data: *const c_void, out_error: *mut *mut c_char) -> i32`
  - `{p}_unregister_{trait_snake}(name: *const c_char, out_error: *mut *mut c_char) -> i32`

Crystal mapping: real C function pointers via closure-free `->(...){}`, `Box(T)`
for `user_data`, malloc'd copies for `out_result`/`out_error`, JSON marshalling
for complex params/returns. Struct **layout** parity is what matters (Crystal lib
struct field order must match the `#[repr(C)]` vtable); `fun` symbol names reuse
the FFI `{p}_...` formulas verbatim.

## Verification gates (every phase)

`crystal build --no-codegen` typecheck; `cargo test --test backends_crystal_*`;
`cargo clippy --lib`; `cargo fmt`. Commit per phase (conventional commits).

Note: the external-SSD `target/` doesn't support hardlinks, so cargo copies the
incremental cache on every rebuild (~15-20 min). Set `CARGO_INCREMENTAL=0` to
avoid that copy and build faster.
