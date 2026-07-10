# Plan: Crystal backend — remaining work

Status: **IN PROGRESS** — core binding generation + C FFI ABI fixed.
Commit: `bc9d84a` — from_json/free helpers for struct params.

---

## A. FFI struct ABI (param side — complete)

- [x] `c_type_of` returns `TypeName*` for non-opaque Named types
- [x] `lib_c_return` skips JSON-string coercion for struct returns with errors
- [x] `gen_call_body` uses `*_to_json`/`*_free` for struct returns
- [x] `gen_wrapper_body` uses `*_from_json`/`*_free` for struct params
- [x] Emit `struct TypeName; end` declarations in lib block
- [x] Emit `fun *_from_json`/`*_to_json`/`*_free` helper declarations
- [x] `ffi_struct_names` excludes opaque handle types
- [ ] **F1** Thread `ffi_structs` through `marshal_value` for streaming method args
  `gen_stream_method` line 416 calls `marshal_value(..., &HashSet::new())` —
  struct params in streaming adapters still JSON-encode. Need to pass `ffi_structs`
  and use `from_json`/`free` pattern.
- [ ] **F2** Verify xberg bindings regenerate with correct ABI
  xberg's `extract`/`extract_batch` take `ExtractInput`/`ExtractionConfig` params
  that need the same struct-pointer treatment. Regenerate and test extraction.
- [ ] **F3** Handle `Optional(Named(...))` struct returns in `gen_call_body`
  Currently only matches `TypeRef::Named(n)` directly. `Optional(ScrapeResult)`
  would fall through to C-string path.

## B. Codegen completeness

- [x] JSON::Serializable with discriminator for internally-tagged enums
  `gen_internally_tagged` uses `use_json_discriminator`.
- [x] Bytes fields use `@[JSON::Field(ignore: true)]` or default value
- [x] 18 assertion types in e2e codegen
- [x] HTTP test client renderer (`TestClientRenderer` trait impl)
- [x] `shard.yml` emitted with targets section
- [x] Per-language `[crates.crystal]` TOML config registered
- [ ] **F4** `CrawlConfig.from_json(...)` requires ALL non-nilable fields
  Go uses `json.Unmarshal` with partial JSON → zero-value defaults. Crystal
  `JSON::Serializable` has no equivalent. Options:
  - Add `def self.default : CrawlConfig` constructors using field defaults
  - Add `@[JSON::Field(default: ...)]` for each field based on Rust `Default`
  - Or provide a convenience constructor that accepts partial JSON
- [ ] **F5** Audit generated code for duplicate `getter` lines
  The `gen_struct` function emits each field twice (visible in generated output).
  Causes `to_json` to duplicate every key. Root cause: `gen_struct` is called
  per type and each field generates `getter` twice.

## C. E2e test framework

- [x] 32 Crystal spec files generated for crawlberg
  (`alef e2e generate` produces `e2e/crystal/spec/*_spec.cr`)
- [ ] **F6** Add crystal overrides in crawlberg e2e call configs
  The e2e tests call `Crawlberg.scrape(json_config, nil)` but bindings have
  `scrape(engine : CrawlEngineHandle, url : String)`. Need per-call overrides:
  ```toml
  [crates.e2e.calls.scrape.overrides.crystal]
  function = "scrape"
  module = "Crawlberg"
  ```
  And update `args` to map fixture fields to binding params.
- [ ] **F7** Make e2e Crystal specs compile
  `cd e2e/crystal && shards install && crystal spec` — the spec files need to
  `require` the crawlberg binding. Currently the e2e `shard.yml` points to
  a package path that may not resolve. Need to verify/fix the dependency path.
- [ ] **F8** Run e2e Crystal specs and fix failures
  Compare output with Go/Zig e2e results. Likely issues:
  - Config passing mismatch (JSON string vs typed struct)
  - Result field accessor naming (snake_case in Crystal)
  - Mock server URL resolution
- [ ] **F9** Add crystal to xberg e2e languages
  Same as F6 but for xberg repo.
- [ ] **F10** Add crystal e2e call overrides for xberg
  Map xberg fixture fields (extraction, batch, OCR) to Crystal binding params.

## D. Test coverage (alef itself)

- [x] 4411 tests pass, clippy clean
- [ ] **F11** Unit test for `ffi_struct_names` and `collect_named_types`
  New helper functions have zero direct test coverage.
- [ ] **F12** Unit test for struct-return code path in `gen_call_body`
  Verify `to_json`/`free` pattern emitted correctly for Named returns.
- [ ] **F13** Unit test for struct-param code path in `gen_wrapper_body`
  Verify `from_json`/`free` pattern emitted correctly for Named params.
- [ ] **F14** Snapshot test update for crawlberg-sized API surface
  Current snapshot uses a minimal API. Generate a snapshot with struct types
  to catch regressions in struct-pointer codegen.

## E. Documentation and examples

- [x] crawlberg example (scrapes httpbin.org, browser mode=Never)
- [x] xberg example (OCR backend register/list/unregister)
- [x] Makefile for both repos (`make example` builds FFI + Crystal)
- [ ] **F15** xberg extraction example with working config
  Deeply nested config types make `from_json` unusable (see F4). Need a
  working extraction example once config defaults are available.
- [ ] **F16** Add Crystal badge/entry to crawlberg README
  Match the pattern of other language badges.
- [ ] **F17** Add Crystal badge/entry to xberg README

## F. xberg-specific runtime issues

- [ ] **F18** Test xberg extraction after ABI fix regeneration
  The segfault in `xberg_extract` was likely caused by the JSON-string vs
  struct-pointer mismatch (same as crawlberg). Regenerate with fix and retest.
- [ ] **F19** xberg `create_engine` equivalent
  xberg may not have a `create_engine` pattern — its functions take config
  directly. Verify correct ABI after regeneration.
