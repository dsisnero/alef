# Plan: Crystal backend — remaining work

Status: **26/26 compile tests**, **29/29 gen-bindings tests**, **4/4 snapshot tests**, **192/192
crystal lib tests**, **4618/4618 total lib tests** pass. **All e2e fixtures run in every
companion repo — nothing hard-skipped:**

| Repo | Suite | Result |
|------|-------|--------|
| crawlberg | `e2e/crystal` | 256 / 0 failures / 0 errors / 1 pending (fixture with no assertions) |
| xberg | `e2e/crystal` | 71 / 0 failures / 0 errors / 1 pending (LLM runtime skip) |
| liter-llm | `e2e/crystal` | 166 / 0 failures / 0 errors / 2 pending (local Ollama runtime skips) |
| html-to-markdown | `e2e/crystal` | 273 / 0 failures / 0 errors / 0 pending |
| tree-sitter-language-pack | `e2e/crystal` | 522 / 0 failures / 0 errors / 0 pending |

The `crystal-all-fixtures` branches in alef + all five companion repos carry this work;
all lib/integration/e2e suites are green. F0–F5 fixed in earlier session; F6, F7, F9–F19
landed in that session; the **all-fixtures expansion** (streaming un-skipped, HTTP fixtures,
liter-llm/html-to-markdown/tree-sitter-language-pack ports, visitor bridges) landed on
`crystal-all-fixtures`.

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
  Streaming calls (`crawl_stream`, `batch_crawl_stream`) were later un-skipped on
  `crystal-all-fixtures` (streaming e2e via `engine.<fn>(RequestType.from_json(...))`).
- [x] **F7** Make e2e Crystal specs compile
  `cd e2e/crystal && shards install && crystal build spec/scrape_spec.cr` — compiles successfully.
  Also verified binary links: `crystal build src/crawlberg.cr -o bin/crawlberg` succeeds.
- [x] **F8** Run e2e Crystal specs
  `cd e2e/crystal && crystal spec` — **193 examples, 0 failures, 0 errors, 21 pending** at the
  time; on `crystal-all-fixtures` all fixtures run (256 examples, 1 pending = no-assertions fixture).
  Blockers fixed this session:
  - Mock server lifecycle: spawning at spec_helper load time left the child dead under
    `crystal spec`; moved to a `Spec.before_suite` `AlefMockServer` singleton holding
    pid/reader in instance vars, draining the pipe in a fiber, and tearing down in
    `Spec.after_suite`. Also parse the `MOCK_SERVERS={...}` line and export
    `MOCK_SERVER_<FIXTURE_UPPER>` env vars for origin-root fixtures.
  - Enum converters: unit enums with serde wire values differing from Crystal variant
    names (e.g. `"og:image"` → `OgImage`) now emit a `XxxConverter` module wired via
    `@[JSON::Field(converter: ...)]`.
  - Array `contains`/`not_contains` on enum fields use `.downcase` so PascalCase
    `.to_s` matches lowercase fixture values.
  - Config-validation fixtures wrap engine setup + call inside `expect_raises`.
  - `is_error` assertions skipped (matches Go/Rust/Python/Dart parity).
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

## G. Stale test assertions + snapshot accepts (fixed this session)

The working-tree codegen changes (nullable `String?` params, `last_error_code`
ABI for fallible scalar/unit returns, getter-based JSON defaults, e2e arg-mapping
model) were ahead of the tests, which asserted the OLD ABI. All 17
tests/snapshots were updated to the new ABI in the same session:

| Suite | Before | After |
|-------|--------|-------|
| `backends_crystal_compile_test` | 25/26 | 26/26 |
| `backends_crystal_gen_bindings_test` | 26/29 | 29/29 |
| `backends_crystal_snapshot_test` | 1/4 | 4/4 (3 accepts) |
| `cargo test --lib` (crystal) | 182/192 | 192/192 |
| `cargo test --lib` (all) | — | 4417/4417 |

Notable expectation changes:
- visitor callbacks use `String?` params (null-checked C strings)
- fallible scalar/unit returns are by-value + `last_error_code`, not `LibC::Char*`
- struct-field defaults are getter initializers, not `@[JSON::Field(default:)]`
- e2e calls pass no positional arg when no `args` mappings are configured;
  string comparisons render as `.to_s.strip`
- adapters signal errors via `set_last_error`

After re-running the four suites green plus the crawlberg e2e suite, **all F-items are complete**.

## H. e2e Crystal specs across companion repos (all green — nothing hard-skipped)

The all-fixtures expansion removed the category skip
(engine/rate_limit/markdown/filter/strategy/metadata/interaction/download) and the
`crystal_effectively_skipped` rule from `src/e2e/codegen/crystal/mod.rs`, and un-skipped
streaming in crawlberg (`crawl_stream`/`batch_crawl_stream`). Every fixture now generates a
Crystal test; runtime-only skips (documented in fixtures) stay pending when their
prerequisite service/key is absent. See the status table at the top for per-repo totals.

Streaming e2e: `engine.<fn>(RequestType.from_json(...))` instance calls, Channel consumed
via `receive?`, summary exposes `chunks`/`stream_content` (chat) or `stream.*` flags
(crawlberg). client_factory base_url points at `MOCK_SERVER_URL/fixtures/{id}`; the mock
server is spawned by `AlefMockServer` when any fixture has `mock_response`.

## I. Runtime-only skips (intentional, fixture-documented)

- **xberg** LLM fixtures (tagged `llm`) → `pending!` when no `XBERG_LLM_API_KEY`.
- **liter-llm / smoke** local-provider fixtures (ollama/llamacpp/vllm model prefixes with no
  `mock_response`) → `pending!` when the local port is unreachable.

Crystal codegen mirroring of other language idioms (per aspect, best match):
| Aspect | Matched language | Crystal implementation |
|--------|------------------|------------------------|
| Config value object | Go `Config{}`, Ruby `Default::default()` | `Config.new` with getter defaults → Rust default shape |
| Tagged-enum defaults | C#/Go from-JSON | `from_json("{\"tag\":\"wire\"}")` / default-variant `.new` |
| Bytes round-trip | Go `[]byte` (JSON int array) | `Array(UInt8)` (was `Bytes`+ignore, which dropped payload) |
| Input construction | Go/C# from-JSON | `ExtractInput.from_json(fixture input)` |
| `$mock_url` | Go `strings.ReplaceAll` | `.gsub("$mock_url", base)` |
| Doc path resolution | Go `os.Chdir(test_documents)` | `Dir.cd(test_documents)` |
| Error surfacing | Ruby RuntimeError / C# exception | FFI `last_error_context` raised |
| Enum-field assertions | Ruby/Dart `fields_enum` | `.to_s.downcase` equals (wire value) |
| FFI feature enabling | hand-maintained Cargo.toml | `summarization` passthrough added |

## J. Binding/codegen capabilities landed on `crystal-all-fixtures`

| Change | Why |
|--------|-----|
| Enum + JSON converter modules emitted **before** structs | `@[JSON::Field(converter: …)]` needs the constant defined earlier (liter-llm `FilePurposeConverter`). |
| Optional params default `= nil`; nilable struct params pass a real null pointer | A `"null"` JSON string breaks the C ABI; `0_u64` timeouts fail the HTTP client. |
| `bytes::Bytes` return → `Bytes` (not `BytesBytes`); deserialize via `Array(UInt8).from_json` | `Slice` has no `from_json`; path-qualified named types now use the last `::` segment. |
| Untagged (shape-discriminated) union fields are nilable | No eager `from_json("{}")` default matches a variant. |
| Internally-tagged variant field types fully-qualified (`LiterLlm::ImageUrl`) | A bare name resolves to the enclosing variant class. |
| `CrystalConfig.borrowed_handles` | Opaque handles that are borrowed/shared (e.g. `Language`) get no freeing `finalize` — prevents double-free. |
| Visitor bridge `&[String]` callback params (`visit_table_row`) | Emits a C string-array pointer + count twin so the callbacks struct layout matches the Rust FFI vtable. |
| `custom_template` visitor actions → `#{param}` interpolation | Expands `{text}` placeholders instead of emitting a `Continue` stub. |
| Streaming supports field-based (crawlberg) and `json_object` (liter-llm) requests | Item type from `streaming_item_type()`; summary exposes `chunks`/`stream_content`/`stream.*`. |
| `display_as_text` fields extract the Text-variant value | `choices[0].message.content` is a discriminated union, not a String. |
| Enum-field assertions compare via `to_json` wire value | `.to_s.downcase` loses underscores (`ToolCalls` → `toolcalls`). |
| `is_document_subfield` narrowed to real wrapper fields | `content`/`tables` are flat on `ConversionResult` (html-to-markdown). |
| `metadata.open_graph[title]` Hash-bracket access | Aliased field paths render as `.open_graph["title"]`. |
| Binary fields (`audio`/`content`) map to the result only under `binary_result` | `file_content` returns `Bytes`; html-to-markdown's `content` is a real field. |
| Unit-return + `error_type` wrappers call the Void FFI fun without assigning | Status-only functions like `clean_cache` (`-> i32` with `set_last_error`). |
| Builtin types not module-qualified in array args | `Array(String)` (not `Array(Module::String)`). |
| Local-provider / LLM fixtures runtime-skip when their service/key is absent | Keeps offline CI green; documented in fixtures. |

## K. CI verification (crystal e2e jobs)

Dispatched the fork CI E2E workflows and observed the crystal jobs end-to-end:

- **Wiring verified**: repo guard removed (fork-local), build-ffi + artifact staging +
  Setup Crystal + Install alef all succeed; the `alef test --e2e --lang crystal` step runs.
- **Blocker (not wiring)**: the `install-alef@v1` action installs the **pinned alef
  version** from `alef.toml` (`alef_version = "0.55.6"` in every fork), which predates the
  Crystal backend. It fails with `unknown field 'crystal'` when parsing `[crates.crystal]`.
- **Required to unblock**: merge + release the Crystal backend upstream (xberg-io/alef),
  then bump `alef_version` in all five companion forks. The local branch binary works
  (`alef test --e2e --lang crystal` green on all five repos); CI is purely waiting on a
  release that ships the crystal config keyword and `Language::Crystal`.

### K2. Fork CI verification — END-TO-END GREEN (fork-built alef)

Forked `install-alef@v1` into a local composite action (`.github/actions/setup-alef`
in each fork) that builds alef from `dsisnero/alef@crystal-all-fixtures` and puts it
on PATH (outside the workspace tree, since consumer `Cargo.toml`s are workspaces).
Dispatched the fork CI E2E workflows and verified the crystal jobs pass:

| Repo | CI crystal result |
|------|-------------------|
| crawlberg | 256 / 0 / 0 / 1 |
| liter-llm | 166 / 0 / 0 / 2 |
| html-to-markdown | 273 / 0 / 0 / 0 |
| tree-sitter-language-pack | 522 / 0 / 0 / 2 (2 parser-not-bundled → runtime pending) |
| xberg | blocked on fork `test_documents` submodule commit unavailable (pre-existing) |

The tslp parser-availability rescue (pending on "not available for download")
keeps static-link local runs fully green while making CI bundle-only runs skip
the two all-group languages.

## L. How the other languages package & distribute (DeepWiki research)

The publish pipeline (`publish.yaml`) follows **prepare → build → publish → finalize**:
build the Rust core + `*-ffi` shared library once per target, then package that
same native lib into each language artifact.

| Language | Registry | How the native FFI is bundled |
|----------|----------|-------------------------------|
| Go | Go modules (GitHub) | `download_ffi` tool (`//go:generate`) fetches `crawlberg-go-<platform>.tar.gz` from GitHub releases → extracts to `.lib/<platform>/`; cgo links via `${SRCDIR}/.lib/` |
| C# / .NET | NuGet (`XbergIo.Crawlberg`) | prebuilt natives staged into `runtimes/<rid>/native/`; P/Invoke `[DllImport]`; `dotnet pack` → `.nupkg` → `publish-nuget` |
| Ruby | RubyGems | precompiled native gems via `rb_sys` + `rake-compiler` (x86_64/aarch64 linux/darwin, x64-mingw-ucrt); `extconf.rb` builds Rust ext; platform gems; `publish-rubygems` |
| Python | PyPI | wheels via PyO3/maturin (one wheel per platform incl. musl) |
| Node | npm | napi-rs native modules, platform sub-packages |
| Rust | crates.io | `cargo publish` the crates |
| Java | Maven Central | natives per classifier (`linux-x86_64`, `macos-arm64`, …) |
| Kotlin Android | Maven Central (AAR) | per-ABI JNI libs |
| Elixir | Hex.pm | NIF archives via rustler |
| PHP | Packagist | PIE extension archives via ext-php-rs |
| Dart | pub.dev | natives via flutter_rust_bridge |
| Swift | SwiftPM | artifact bundle `.binaryTarget` → GitHub release URL + checksum |
| WASM | npm | static-subset wasm module |
| Zig | (packages) | C ABI wrapper |
| Homebrew | brew | CLI bottle DSLs |

### Crystal distribution gap (documented, not yet implemented)
Crystal has **no upstream registry for prebuilt native shards** — `shards` (Crystal's
package manager) installs from source/Git, and there is no `runtimes/` or platform-gem
mechanism. The current fork CI builds alef + the FFI lib locally (via the
`setup-alef` fork action) and runs `crystal spec` with `--link-flags`. To distribute
Crystal like the others, the FFI `.so`/`.dylib`/`.dll` would be uploaded to GitHub
releases (like the Go `.tar.gz` archives) and the Crystal shard's `spec_helper` (or a
small `download_ffi` helper) would fetch + place it on the link path. The
`downloaded_languages()`/parser-bundle story (tslp) mirrors this same
release-artifact approach.

### L2. The exact Go `download_ffi` mechanism (DeepWiki)

Go's distribution (the model the Crystal shards mirror):

1. **`//go:generate`**: `packages/go/generate.go` holds `//go:generate go run ./cmd/download_ffi`. Consumer runs `go generate` before `go build`/`go test`.
2. **`cmd/download_ffi/main.go`**:
   - `determinePaths` maps `runtime.GOOS`/`GOARCH` → asset names (`darwin`→`macos`, `amd64`→`x86_64`, `arm64`→`aarch64` except darwin keeps `arm64`).
   - `downloadAndExtractLibrary` builds URL `https://github.com/<repo>/releases/download/v<ver>/crawlberg-go-<os>-<arch>.tar.gz` (`moduleVersion` constant → release tag), downloads, extracts to `moduleRoot/.lib/<os>-<arch>`, copies the `.so`/`.dylib`/`.dll` to `bindingLibDir`.
   - A shared cache dir is used (extract once, copy out), so repeated `go generate` runs are cheap.
3. **cgo wiring**: `binding.go`/`ffi.go` declare
   `CGO_CFLAGS: -I${SRCDIR}/include` and
   `CGO_LDFLAGS: -L${SRCDIR}/.lib/macos-arm64 -Wl,-rpath,${SRCDIR}/.lib/macos-arm64 -lcrawlberg_ffi`
   (`${SRCDIR}` = dir of the Go source file). No consumer build flags needed.
4. **Tag handling**: publish pushes `packages/go/vX.Y.Z` subdirectory tags so the Go module proxy resolves the version.
5. Smoke test (`test_apps/go`): `go mod download` → invoke `download_ffi` → `go test`.

**Crystal mirror (implemented in the `.cr` shards):** no `//go:generate` or `${SRCDIR}` — Crystal's `@[Link(ldflags:)]` is static and linker search paths are fixed. So the shards ship:
- `scripts/download_ffi.sh` (same platform→asset mapping, fetches the release `.tar.gz`, stages into `.lib/`),
- `make spec` / `scripts/spec.sh` wrapping `crystal spec --link-flags="-L$PWD/.lib -Wl,-rpath,$PWD/.lib"`,
- README documenting the `--link-flags` requirement (the Crystal-native equivalent of Go's `${SRCDIR}` cgo flags).
Verified green in CI on `dsisnero/crawlberg.cr`.

### L3. Crystal shard distribution — DONE (5 repos, CI green)

Created one Crystal shard repo per package under `dsisnero/`, each shipping the
alef-generated binding + a Go-style `download_ffi.sh` (Crystal has no prebuilt-native
shard registry). Each repo's CI downloads the FFI release artifact and runs the spec.

| Shard repo | shard name | version | CI |
|------------|-----------|---------|----|
| `dsisnero/crawlberg.cr` | `crawlberg` | 1.1.4 | success (ubuntu + macos) |
| `dsisnero/xberg.cr` | `xberg` | 1.1.0 | success (bundles onnxruntime from release archive) |
| `dsisnero/liter-llm.cr` | `liter_llm` | 1.16.0 | success |
| `dsisnero/html-to-markdown.cr` | `html_to_markdown_rs` | 3.10.6 | success |
| `dsisnero/tree-sitter-language-pack.cr` | `tree_sitter_language_pack` | 1.14.3 | success |

Pattern per repo:
- `shard.yml` (name/version/description/crystal>=1.0.0/targets) + `src/*.cr` (binding) + `spec/`.
- `scripts/download_ffi.sh`: maps platform → upstream release asset (`*-ffi-v<ver>-<target>.tar.gz`),
  downloads + extracts into `.lib/`, copies the `.so`/`.dylib`/`.dll`.
- `scripts/spec.sh` / `make spec`: `crystal spec --link-flags="-L$PWD/.lib -Wl,-rpath,$PWD/.lib"` —
  the Crystal-native equivalent of Go's `${SRCDIR}` cgo flags (Crystal's `@[Link]` is static).
- xberg.cr additionally links the `libonnxruntime.so.1` bundled in the FFI release archive
  (`-l:libonnxruntime.so.1` + `LD_LIBRARY_PATH`) since xberg's FFI needs ONNX Runtime.
- Consumer: `shards install` + `./scripts/download_ffi.sh` + `--link-flags` (documented in each README).

### L4. Crystal examples/guides generation flow (DONE — mirrors other languages)

DeepWiki research (crawlberg + liter-llm): the other languages ship examples/guides via
(1) a generated package `README.md` and (2) a docs-site with API reference + usage tabs.
Snippets live in `docs-site/src/snippets/<lang>/...`; `alef readme` renders them into the
package README (Jinja template + `[crates.readme.languages.<lang>]` config), and `alef docs`
emits the API reference (`api-<lang>.md`). docs-site usage pages import the same snippets.

Crystal now follows this exactly (all 5 forks):

| Repo | `alef readme --lang crystal` | `alef docs --lang crystal` | docs-site tab added |
|------|------------------------------|----------------------------|---------------------|
| crawlberg | `packages/crystal/README.md` ✓ | `api-crystal.md` ✓ | Basic Usage tab |
| html-to-markdown | `packages/crystal/README.md` ✓ | `api-crystal.md` ✓ | usage.mdx tab |
| liter-llm | `packages/crystal/README.md` ✓ | `api-crystal.md` ✓ | chat.mdx tab |
| xberg | `packages/crystal/README.md` ✓ | `api-crystal.md` ✓ | extraction.mdx tab |
| tree-sitter-language-pack | `packages/crystal/README.md` ✓ | `api-crystal.md` ✓ | quickstart install tab |

Flow (identical to Python/Go/etc.):
- `[crates.readme.languages.crystal]` config: name/template/output_path/package_manager(=`shards`)/description/snippets.
- `docs-site/src/snippets/crystal/getting-started/basic_usage.md` (or basic_chat / api/extract.md per repo)
  holds the Crystal "hello world" with the `download_ffi.sh` + `--link-flags` steps.
- `alef readme --lang crystal` → package README (installed in the `.cr` shard + published long_description analog).
- `alef docs --lang crystal` → docs-site API reference, tabbed alongside the other languages.
- README/docs generation is idempotent (`Generated 0 README files` on re-run).
