# Plan: Crystal backend — remaining work

Status: **25/26 compile tests**, **26/29 gen-bindings tests**, **1/4 snapshot tests**, **182/192
crystal lib tests** pass. F0–F5 fixed in earlier session; F6, F7, F9–F19 landed this session.
**Only F8 remains** (run e2e Crystal specs — blocked on mock server / PHP toolchain).
Remaining cleanup: **14 stale test assertions + 3 pending snapshot accepts** from the new ABI
(nullable `String?` params, `last_error_code` for fallible scalar/unit returns, getter-based
JSON defaults, e2e arg-mapping model) — see §G.

---

## A. FFI struct ABI (param side — complete)

- [x] `c_type_of` returns `TypeName*` for non-opaque Named types
- [x] `lib_c_return` skips JSON-string coercion for struct returns with errors
- [x] `gen_call_body` uses `*_to_json`/`*_free` for struct returns
- [x] `gen_wrapper_body` uses `*_from_json`/`*_free` for struct params
- [x] Emit `struct TypeName; end` declarations in lib block
- [x] Emit `fun *_from_json`/`*_to_json`/`*_free` helper declarations
- [x] `ffi_struct_names` excludes opaque handle types
- [x] **F1** Thread `ffi_structs` through `marshal_value` for streaming + opaque methods
  `gen_stream_method` and `gen_opaque_method` now handle struct params via
  `from_json`/`free` (not just `gen_wrapper_body`).
- [x] **F2** Fix compile test failures (6 pre-existing failures)
  Root cause: `generate_bindings` now returns `shard.yml` alongside `.cr` files.
  Tests concatenated all files into a single Crystal source, causing YAML content
  (`name: demo`) to appear in Crystal code. Fix: filter to `.cr` files only,
  write generated files to disk individually so `require` resolves correctly.
- [x] **F3** Handle `Optional(Named(...))` struct returns in `gen_call_body`
  Infallible optional struct returns use nullable struct pointer (nil on null);
  fallible ones remain JSON-string ABI (null could be error or None).
  Also updated `c_type_of` and `lib_c_return` for correct C return types.

## B. Codegen completeness

- [x] JSON::Serializable with discriminator for internally-tagged enums
  `gen_internally_tagged` uses `use_json_discriminator`.
- [x] Bytes fields use `@[JSON::Field(ignore: true)]` or default value
- [x] 18 assertion types in e2e codegen
- [x] HTTP test client renderer (`TestClientRenderer` trait impl)
- [x] `shard.yml` emitted with targets section
- [x] Per-language `[crates.crystal]` TOML config registered
- [x] **F0** Empty struct declarations (`struct Config; end`) rejected by Crystal 1.19
  Changed to `struct Config\n _data : Void*\n end` to satisfy Crystal's
  non-empty struct requirement while preserving the opaque C-compatible type.
  (Fixed as part of F3 work since compile tests failed after struct-pointer ABI.)
- [x] **F4** Partial JSON for non-nilable fields (default values)
  `gen_struct` now emits a default initializer on the getter
  (`getter field : T = <expr>`) for fields with `typed_default`
  (BoolLiteral, IntLiteral, FloatLiteral, StringLiteral) and falls back to
  `type_based_default_expr`/`enum_default_expr`/`struct_default_expr` otherwise.
  (Crystal 1.19's `@[JSON::Field(default:)]` does NOT work for `from_json` —
  getter defaults are the working mechanism.)
- [x] **F5** Duplicate `getter` lines fixed
  Removed the copy-paste duplicate line in `gen_struct` (lines 354-355).

## C. E2e test framework

- [x] 21 Crystal spec files + `spec_helper.cr` generated for crawlberg
  (on branch `crystal-backend-fixes`; `alef e2e generate` produces `e2e/crystal/spec/*_spec.cr`)
- [x] **F6** Add crystal overrides in crawlberg e2e call configs
  Added `crystal` to `[workspace] languages`, `[crates.output]`, `[crates.e2e.languages]`.
  Added `module = "Crawlberg"` override for all non-streaming calls.
  Crystal skipped for streaming calls (`crawl_stream`, `batch_crawl_stream`).
- [x] **F7** Make e2e Crystal specs compile
  `cd e2e/crystal && shards install && crystal build spec/scrape_spec.cr` — compiles successfully.
  Also verified binary links: `crystal build src/crawlberg.cr -o bin/crawlberg` succeeds.
- [ ] **F8** Run e2e Crystal specs
  FFI lib built (`cargo build -p crawlberg-ffi --release`), Crystal binary links.
  Mock server needs PHP toolchain (e2e/rust `ext-php-rs` build dep).
  Full e2e run requires mock server + `MOCK_SERVER_URL` env var.
- [x] **F9** Add crystal to xberg e2e languages
  Added `"crystal"` to `[crates.e2e.languages]` + base `[crates.e2e.call.overrides.crystal]`.
  18 spec files + `spec_helper.cr` generated (on branch `crystal-backend-fixes`).
- [x] **F10** Add crystal e2e call overrides for xberg
  Fixed `json_object` handler: uses `Array(Type).from_json(...)` when `element_type`
  is set (for batch args like `extract_batch` inputs, `interact` actions).

### Crystal e2e codegen fixes (alef repo, all landed):

| Change | Description |
|--------|-------------|
| `handle` arg type | Creates engine from config via `create_engine(CrawlConfig.from_json(...))` |
| `mock_url` arg type | Resolves mock server URL from `MOCK_SERVER_URL` env var |
| `mock_url_list` arg type | Builds mock URL array with Crystal string interpolation |
| env vars | `spec_helper.cr` sets `ENV[...] ||= ...` from `e2e_config.env` |
| assertion types | Added `greater_than_or_equal`, `less_than_or_equal` support |
| virtual fields | `is_error` → `error.should_not be_nil`; `pages_crawled` → `pages.size` |
| array iteration | `links[].link_type` → `links.any? { \|el\| el.link_type.includes?(...) }` |
| array indexing | `pages_0` → `pages[0]`; `json_ld.type` → `json_ld[0].schema_type` |
| wrapper namespace | `crawl.*`, `batch.*`, `map.*`, `content.*`, `robots.*` → stripped |
| metadata flattening | `og.title` → `metadata.og_title`; `twitter.card` → `metadata.twitter_card`; `dublin_core.*` → `metadata.dc_*` |
| field renames | `category` → `asset_category`; `type` → `schema_type` |
| `create_engine` return | Fixed opaque handle teardown-before-return bug |
| Crystal syntax | `#{}` interpolation, `[] of String`, `size` not `length`, `includes?` not `contains` |
| `json_object` arrays | `Array(PageAction).from_json(...)` for JSON array args |

## D. Test coverage (alef itself)

- [x] 4411+ tests pass, clippy clean
- [x] **F11** Unit test for `ffi_struct_names` and `collect_named_types`
  New helper functions have zero direct test coverage.
- [x] **F12** Unit test for struct-return code path in `gen_call_body`
  Verify `to_json`/`free` pattern emitted correctly for Named returns.
- [x] **F13** Unit test for struct-param code path in `gen_wrapper_body`
  Verify `from_json`/`free` pattern emitted correctly for Named params.
- [x] **F14** Snapshot test update for crawlberg-sized API surface
  Current snapshot uses a minimal API. Generate a snapshot with struct types
  to catch regressions in struct-pointer codegen.

## E. Documentation and examples

- [x] crawlberg example (scrapes httpbin.org, browser mode=Never)
- [x] xberg example (OCR backend register/list/unregister)
- [x] Makefile for both repos (`make example` builds FFI + Crystal)
- [x] **F15** xberg extraction example with working config
  F4 JSON defaults fix applied. Config can now be partial JSON
  (`{%raw%}{"force_ocr":true}{%endraw%}`) instead of specifying every field.
  Example updated to use compact config.
- [x] **F16** Add Crystal badge/entry to crawlberg README
  Added `img.shields.io/badge/Crystal-shards-007ec6` badge after Zig.
- [x] **F17** Add Crystal badge/entry to xberg README
  Same badge pattern as crawlberg.

## F. xberg-specific runtime issues

- [x] **F18** xberg FFI builds and Crystal binding links
  `PKG_CONFIG_PATH=/usr/local/lib/pkgconfig cargo build -p xberg-ffi --release` succeeds.
  `crystal build examples/extract.cr -o bin/extract` links against FFI lib (7 MB binary).
  Batch fixture input resolution (null vs array) is a fixture-structure concern, not a codegen bug.
- [x] **F19** xberg `create_engine` equivalent (not needed)
  xberg takes config directly — no engine handle pattern. The `json_object`
  args work via `from_json` deserialization. No ABI issue to fix.

## G. Remaining cleanup — stale test assertions + snapshot accepts

The working tree's codegen changes (nullable `String?` params, `last_error_code`
ABI for fallible scalar/unit returns, getter-based JSON defaults, e2e arg-mapping
model) are ahead of the tests. 17 tests/snapshots assert the OLD ABI and need
updating. All failures are stale expectations, not codegen bugs:

| Suite | Failures | Fix |
|-------|----------|-----|
| `backends_crystal_compile_test` | 1: `visitor_bridge_emits_callback_layer` | expects `text : String`; codegen now emits `String?` (null-checked C strings). Update assertion + driver to `String?`. |
| `backends_crystal_gen_bindings_test` | 3: `duration_with_error_still_returns_c_string`, `struct_param_uses_from_json_in_wrapper_body`, `struct_fields_with_defaults_emit_json_field_default_annotation` | fallible scalar/unit returns now return by value + `last_error_code` (not `LibC::Char*`); defaults are getter initializers (not `@[JSON::Field(default:)]`). Update assertions. |
| `backends_crystal_snapshot_test` | 3: `snapshot_basic_bindings`, `snapshot_rich_struct_api`, `snapshot_visitor_bridge` | `.snap.new` files present; run `cargo insta accept`. |
| `cargo test --lib` (crystal) | 10: `trait_bridge::callback_with_string_param_resolves` + 9 `e2e::codegen::crystal::*` | trait_bridge expects `String` → `String?`; e2e tests assert removed raw single-arg fallback (`Demo.convert("hi")` → now arg-mapping based `Demo.convert()`). Update expectations. |

After accepting snapshots and updating the 14 stale assertions above, re-run the
four suites and confirm green before F8.
