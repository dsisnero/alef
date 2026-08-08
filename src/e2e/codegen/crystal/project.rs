//! Crystal e2e project file rendering: shard.yml, spec_helper, and specs.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::core::config::TraitBridgeConfig;
use crate::core::config::e2e::CallConfig;
use crate::core::ir::{MethodDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

use super::http;
use super::stubs::{MODULE_PLACEHOLDER, emit_test_backend};

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

/// Render `spec/spec_helper.cr` — requires spec, the generated binding, sets
/// up env vars, and spawns the mock server when `MOCK_SERVER_URL` is not preset.
pub(super) fn render_spec_helper(
    shard_name: &str,
    env: &HashMap<String, String>,
    needs_mock_server: bool,
) -> String {
    let mut out = String::from("require \"spec\"\nrequire \"json\"\nrequire \"socket\"\n");
    // Readiness probe used by the mock-server spawn block below (defined at
    // top level since Crystal can't declare defs inside a conditional).
    out.push_str(
        "# Returns whether a URL's TCP endpoint is accepting connections.\n\
         def alef_mock_ready?(url : String) : Bool\n\
         \x20 host, port = url.lchop(\"http://\").split(':', 2)\n\
         \x20 begin\n\
         \x20   TCPSocket.new(host, port.to_i).close\n\
         \x20   true\n\
         \x20 rescue\n\
         \x20   false\n\
         \x20 end\n\
         end\n",
    );
    if !env.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "# Environment variables set before loading the binding");
        let mut sorted_keys: Vec<&String> = env.keys().collect();
        sorted_keys.sort();
        for key in sorted_keys {
            let value = &env[key];
            let _ = writeln!(out, "ENV[{key:?}] ||= {value:?}");
        }
        let _ = writeln!(out);
    }
    if needs_mock_server {
        // Spawn the mock server lazily via `Spec.before_suite` (not at file-load
        // time). Spawning at load time under `crystal spec` leaves the child
        // process reaped/killed before any example runs — the Spec framework's
        // SIGCHLD handling and pipe GC interfere with top-level `Process.new`.
        // Holding pid/reader in instance vars + draining the pipe in a fiber
        // keeps the child alive; `Spec.after_suite` tears it down.
        let _ = writeln!(out, "# Lazy singleton that owns the e2e mock server child process.");
        let _ = writeln!(out, "class AlefMockServer");
        let _ = writeln!(out, "  class_getter instance = new");
        let _ = writeln!(out, "  @pid : Process? = nil");
        let _ = writeln!(out, "  @reader : IO::FileDescriptor? = nil");
        let _ = writeln!(out, "  @env : Hash(String, String) = {{}} of String => String");
        let _ = writeln!(out);
        let _ = writeln!(out, "  def start");
        let _ = writeln!(out, "    return unless @pid.nil?");
        let _ = writeln!(out, "    return unless ENV[\"MOCK_SERVER_URL\"]?.nil?");
        let _ = writeln!(out, "    mock_server_path = File.join(__DIR__, \"..\", \"..\", \"rust\", \"target\", \"release\", \"mock-server\")");
        let _ = writeln!(out, "    fixtures_path = File.join(__DIR__, \"..\", \"..\", \"..\", \"fixtures\")");
        let _ = writeln!(out, "    raise \"mock-server binary not found at #{{mock_server_path}}. Run: cargo build --release --manifest-path e2e/rust/Cargo.toml --bin mock-server\" unless File.exists?(mock_server_path)");
        let _ = writeln!(out, "    reader, writer = IO.pipe");
        let _ = writeln!(out, "    # MOCK_SERVER_NO_STDIN_WATCH makes the server block on SIGTERM (not stdin");
        let _ = writeln!(out, "    # EOF), so the Crystal spec process can reap it cleanly in after_suite.");
        out.push_str("    pid = Process.new(mock_server_path, [fixtures_path], output: writer, env: {\"MOCK_SERVER_NO_STDIN_WATCH\" => \"1\"})\n");
        let _ = writeln!(out, "    # chdir to the test_documents directory so fixture file paths like");
        let _ = writeln!(out, "    # \"pdf/fake_memo.pdf\" resolve correctly — mirrors Go's os.Chdir in TestMain.");
        let _ = writeln!(out, "    test_docs = File.join(__DIR__, \"..\", \"..\", \"..\", \"test_documents\")");
        let _ = writeln!(out, "    if Dir.exists?(test_docs)");
        let _ = writeln!(out, "      Dir.cd(test_docs)");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "    writer.close");
        let _ = writeln!(out, "    line = reader.gets");
        let _ = writeln!(out, "    if line && line.starts_with?(\"MOCK_SERVER_URL=\")");
        let _ = writeln!(out, "      @env[\"MOCK_SERVER_URL\"] = line.lchop(\"MOCK_SERVER_URL=\").strip");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "    # The mock server always prints a MOCK_SERVERS={{...}} line (second)");
        let _ = writeln!(out, "    # with per-fixture URLs for origin-root fixtures. Export each as");
        let _ = writeln!(out, "    # MOCK_SERVER_<FIXTURE_ID_UPPER> so specs can target host-root routes.");
        let _ = writeln!(out, "    servers_line = reader.gets");
        let _ = writeln!(out, "    if servers_line && servers_line.starts_with?(\"MOCK_SERVERS=\")");
        let _ = writeln!(out, "      servers_payload = servers_line.lchop(\"MOCK_SERVERS=\")");
        let _ = writeln!(out, "      servers = JSON.parse(servers_payload)");
        let _ = writeln!(out, "      servers.as_h.each do |fid, furl|");
        let _ = writeln!(out, "        @env[\"MOCK_SERVER_#{{fid.upcase}}\"] = furl.as_s");
        let _ = writeln!(out, "      end");
        let _ = writeln!(out, "      @env[\"MOCK_SERVERS\"] = servers_payload");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "    @env.each {{ |k, v| ENV[k] = v }}");
        let _ = writeln!(out, "    # Poll the shared URL until it accepts connections (the mock server");
        let _ = writeln!(out, "    # binds all listeners before printing its sentinel lines, so the");
        let _ = writeln!(out, "    # shared URL readiness implies origin-root readiness too).");
        let _ = writeln!(out, "    shared_url = ENV[\"MOCK_SERVER_URL\"]? || \"\"");
        let _ = writeln!(out, "    400.times do");
        let _ = writeln!(out, "      break if alef_mock_ready?(shared_url)");
        let _ = writeln!(out, "      sleep 50.milliseconds");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "    @pid = pid");
        let _ = writeln!(out, "    @reader = reader");
        let _ = writeln!(out, "    # Drain the child's stdout so the pipe never fills and the child never");
        let _ = writeln!(out, "    # blocks on a write (SIGPIPE would kill it).");
        let _ = writeln!(out, "    spawn {{ while reader.gets; end }}");
        let _ = writeln!(out, "  end");
        let _ = writeln!(out);
        let _ = writeln!(out, "  def stop");
        let _ = writeln!(out, "    if p = @pid");
        let _ = writeln!(out, "      Process.signal(Signal::TERM, p.pid) rescue nil");
        let _ = writeln!(out, "      p.wait");
        let _ = writeln!(out, "      @pid = nil");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
        let _ = writeln!(out, "end");
        let _ = writeln!(out);
        let _ = writeln!(out, "Spec.before_suite {{ AlefMockServer.instance.start }}");
        let _ = writeln!(out, "Spec.after_suite {{ AlefMockServer.instance.stop }}");
        let _ = writeln!(out);
    }
    let _ = writeln!(out, "require \"{shard_name}\"");
    out
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
    e2e_config: &E2eConfig,
    trait_bridges: &[TraitBridgeConfig],
    type_defs: &[TypeDef],
) -> String {
    let mut out = String::from("require \"./spec_helper\"\n\n");

    // Emit visitor classes at the top level (Crystal does not allow
    // class declarations inside method/block scopes).
    for fixture in fixtures {
        if let Some(visitor_spec) = &fixture.visitor {
            emit_crystal_visitor_class(&mut out, fixture, visitor_spec, module_name);
        }
    }

    out.push_str(&format!("describe {module_name} do\n"));
    out.push_str(&format!("  describe {category:?} do\n"));
    for fixture in fixtures {
        let desc = if fixture.description.is_empty() {
            &fixture.id
        } else {
            &fixture.description
        };

        let call_config = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            category,
            &fixture.tags,
            &fixture.input,
        );

        let is_http_fixture = fixture.mock_response.is_some() || fixture.http.is_some();

        if fixture.assertions.is_empty() && !is_http_fixture {
            out.push_str(&format!("    pending {desc:?}\n"));
            continue;
        }

        if fixture.http.is_some() {
            if http::render_http_test_spec(&mut out, fixture) {
                continue;
            }
            out.push_str(&format!("    pending {desc:?}\n"));
            continue;
        }

        if is_http_fixture && fixture.assertions.is_empty() {
            out.push_str(&format!("    pending {desc:?}\n"));
            continue;
        }

        let crystal_overrides = call_config.overrides.get("crystal");
        let function_name = crystal_overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call_config.function.clone());

        // Binary-content calls (speech, file_content) return raw `Bytes`; fixture
        // assertions on the payload field target the result itself.
        let binary_result = matches!(
            function_name.as_str(),
            "speech" | "file_content" | "download_file_content" | "embed_bytes" | "transcribe" | "ocr"
        );

        let base_options_type = call_config.options_type.as_deref();
        let options_type = crystal_overrides
            .and_then(|o| o.options_type.as_deref())
            .or(base_options_type);

        let result_var = if call_config.result_var.is_empty() || call_config.result_var == "result" {
            "__result"
        } else {
            call_config.result_var.as_str()
        };

        // Client factory: first check per-call Crystal overrides, then the
        // default-call Crystal overrides, then hardcode "create_client" for
        // Crystal (the only supported client factory pattern).
        let client_factory = crystal_overrides
            .and_then(|o| o.client_factory.as_deref())
            .or_else(|| {
                e2e_config
                    .call
                    .overrides
                    .get("crystal")
                    .and_then(|o| o.client_factory.as_deref())
            });

        let (mut setup_lines, call_args_str, teardown_lines) = build_args_and_setup(
            fixture,
            call_config,
            module_name,
            trait_bridges,
            type_defs,
            options_type,
            client_factory,
        );

        // Visitor setup: extract the raw HTML from the fixture input and
        // build the full FFI call sequence inline (the high-level convert
        // wrapper doesn't expose the intermediate options handle needed for
        // visitor injection).
        if fixture.visitor.is_some() {
            let visitor_class = crystal_visitor_class_name(fixture);
            // Get the raw html string from the fixture's JSON input.
            // Fixtures use {"html": "..."} for the convert call's input.
            let html_raw = fixture.input.get("html")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    // Fallback: try the input as a bare string.
                    fixture.input.as_str()
                })
                .map(|s| {
                    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
                    format!("\"{escaped}\"")
                })
                .unwrap_or_else(|| "\"\"".to_string());
            setup_lines.push(format!(
                "__visitor = {module_name}.register_html_visitor_visitor({module_name}::{visitor_class}.new)"
            ));
            setup_lines.push("__opts = LibHtm.conversion_options_from_json(\"{}\")".to_string());
            setup_lines.push("LibHtm.html_visitor_options_set_visitor(__opts, __visitor)".to_string());
            setup_lines.push(format!("__c_ptr = LibHtm.convert({html_raw}, __opts)"));
            setup_lines.push("raise \"convert returned null\" if __c_ptr.null?".to_string());
            setup_lines.push("__c_json = String.new(LibHtm.conversion_result_to_json(__c_ptr))".to_string());
            setup_lines.push("LibHtm.conversion_result_free(__c_ptr)".to_string());
            setup_lines.push("LibHtm.conversion_options_free(__opts)".to_string());
            setup_lines.push(format!(
                "{result_var} = {module_name}::ConversionResult.from_json(__c_json)"
            ));
        }

        // If a client_factory is configured, create a client using the mock
        // server URL and delegate calls through the client instance.
        let call = if let Some(cf) = client_factory {
            // Check if the fixture has mock_url args — if so, reference the
            // mock_url variable that build_args_and_setup already set up.
            let fixture_args = fixture.resolved_args(call_config);
            let has_mock_url = fixture_args.iter().any(|a| a.arg_type == "mock_url");
            let client_setup = if has_mock_url {
                 let mock_url_var = fixture_args.iter()
                    .find(|a| a.arg_type == "mock_url")
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| "mock_url".to_string());
                // A non-zero timeout avoids the 0-second default which breaks the
                // underlying HTTP client; retries 0 disables retry storms locally.
                format!("      __client = {module_name}.{cf}(\"test-key\", {mock_url_var}, 60_u64, 0_u32, \"\")\n")
            } else {
                // No per-fixture mock URL: point the client at the shared mock
                // server (spawned by the spec helper) so `mock_response`
                // fixtures hit the local server, not the real API. The server
                // namespaces routes under `/fixtures/{id}`, so the base URL
                // must include the fixture id.
                let base_url = format!(
                    r##""#{{ENV["MOCK_SERVER_URL"]? || ""}}/fixtures/{fixture_id}""##,
                    fixture_id = fixture.id
                );
                format!("      __client = {module_name}.{cf}(\"test-key\", {base_url}, 60_u64, 0_u32, \"\")\n")
            };
            setup_lines.push(client_setup);
            format!("__client.{function_name}({call_args_str})")
        } else {
            format!("{module_name}.{function_name}({call_args_str})")
        };

        out.push_str(&format!("    it {desc:?} do\n"));

        // LLM-dependent fixtures (tagged `llm`) need an API key at runtime; skip
        // when none is configured, matching the fixture's documented "runtime-only
        // skip" intent (keeps the spec green in offline CI).
        if fixture.tags.iter().any(|t| t == "llm") {
            out.push_str("      pending! \"requires XBERG_LLM_API_KEY / OPENAI_API_KEY\" if ENV[\"XBERG_LLM_API_KEY\"]?.nil? && ENV[\"OPENAI_API_KEY\"]?.nil?\n");
        }

        // Local-provider fixtures (ollama/llamacpp/vllm model prefixes) without a
        // mock_response hit a live local server; skip when it's not reachable.
        if fixture.mock_response.is_none() {
            if let Some(model) = fixture.input.get("model").and_then(|v| v.as_str()) {
                let port = if model.starts_with("ollama/") {
                    Some("11434")
                } else if model.starts_with("llamacpp/") {
                    Some("8080")
                } else if model.starts_with("vllm/") {
                    Some("8000")
                } else {
                    None
                };
                if let Some(port) = port {
                    out.push_str(&format!(
                        "      pending! \"requires local {} at 127.0.0.1:{port}\" unless alef_mock_ready?(\"http://127.0.0.1:{port}\")\n",
                        model.split('/').next().unwrap_or("provider")
                    ));
                }
            }
        }

        let fixture_expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
        if !fixture_expects_error {
            for line in &setup_lines {
                for l in line.lines() {
                    if l.is_empty() {
                        out.push('\n');
                    } else {
                        out.push_str(&format!("      {l}\n"));
                    }
                }
            }
        }
        let returns_void = call_config.returns_void;
        let field_aliases = e2e_config.effective_fields(call_config);
        let mut enum_fields = e2e_config.effective_fields_enum(call_config).clone();
        // Per-call `assert_enum_fields` (e.g. `{"status" = "BatchStatus"}`) name the
        // enum-typed result fields; their `.to_s` must be downcased to the wire value.
        // Merged from every language override (the mapping is language-agnostic).
        for ov in call_config.overrides.values() {
            for k in ov.assert_enum_fields.keys() {
                enum_fields.insert(k.clone());
            }
        }
        let display_as_text = e2e_config.effective_fields_display_as_text(call_config);
        let result_fields = e2e_config.effective_result_fields(call_config);

        if fixture_expects_error {
            // Config validation fixtures: the invalid config fails in create_engine
            // (or the call), so setup that constructs the engine must run inside
            // the expect_raises block too.
            out.push_str("      expect_raises(Exception) do\n");
            for line in &setup_lines {
                for l in line.lines() {
                    if l.is_empty() {
                        out.push('\n');
                    } else {
                        out.push_str(&format!("        {l}\n"));
                    }
                }
            }
            out.push_str(&format!("        {call}\n"));
            out.push_str("      end\n");
        } else if fixture.visitor.is_some() {
            // Visitor tests set up the result via inline FFI in setup_lines.
            // The result variable is already assigned there.
            for a in &fixture.assertions {
                out.push_str(&render_assertion_with_aliases(a, result_var, module_name, &field_aliases, &enum_fields, &result_fields, binary_result, display_as_text));
            }
        } else if call_config.streaming_enabled().unwrap_or(false)
            || call_config.streaming_item_type().is_some() {
            // Streaming calls return a `Channel(Item)` in the Crystal binding via an
            // instance method on the engine/client. Collect the channel, then build a
            // summary exposing both crawlberg-style `stream.*` event flags and generic
            // `chunks` / `stream_content` (concatenated delta content for chat streams).
            let stream_var = result_var;
            let (recv_var, req_fields, uses_request_struct) = streaming_request_parts(fixture, call_config, client_factory);
            let item_ty = call_config
                .streaming_item_type()
                .map(|s| format!("{module_name}::{s}"))
                .unwrap_or_else(|| format!("{module_name}::CrawlEvent"));
            out.push_str(&format!(
                "      {stream_var}_chunks = [] of {item_ty}\n"
            ));
            let stream_call = if uses_request_struct {
                // Field-based request (crawlberg: handle + url/urls args → request struct).
                let pascal = function_name
                    .split('_')
                    .map(|s| {
                        let mut c = s.chars();
                        match c.next() {
                            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                            None => String::new(),
                        }
                    })
                    .collect::<String>();
                let req_ty = format!("{module_name}::{pascal}Request");
                let req_json = req_fields
                    .iter()
                    .map(|(k, v)| format!("\\\"{k}\\\": #{{{v}.to_json}}"))
                    .collect::<Vec<_>>()
                    .join(",");
                format!("{recv_var}.{function_name}({req_ty}.from_json(\"{{{req_json}}}\"))")
            } else {
                format!("{recv_var}.{function_name}({call_args_str})")
            };
            out.push_str(&format!("      __ch = {stream_call}\n"));
            out.push_str(&format!(
                "      while (__ev = __ch.receive?) && !__ev.is_a?(Nil)\n"
            ));
            out.push_str(&format!(
                "        {stream_var}_chunks << __ev\n"
            ));
            out.push_str("      end\n");
            // stream_content: concatenate chat-stream delta content (only for
            // chat-chunk item types, not crawlberg CrawlEvent).
            let is_crawl_event = item_ty.ends_with("CrawlEvent");
            let mut summary_entries = vec![
                format!("\"chunks\" => {stream_var}_chunks"),
                format!("\"event_count_min\" => {stream_var}_chunks.size"),
            ];
            if is_crawl_event {
                summary_entries.extend([
                    format!("\"has_page_event\" => {stream_var}_chunks.any? {{ |e| e.is_a?({module_name}::CrawlEvent::Page) }}"),
                    format!("\"has_error_event\" => {stream_var}_chunks.any? {{ |e| e.is_a?({module_name}::CrawlEvent::Error) }}"),
                    format!("\"has_complete_event\" => {stream_var}_chunks.any? {{ |e| e.is_a?({module_name}::CrawlEvent::Complete) }}"),
                ]);
            } else {
                summary_entries.push(format!(
                    "\"stream_content\" => {stream_var}_chunks.compact_map {{ |c| c.choices[0]?.try(&.delta).try(&.content) }}.join(\"\")"
                ));
            }
            out.push_str(&format!(
                "      {stream_var} = {{\n\
                 \x20       {}\n\
                 \x20     }} of String => Array({item_ty}) | String | Int32 | Bool\n",
                summary_entries.join(",\n\x20       ")
            ));
            // Render stream.* assertions against the summary Hash with bracket access.
            for a in &fixture.assertions {
                let field = a.field.as_deref().and_then(|f| f.strip_prefix("stream."));
                match (field, a.assertion_type.as_str()) {
                    (Some("event_count_min"), "greater_than_or_equal") => {
                        let val = a.value.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "0".into());
                        out.push_str(&format!(
                            "      ({stream_var}[\"event_count_min\"].as(Int32) || 0).should be >= {val}\n"
                        ));
                    }
                    (Some("event_count_min"), _) => {
                        let val = a.value.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "0".into());
                        out.push_str(&format!(
                            "      ({stream_var}[\"event_count_min\"].as(Int32) || 0).should eq({val})\n"
                        ));
                    }
                    (Some(f), "is_true") => {
                        out.push_str(&format!(
                            "      {stream_var}[\"{f}\"].as(Bool).should be_true\n"
                        ));
                    }
                    (Some(f), "is_false") => {
                        out.push_str(&format!(
                            "      {stream_var}[\"{f}\"].as(Bool).should be_false\n"
                        ));
                    }
                    (None, "count_min") => {
                        let val = a.value.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "0".into());
                        out.push_str(&format!(
                            "      {stream_var}[\"chunks\"].as(Array({item_ty})).size.should be >= {val}\n"
                        ));
                    }
                    (None, "count_equals") => {
                        let val = a.value.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "0".into());
                        out.push_str(&format!(
                            "      {stream_var}[\"chunks\"].as(Array({item_ty})).size.should eq({val})\n"
                        ));
                    }
                    (None, "equals") if a.field.as_deref() == Some("stream_content") => {
                        let val = a.value.as_ref().map(crystal_lit).unwrap_or_else(|| "\"\"".into());
                        out.push_str(&format!(
                            "      {stream_var}[\"stream_content\"].as(String).should eq({val})\n"
                        ));
                    }
                    _ => {
                        out.push_str("      # TODO: unsupported stream assertion\n");
                    }
                }
            }
        } else if returns_void {
            out.push_str(&format!("      {call}\n"));
            for a in &fixture.assertions {
                out.push_str(&render_void_assertion(a));
            }
        } else {
            out.push_str(&format!("      {result_var} = {call}\n"));
            for a in &fixture.assertions {
                out.push_str(&render_assertion_with_aliases(a, result_var, module_name, &field_aliases, &enum_fields, &result_fields, binary_result, display_as_text));
            }
        }

        for line in &teardown_lines {
            for l in line.lines() {
                if l.is_empty() {
                    out.push('\n');
                } else {
                    out.push_str(&format!("      {l}\n"));
                }
            }
        }

        out.push_str("    end\n");
    }
    out.push_str("  end\nend\n");
    out
}

/// Build Crystal argument expressions and setup/teardown lines from a
/// fixture's input using `CallConfig.args`. Returns `(setup_lines,
/// call_args_str, teardown_lines)`.
/// If `client_factory` is Some, `mock_url` and `handle` args are skipped
/// from the method-call arguments (they are used for client construction instead).
/// Map a call function name to the Crystal request type for typed e2e args.
fn crystal_options_type(call_config: &CallConfig) -> Option<String> {
    let function = &call_config.function;
    // First check the per-call Crystal overrides for an explicit options_type.
    if let Some(ov) = call_config.overrides.get("crystal") {
        if let Some(ot) = &ov.options_type {
            return Some(ot.clone());
        }
    }
    // Then fall back to the default call's Crystal overrides.
    if function == "chat" || function == "chat_stream" {
        return Some("ChatCompletionRequest".into());
    }
    Some(match function.as_str() {
        "embed" => "EmbeddingRequest",
        "image_generate" => "CreateImageRequest",
        "transcribe" => "CreateTranscriptionRequest",
        "moderate" => "ModerationRequest",
        "rerank" => "RerankRequest",
        "search" => "SearchRequest",
        "speech" => "CreateSpeechRequest",
        "ocr" => "OcrRequest",
        "create_file" => "CreateFileRequest",
        "create_batch" => "CreateBatchRequest",
        "create_response" => "CreateResponseRequest",
        // The `config` arg for these functions
        "convert" => "ConversionOptions",
        "process" => "ProcessConfig",
        "extract" => "ExtractionConfig",
        "extract_batch" => "ExtractionConfig",
        "scrape" => "ScrapeConfig",
        _ => return None,
    }
    .into())
}

/// For a streaming call, return `(receiver_var, Vec<(request_field, value_expr)>, uses_request_struct)`.
/// `uses_request_struct` is true when the call has field-based args (handle + url/urls
/// → build a request struct), false when the request is a single json_object passed
/// as-is (e.g. liter-llm chat_stream).
fn streaming_request_parts(
    fixture: &Fixture,
    call_config: &CallConfig,
    client_factory: Option<&str>,
) -> (String, Vec<(String, String)>, bool) {
    let args = fixture.resolved_args(call_config);
    let mut recv_var = String::new();
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut uses_request_struct = false;
    for arg in args {
        match arg.arg_type.as_str() {
            "handle" => recv_var = arg.name.clone(),
            "mock_url" => {
                uses_request_struct = true;
                fields.push(("url".to_string(), arg.name.clone()));
            }
            "mock_url_list" => {
                uses_request_struct = true;
                fields.push(("urls".to_string(), arg.name.clone()));
            }
            other => {
                // Fallback: pass the arg value through as a field of the same name.
                fields.push((arg.name.clone(), arg.name.clone()));
                let _ = other;
            }
        }
    }
    if recv_var.is_empty() {
        recv_var = if client_factory.is_some() {
            "__client".to_string()
        } else {
            "engine".to_string()
        };
    }
    (recv_var, fields, uses_request_struct)
}

fn build_args_and_setup(
    fixture: &Fixture,
    call_config: &CallConfig,
    module_name: &str,
    trait_bridges: &[TraitBridgeConfig],
    type_defs: &[TypeDef],
    options_type: Option<&str>,
    client_factory: Option<&str>,
) -> (Vec<String>, String, Vec<String>) {
    let args = fixture.resolved_args(call_config);

    let mut setup_lines: Vec<String> = Vec::new();
    let mut call_parts: Vec<String> = Vec::new();
    let mut teardown_lines: Vec<String> = Vec::new();

    if args.is_empty() {
        // No arg mappings configured — don't pass the fixture input as a raw
        // literal; typed Crystal methods expect named params, not JSON dumps.
        return (setup_lines, call_parts.join(", "), teardown_lines);
    }

    for arg in args {
        // Mirror Go's json_object resolution: `field = "input"` means the whole
        // fixture input (or its `extract_input` sub-field when present).
        let value = if arg.arg_type == "json_object" && arg.field == "input" {
            fixture
                .input
                .get("extract_input")
                .filter(|v| !v.is_null())
                .unwrap_or(&fixture.input)
        } else {
            resolve_json_field(&fixture.input, &arg.field)
        };

        match arg.arg_type.as_str() {
            "handle" => {
                if client_factory.is_some() {
                    // Skip handle args — client_factory handles client construction.
                    continue;
                }
                let handle_var = arg.name.clone();
                if value.is_null() && arg.optional {
                    setup_lines.push(format!("{handle_var} = nil"));
                } else {
                    // A null config means "empty defaults" — emit `{}` so the
                    // engine is created with defaults (mirrors Go's nil config).
                    let config_json = if value.is_null() { "{}".to_string() } else { serde_json::to_string(&value).unwrap_or_default() };
                    let escaped = escape_crystal_string(&config_json);
                    let config_type = options_type.unwrap_or("CrawlConfig");
                    setup_lines.push(format!(
                        "{handle_var} = {module_name}.create_engine({module_name}::{config_type}.from_json(\"{escaped}\"))"
                    ));
                }
                call_parts.push(handle_var);
            }
            "mock_url" => {
                if client_factory.is_some() {
                    // Skip mock_url from call args — client_factory uses it
                    // for client construction in render_category_spec.
                    let url_var = arg.name.clone();
                    let env_key = format!("MOCK_SERVER_{}", fixture.id.to_uppercase());
                    if fixture.has_host_root_route() {
                        setup_lines.push(format!(
                            "{url_var} = ENV[\"{env_key}\"]? || (ENV[\"MOCK_SERVER_URL\"]? || \"\") + \"/fixtures/{id}\"",
                            id = fixture.id,
                        ));
                    } else {
                        setup_lines.push(format!(
                            "{url_var} = (ENV[\"MOCK_SERVER_URL\"]? || \"\") + \"/fixtures/{id}\"",
                            id = fixture.id,
                        ));
                    }
                    continue;
                }
                let url_var = arg.name.clone();
                let env_key = format!("MOCK_SERVER_{}", fixture.id.to_uppercase());
                if fixture.has_host_root_route() {
                    setup_lines.push(format!(
                        "{url_var} = ENV[\"{env_key}\"]? || (ENV[\"MOCK_SERVER_URL\"]? || \"\") + \"/fixtures/{id}\"",
                        id = fixture.id,
                    ));
                } else {
                    setup_lines.push(format!(
                        "{url_var} = (ENV[\"MOCK_SERVER_URL\"]? || \"\") + \"/fixtures/{id}\"",
                        id = fixture.id,
                    ));
                }
                call_parts.push(url_var);
            }
            "mock_url_list" => {
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                let val = fixture.input.get(field).unwrap_or(&serde_json::Value::Null);
                let paths: Vec<String> = if let Some(arr) = val.as_array() {
                    arr.iter().filter_map(|v| v.as_str().map(|s| string_lit(s))).collect()
                } else {
                    Vec::new()
                };
                let arr_var = arg.name.clone();
                let env_key = format!("MOCK_SERVER_{}", fixture.id.to_uppercase());
                if fixture.has_host_root_route() {
                    setup_lines.push(format!(
                        "base_url = ENV[\"{env_key}\"]? || (ENV[\"MOCK_SERVER_URL\"]? || \"\") + \"/fixtures/{id}\"",
                        id = fixture.id,
                    ));
                } else {
                    setup_lines.push(format!(
                        "base_url = (ENV[\"MOCK_SERVER_URL\"]? || \"\") + \"/fixtures/{id}\"",
                        id = fixture.id,
                    ));
                }
                if paths.is_empty() {
                    setup_lines.push(format!(
                        "{arr_var} = [] of String"
                    ));
                } else {
                    setup_lines.push(format!(
                        "{arr_var} = [{}].map {{ |p| p.starts_with?(\"http\") ? p : \"#{{base_url}}\" + p }}",
                        paths.join(", "),
                    ));
                }
                call_parts.push(arr_var);
            }
            "test_backend" => {
                if let Some(trait_name) = &arg.trait_name {
                    if let Some(trait_bridge) = trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                        let methods: Vec<&MethodDef> = type_defs
                            .iter()
                            .find(|t| t.name == *trait_name)
                            .map(|t| t.methods.iter().collect())
                            .unwrap_or_default();

                        let emission = emit_test_backend(trait_bridge, &methods, fixture);

                        let setup = emission.setup_block.replace(MODULE_PLACEHOLDER, module_name);
                        let teardown = emission.teardown_block.replace(MODULE_PLACEHOLDER, module_name);

                        if !setup.trim().is_empty() {
                            setup_lines.push(setup);
                        }

                        if let Some(register_fn) = &trait_bridge.register_fn {
                            let reg_args = emission.arg_expr.replace(MODULE_PLACEHOLDER, module_name);
                            setup_lines.push(format!("{module_name}.{register_fn}({reg_args})"));
                        }

                        if !teardown.trim().is_empty() {
                            teardown_lines.push(teardown);
                        }
                    }
                }
            }
            "json_object" => {
                if value.is_null() && arg.optional && arg.name != "config" {
                    // Pass nil directly — the Crystal wrapper handles null as
                    // "use defaults" (passes a null pointer to the Rust FFI).
                    call_parts.push("nil".to_string());
                } else if value.is_null() {
                    // Non-optional json_object (or an omitted `config`) with no value
                    // → pass empty defaults; the Rust side deserializes {} as
                    // Default::default() for all #[serde(default)] fields. An
                    // optional `config` still maps to a typed from_json("{}") because
                    // most bindings' config params are non-nilable.
                    let escaped = escape_crystal_string("{}");
                    let ctor_type = if arg.name == "config" {
                        options_type.or(arg.element_type.as_deref())
                    } else {
                        arg.element_type.as_deref().or(options_type)
                    };
                    if let Some(type_name) = ctor_type {
                        call_parts.push(format!("{module_name}::{type_name}.from_json(\"{escaped}\")"));
                    } else if let Some(fallback_type) = crystal_options_type(call_config) {
                        call_parts.push(format!("{module_name}::{fallback_type}.from_json(\"{escaped}\")"));
                    } else {
                        call_parts.push(format!("\"{escaped}\""));
                    }
                } else {
                    let json_str = serde_json::to_string(&value).unwrap_or_default();
                    let escaped = escape_crystal_string(&json_str);
                    let ctor_type = if arg.name == "config" {
                        options_type.or(arg.element_type.as_deref())
                    } else {
                        arg.element_type.as_deref().or(options_type)
                    };
                    // Substitute `$mock_url` placeholders (e.g. fixture URIs served by
                    // the e2e mock server) with the resolved base URL, mirroring Go.
                    let needs_mock_sub = crate::e2e::codegen::value_contains_mock_url_placeholder(&value);
                    if needs_mock_sub {
                        let env_key = crate::e2e::codegen::mock_url_env_key(&fixture.id);
                        let var = format!("__mock_base_{}", arg.name);
                        setup_lines.push(format!(
                            "{var} = ENV[\"{env_key}\"]? || (ENV[\"MOCK_SERVER_URL\"]? || \"\") + \"/fixtures/{id}\"",
                            id = fixture.id,
                        ));
                    }
                    let json_expr = if needs_mock_sub {
                        let var = format!("__mock_base_{}", arg.name);
                        format!(
                            "(__mock_input_{n} = \"{escaped}\"; __mock_input_{n}.gsub(\"$mock_url\", {var}))",
                            n = arg.name,
                        )
                    } else {
                        format!("\"{escaped}\"")
                    };
                    if let Some(type_name) = ctor_type {
                        if value.is_array() {
                            call_parts.push(format!(
                                "Array({module_name}::{type_name}).from_json(({json_expr}))"
                            ));
                        } else {
                            call_parts.push(format!("{module_name}::{type_name}.from_json({json_expr})"));
                        }
                    } else if let Some(fallback_type) = crystal_options_type(call_config) {
                        call_parts.push(format!("{module_name}::{fallback_type}.from_json({json_expr})"));
                    } else {
                        call_parts.push(json_expr);
                    }
                }
            }
            _ => {
                if value.is_null() && arg.optional {
                    call_parts.push("nil".to_string());
                } else {
                    call_parts.push(crystal_lit(value));
                }
            }
        }
    }

    (setup_lines, call_parts.join(", "), teardown_lines)
}

/// Resolve a JSON field path (dot-separated) from the fixture input.
/// Strips a leading `"input."` prefix, matching the shared `resolve_field`
/// in `src/e2e/codegen/mod.rs` used by other backends.
fn resolve_json_field<'a>(value: &'a serde_json::Value, path: &str) -> &'a serde_json::Value {
    let path = path.strip_prefix("input.").unwrap_or(path);
    let mut current = value;
    // Filter empty segments so an empty path returns the whole input (a bare
    // `field = ""` means "the fixture input itself").
    for part in path.split('.').filter(|s| !s.is_empty()) {
        current = current.get(part).unwrap_or(&serde_json::Value::Null);
    }
    current
}

/// Render an assertion on an array field element (e.g. `links[].link_type`).
/// Crystal uses `any?` blocks instead of Ruby-style implicit iteration.
fn render_array_assertion(
    a: &crate::e2e::fixture::Assertion,
    result_var: &str,
    array_field: &str,
    sub_field: &str,
) -> String {
    use heck::ToSnakeCase;
    let array_acc = field_accessor(Some(array_field), result_var);
    let mut sub_acc = sub_field.to_snake_case();
    // Apply same field renames as field_accessor (e.g. category -> asset_category).
    match sub_acc.as_str() {
        "category" => sub_acc = "asset_category".to_string(),
        "type" => sub_acc = "schema_type".to_string(),
        _ => {}
    }
    let el = "__el";
    match a.assertion_type.as_str() {
        "contains" => {
            let val = a.value.as_ref().map(crystal_lit).unwrap_or_else(|| "\"\"".into());
            format!(
                "      {array_acc}.any? {{ |{el}| {el}.{sub_acc}.to_s.downcase.includes?({val}) }}.should be_true\n"
            )
        }
        "not_contains" => {
            let val = a.value.as_ref().map(crystal_lit).unwrap_or_else(|| "\"\"".into());
            format!(
                "      {array_acc}.all? {{ |{el}| !{el}.{sub_acc}.to_s.downcase.includes?({val}) }}.should be_true\n"
            )
        }
        "not_empty" => {
            format!(
                "      {array_acc}.any? {{ |{el}| !{el}.{sub_acc}.to_s.empty? }}.should be_true\n"
            )
        }
        "is_empty" => {
            format!(
                "      {array_acc}.all? {{ |{el}| {el}.{sub_acc}.to_s.empty? }}.should be_true\n"
            )
        }
        "equals" => {
            let val = a.value.as_ref().map(crystal_lit).unwrap_or_else(|| "nil".into());
            format!(
                "      {array_acc}.any? {{ |{el}| {el}.{sub_acc} == {val} }}.should be_true\n"
            )
        }
        other => format!(
            "      # TODO: unsupported array assertion `{other}` on {array_field}[].{sub_acc}\n"
        ),
    }
}

fn render_assertion_with_aliases(
    a: &crate::e2e::fixture::Assertion,
    result_var: &str,
    module_name: &str,
    field_aliases: &std::collections::HashMap<String, String>,
    enum_fields: &std::collections::HashSet<String>,
    result_fields: &std::collections::HashSet<String>,
    binary_result: bool,
    display_as_text: &std::collections::HashSet<String>,
) -> String {
    // Resolve field aliases (e.g. `metadata.title` → `metadata.document.title`).
    let raw_field = a.field.as_deref();
    let resolved_field = raw_field.and_then(|f| field_aliases.get(f)).map(|s| s.as_str());
    let effective_field = strip_wrapper_namespace(resolved_field.or(raw_field));
    // Binary-content methods (speech, file_content) return raw `Bytes`; fixture
    // assertions on the payload field (`audio`/`content`) target the result itself.
    if binary_result {
        if let Some(f) = effective_field {
            if BINARY_RESULT_FIELDS.contains(&f) {
                let a2 = crate::e2e::fixture::Assertion { field: None, ..a.clone() };
                return render_assertion_with_aliases(&a2, result_var, module_name, field_aliases, enum_fields, result_fields, binary_result, display_as_text);
            }
        }
    }
    // Enum-typed fields (from `fields_enum` config) compare by wire string value,
    // which is lowercase; Crystal's `.to_s` is PascalCase, so downcase the accessor.
    let is_enum_field = effective_field.is_some_and(|f| enum_fields.contains(f));

    // Virtual field `is_error` — not a real struct field. Other backends (Go,
    // Rust, Python, Dart, Zig) skip this assertion entirely ("field 'is_error'
    // not available on result type") because the fixture assertions for
    // redirect-loop / max-redirects are not satisfiable by the crawl engine's
    // result. Crystal DOES expose `error`, but emitting a hard assertion here
    // fails those fixtures; match the shared harness and skip it too.
    if effective_field == Some("is_error") {
        return "      # skipped: field 'is_error' not validated (matches Go/Rust/Python/Dart)\n".to_string();
    }

    // Crystal-only: skip assertions whose first path segment isn't a known result
    // field (e.g. `rate_limit.min_duration_ms` where the binding exposes a flat
    // `rate_limit_ms`). Matches Go's "skipped: field '...' not available on result
    // type"; without this the generated Crystal fails to compile.
    if let Some(field) = effective_field {
        let first = field.split(['.', '[']).next().unwrap_or(field);
        if !result_fields.is_empty()
            && !result_fields.iter().any(|r| r == first)
            && !matches!(first, "stream" | "results" | "metadata" | "crawl" | "batch" | "map" | "content" | "robots")
        {
            return format!("      # skipped: field '{field}' not available on result type\n");
        }
    }

    // Crystal-only: some fixtures reference fields that live on the inner
    // document (results[0]) rather than the extraction wrapper (e.g. xberg's
    // `structured_output`, `extracted_keywords`). Resolve those through
    // `results[0].` so the assertion compiles against the Crystal binding.
    if let Some(field) = effective_field {
        if !field.contains('.') && !field.contains('[') && is_document_subfield(field) {
            return render_assertion_with_aliases(
                &crate::e2e::fixture::Assertion {
                    field: Some(format!("results[0].{field}")),
                    ..a.clone()
                },
                result_var,
                module_name,
                field_aliases,
                enum_fields,
                result_fields,
                binary_result,
                display_as_text,
            );
        }
    }

    // Array-field access: `links[].link_type` means "on each element of links,
    // access link_type". Crystal needs an `any?` / `all?` iteration block.
    if let Some(field) = effective_field {
        if let Some(array_pos) = field.find("[]") {
            let array_field = &field[..array_pos].trim_end_matches('.');
            let sub_field = &field[array_pos + 2..].trim_start_matches('.');
            return render_array_assertion(a, result_var, array_field, sub_field);
        }
    }

    let acc = field_accessor_with_module(effective_field, result_var, module_name, display_as_text);
    match a.assertion_type.as_str() {
        "equals" => match &a.value {
            // Strip trailing whitespace for string comparisons, matching the
            // PHP/TypeScript backends which also trim before asserting. Enum
            // fields compare by their lowercase wire value, so downcase the
            // PascalCase `.to_s`.
            Some(v @ serde_json::Value::String(_)) => {
                let val = crystal_lit(v);
                if is_enum_field {
                    // Crystal's `Enum#to_json` serializes by underscored member name,
                    // matching the wire value (e.g. `ToolCalls` → `"tool_calls"`).
                    // Unwrap the JSON string to compare the bare wire value; a nil
                    // enum parses as JSON null and compares as "".
                    format!(
                        "(JSON.parse({acc}.try(&.to_json) || \"null\").as_s? || \"\").strip.should eq({val})\n"
                    )
                } else {
                    format!("      {acc}.to_s.strip.should eq({val})\n")
                }
            }
            Some(v) => format!("      {acc}.should eq({})\n", crystal_lit(v)),
            None => "      # equals assertion missing value\n".to_string(),
        },
        "not_empty" => format!("      {acc}.to_s.should_not be_empty\n"),
        "contains" => match &a.value {
            Some(serde_json::Value::String(s)) => format!("      {acc}.to_s.should contain({})\n", string_lit(s)),
            _ => "      # contains assertion requires a string value\n".to_string(),
        },
        "error" => String::new(),
        "contains_all" => match &a.values {
            Some(values) if !values.is_empty() => values
                .iter()
                .map(|v| format!("      {acc}.to_s.should contain({})\n", crystal_lit(v)))
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
        "is_empty" => format!("      {acc}.to_s.should be_empty\n"),
        "starts_with" => {
            let val = a.value.as_ref().map(crystal_lit).unwrap_or_else(|| "\"\"".into());
            format!("      {acc}.to_s.should start_with({val})\n")
        }
        "ends_with" => {
            let val = a.value.as_ref().map(crystal_lit).unwrap_or_else(|| "\"\"".into());
            format!("      {acc}.to_s.should end_with({val})\n")
        }
        "matches_regex" => {
            let val = a.value.as_ref().map(crystal_lit).unwrap_or_else(|| "\"\"".into());
            format!("      {acc}.to_s.should match({val})\n")
        }
        "greater_than" | "less_than" | "greater_than_or_equal" | "less_than_or_equal" => {
            let val = a.value.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "0".into());
            let op = match a.assertion_type.as_str() {
                "greater_than" => ">",
                "less_than" => "<",
                "greater_than_or_equal" => ">=",
                "less_than_or_equal" => "<=",
                _ => unreachable!(),
            };
            // Nilable accessors need `(expr || 0)` so Crystal can resolve
            // the comparison operator (the union `T | Nil` doesn't have `>=`).
            // Apply unconditionally — `(non_nilable || 0)` is a no-op for
            // non-nilable types (they're never falsy).
            let safe_acc = format!("({acc} || 0)");
            format!("      {safe_acc}.should be {op} {val}\n")
        }
        "min_length" | "max_length" | "count_equals" | "count_min" => {
            let val = a.value.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "0".into());
            let cmp = match a.assertion_type.as_str() {
                "count_equals" => "eq",
                "count_min" | "min_length" => "be >=",
                "max_length" => "be <=",
                _ => unreachable!(),
            };
            // For count assertions the field is an array (possibly nilable through
            // a `.try` chain). Call `.size` on the array and default to 0 when the
            // chain is nil — never `.to_s.size` (measures string length).
            if acc.contains(".try(") {
                format!("      ({acc}.try(&.size) || 0).should {cmp}({val})\n")
            } else {
                format!("      {acc}.size.should {cmp}({val})\n")
            }
        }
        "is_true" => format!("      {acc}.should be_true\n"),
        "is_false" => format!("      {acc}.should be_false\n"),
        "not_contains" => match &a.value {
            Some(serde_json::Value::String(s)) => format!("      {acc}.to_s.should_not contain({})\n", string_lit(s)),
            _ => "      # not_contains assertion requires a string value\n".to_string(),
        },
        "method_result" => {
            let method = a.method.as_deref().unwrap_or("(missing_method)");
            let method_args = build_method_args(a.args.as_ref());
            let call = format!("{acc}.{method}{method_args}");
            match a.check.as_deref() {
                Some("equals") => {
                    let val = a.value.as_ref().map(crystal_lit).unwrap_or_else(|| "nil".into());
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

/// Render an assertion for a void-returning function call. Since there is no
/// result variable, emit the assertion as a Crystal comment noting what
/// would be checked.
fn render_void_assertion(a: &crate::e2e::fixture::Assertion) -> String {
    let msg = match a.assertion_type.as_str() {
        "error" => String::new(),
        "equals" => format!(
            "expects {} to eq {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "not_empty" => format!("expects {} not to be empty", a.field.as_deref().unwrap_or("(result)")),
        "is_empty" => format!("expects {} to be empty", a.field.as_deref().unwrap_or("(result)")),
        "contains" => format!(
            "expects {} to contain {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "starts_with" => format!(
            "expects {} to start with {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "ends_with" => format!(
            "expects {} to end with {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "matches_regex" => format!(
            "expects {} to match {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "greater_than" => format!(
            "expects {} to be > {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "less_than" => format!(
            "expects {} to be < {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "min_length" => format!(
            "expects {} size >= {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "max_length" => format!(
            "expects {} size <= {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "count_equals" => format!(
            "expects {} size == {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "count_min" => format!(
            "expects {} size >= {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        "is_true" => format!("expects {} to be true", a.field.as_deref().unwrap_or("(result)")),
        "is_false" => format!("expects {} to be false", a.field.as_deref().unwrap_or("(result)")),
        "not_contains" => format!(
            "expects {} not to contain {:?}",
            a.field.as_deref().unwrap_or("(result)"),
            a.value
        ),
        other => format!("assertion type `{other}`"),
    };
    if msg.is_empty() {
        String::new()
    } else {
        format!("      # void-return: {msg}\n")
    }
}

/// Build the Crystal accessor for an assertion's optional dot-path field.
/// `None` → `{result_var}`; `"meta.title"` → `{result_var}.meta.title` (snake_cased).
/// Strip function-name wrapper namespace from field paths like
/// `crawl.pages_crawled`, `batch.completed_count`, `map.min_urls`.
fn strip_wrapper_namespace(field: Option<&str>) -> Option<&str> {
    field.and_then(|f| {
        let parts: Vec<&str> = f.splitn(2, '.').collect();
        if parts.len() == 2 && matches!(parts[0], "crawl" | "batch" | "map" | "content" | "robots") {
            Some(parts[1])
        } else {
            Some(f)
        }
    })
}

/// Fields that live on the inner extracted document (`results[0]`) of a wrapper
/// result (e.g. xberg's `ExtractionResult` → `ExtractedDocument`), referenced by
/// some fixtures without the `results[0].` prefix. Crystal-only resolution so the
/// fixture file stays shared across backends.
fn is_document_subfield(field: &str) -> bool {
    matches!(
        field,
        "structured_output"
            | "extracted_keywords"
            | "content"
            | "elements"
            | "summary"
            | "quality_score"
            | "chunks"
            | "tables"
            | "searchable"
    )
}

/// Known optional field prefixes in Crystal bindings. When an assertion field
/// path goes through one of these parents, the accessor uses `.try` so Crystal's
/// nil-safe type checking passes. Derived from common `fields_optional` patterns.
const OPTIONAL_PARENTS: &[&str] = &[
    "document", "metadata", "summary", "nodes", "results", "data", "elements",
    "keywords", "key_words", "extracted_keywords", "structured_output", "markdown",
    "downloaded_document", "response_meta", "extraction_meta", "screenshot", "usage",
    "tool_calls", "segments",
];

/// Fields on binary-content results (e.g. `speech.audio`, `file_content.content`):
/// the binding returns raw `Bytes`, so the field is the result itself.
const BINARY_RESULT_FIELDS: &[&str] = &["audio", "content"];

/// Discriminant variant names for tagged unions. When a field path goes through
/// one of these (e.g. `format.excel`), the accessor uses `.as?(ParentType::Variant)`
/// instead of `.try(&.variant_name)` since only that variant has the sub-fields.
const DISCRIMINANT_VARIANTS: &[&str] = &[
    "excel", "pdf", "docx", "pptx", "email", "archive", "image",
    "xml", "text", "html", "csv", "epub", "audio", "code",
    "ocr", "bibtex", "citation", "fiction_book", "dbf", "jats", "pst",
];

/// Map a discriminant variant name to its parent type for `as?` casting.
/// Returns `(parent_short_name, variant_type)` where variant_type includes
/// the parent namespace.
fn discriminant_variant_type(seg: &str) -> Option<(&'static str, &'static str)> {
    match seg {
        "excel" => Some(("format", "FormatMetadata::Excel")),
        "pdf" => Some(("format", "FormatMetadata::Pdf")),
        "docx" => Some(("format", "FormatMetadata::Docx")),
        "pptx" => Some(("format", "FormatMetadata::Pptx")),
        "email" => Some(("format", "FormatMetadata::Email")),
        "archive" => Some(("format", "FormatMetadata::Archive")),
        "image" => Some(("format", "FormatMetadata::Image")),
        "xml" => Some(("format", "FormatMetadata::Xml")),
        "text" => Some(("format", "FormatMetadata::Text")),
        "html" => Some(("format", "FormatMetadata::Html")),
        "csv" => Some(("format", "FormatMetadata::Csv")),
        "epub" => Some(("format", "FormatMetadata::Epub")),
        "audio" => Some(("format", "FormatMetadata::Audio")),
        "code" => Some(("format", "FormatMetadata::Code")),
        "ocr" => Some(("format", "FormatMetadata::Ocr")),
        "bibtex" => Some(("format", "FormatMetadata::Bibtex")),
        "citation" => Some(("format", "FormatMetadata::Citation")),
        "fiction_book" => Some(("format", "FormatMetadata::FictionBook")),
        "dbf" => Some(("format", "FormatMetadata::Dbf")),
        "jats" => Some(("format", "FormatMetadata::Jats")),
        "pst" => Some(("format", "FormatMetadata::Pst")),
        _ => None,
    }
}

/// Known array field names in the Crystal binding. When accessed without `[]` or `_N`
/// index in the field path, Crystal needs an implicit `[0]` to reach subfields.
const ARRAY_FIELDS: &[&str] = &[
    "json_ld", "links", "images", "feeds", "assets", "cookies",
    "pages", "urls", "results", "action_results",
];

fn field_accessor(field: Option<&str>, result_var: &str) -> String {
    field_accessor_with_module(field, result_var, "", &std::collections::HashSet::new())
}

fn field_accessor_with_module(
    field: Option<&str>,
    result_var: &str,
    module_name: &str,
    display_as_text: &std::collections::HashSet<String>,
) -> String {
    use heck::ToSnakeCase;
    match field {
        None => result_var.to_string(),
        Some(path) => {
            // Binary-content methods (speech, file_content) return raw `Bytes`;
            // fixture assertions reference the payload field (`audio`/`content`)
            // which is the result itself, not a struct field.
            let root = path.split(['.', '[']).next().unwrap_or(path);
            if BINARY_RESULT_FIELDS.contains(&root) {
                return result_var.to_string();
            }
            let mut acc = result_var.to_string();
            let segments: Vec<&str> = path.split('.').filter(|s| !s.is_empty()).collect();

            // Handle metadata namespace flattening: Crystal binds og/twitter/dc fields
            // as prefix-named getters on `metadata` (e.g. `og.title` → `metadata.og_title`).
            if segments.len() >= 2 && !path.contains("[]") {
                let prefix = segments[0];
                let sub = segments[1].to_snake_case();
                let prefix_mapped = match (prefix, sub.as_str()) {
                    ("og", _) => Some(format!("metadata.og_{sub}")),
                    ("twitter", "card_type") => Some("metadata.twitter_card".to_string()),
                    ("twitter", _) => Some(format!("metadata.twitter_{sub}")),
                    ("dublin_core", _) => Some(format!("metadata.dc_{sub}")),
                    ("article", _) => Some(format!("metadata.article.{sub}")),
                    _ => None,
                };
                if let Some(mapped) = prefix_mapped {
                    acc.push('.');
                    acc.push_str(&mapped);
                    for seg in &segments[2..] {
                        acc.push('.');
                        acc.push_str(&seg.to_snake_case());
                    }
                    return acc;
                }
                // Implicit first-element access for known array fields (e.g. `json_ld.type`).
                // Skip array-level properties like `links.length` (should become `links.size`).
                if ARRAY_FIELDS.contains(&prefix) && sub != "length" && sub != "size" {
                    let sub_snake = sub.to_snake_case();
                    let sub_mapped = match sub_snake.as_str() {
                        "type" => "schema_type",
                        "category" => "asset_category",
                        other => other,
                    };
                    acc.push('.');
                    acc.push_str(prefix);
                    acc.push_str("[0].");
                    // If the array's sub-field is optional (nilable), use
                    // try so Crystal's nil-safe type checking passes.
                    if OPTIONAL_PARENTS.contains(&segments[1]) {
                        acc.push_str("try(&.");
                        acc.push_str(&sub_mapped);
                        acc.push(')');
                    } else {
                        acc.push_str(&sub_mapped);
                    }
                    for seg in &segments[2..] {
                        // Array sub-fields may be nilable; use try for
                        // Crystal nil-safe type checking.
                        acc.push_str(".try(&.");
                        acc.push_str(&seg.to_snake_case());
                        acc.push(')');
                    }
                    return acc;
                }
            }

            let mut in_try_chain = false;
            let mut parent_seg = String::new();
            for raw_seg in path.split('.').filter(|s| !s.is_empty()) {
                let seg = raw_seg.to_snake_case();

                // Once a nilable parent is encountered, use `.try(&.field)`
                // for all subsequent segments so Crystal's nil-safe type
                // system accepts the chain.
                if OPTIONAL_PARENTS.contains(&raw_seg) {
                    in_try_chain = true;
                }
                if in_try_chain {
                    // For `.size` inside a try chain, use `.to_a.size` so the
                    // result is always an Int (empty array if nil) instead of
                    // `Int | Nil` which would fail `should be >= N` assertions.
                    if seg == "size" {
                        acc.push_str(".to_a.size");
                        continue;
                    }
                    // Discriminant variant access: when the parent is `format`,
                    // and this segment is a known variant name, downcast with
                    // `as?` instead of `try(&.name)` (only that variant has
                    // the sub-fields).
                    if parent_seg == "format" && DISCRIMINANT_VARIANTS.contains(&raw_seg) {
                        if let Some((_, ty)) = discriminant_variant_type(raw_seg) {
                            acc.push_str(".as?(");
                            if !module_name.is_empty() {
                                acc.push_str(module_name);
                                acc.push_str("::");
                            }
                            acc.push_str(ty);
                            acc.push(')');
                            parent_seg = raw_seg.to_string();
                            continue;
                        }
                    }
                    acc.push_str(".try(&.");
                    // Crystal arrays use `.size` (no `Array#length`).
                    if seg == "length" {
                        acc.push_str("size");
                    } else {
                        acc.push_str(&seg);
                    }
                    acc.push(')');
                    parent_seg = raw_seg.to_string();
                    continue;
                }
                parent_seg = raw_seg.to_string();

                // Crystal uses `size` for array length (no `Array#length`).
                if seg == "length" {
                    acc.push_str(".size");
                    continue;
                }
                // Virtual fields: map to their concrete Crystal equivalents.
                match seg.as_str() {
                    "pages_crawled" | "min_pages" => {
                        acc.push_str(".pages.size");
                        continue;
                    }
                    "min_urls" => {
                        acc.push_str(".urls.size");
                        continue;
                    }
                    _ => {}
                }
                // Field renames: Crystal binding renames `type` (reserved keyword) to
                // `type_` / `schema_type`, and `category` to `asset_category`.
                if seg == "type" {
                    acc.push_str(".schema_type");
                    continue;
                }
                if seg == "category" {
                    acc.push_str(".asset_category");
                    continue;
                }
                // Array index access: `results[0]` -> `results[0]`.
                // Also sets in_try_chain if the base array name is in OPTIONAL_PARENTS.
                if let Some(open_bracket) = seg.find('[') {
                    if let Some(close_bracket) = seg.find(']') {
                        let base = &seg[..open_bracket];
                        let index_str = &seg[open_bracket + 1..close_bracket];
                        if let Ok(_) = index_str.parse::<usize>() {
                            if OPTIONAL_PARENTS.contains(&base) {
                                in_try_chain = true;
                                // Nil-safe index access on a nilable array.
                                acc.push('.');
                                acc.push_str(base);
                                acc.push_str(".try(&.[");
                                acc.push_str(index_str);
                                acc.push_str("])");
                            } else {
                                acc.push('.');
                                acc.push_str(base);
                                acc.push('[');
                                acc.push_str(index_str);
                                acc.push(']');
                            }
                            continue;
                        }
                    }
                }
                // Array index access: `pages_0` -> `pages[0]`.
                if let Some(underscore) = seg.rfind('_') {
                    let base = &seg[..underscore];
                    let index_str = &seg[underscore + 1..];
                    if let Ok(_) = index_str.parse::<usize>() {
                        if OPTIONAL_PARENTS.contains(&base) {
                            in_try_chain = true;
                            // Nil-safe index access on a nilable array.
                            acc.push('.');
                            acc.push_str(base);
                            acc.push_str(".try(&.[");
                            acc.push_str(index_str);
                            acc.push_str("])");
                        } else {
                            acc.push('.');
                            acc.push_str(base);
                            acc.push('[');
                            acc.push_str(index_str);
                            acc.push(']');
                        }
                        continue;
                    }
                }
                acc.push('.');
                acc.push_str(&seg);
            }
            // Display-as-text fields carry a discriminated content union rather than
            // a plain string (e.g. `AssistantContent` with a `Text` variant). Downcast
            // to the text variant and pull its string value.
            if !display_as_text.is_empty() && path.split(['.', '[']).next().is_some_and(|r| r != "results")
                && display_as_text.contains(path) {
                // Exact field match: append text extraction.
                let module_prefix = if module_name.is_empty() {
                    String::new()
                } else {
                    format!("{module_name}::")
                };
                acc.push_str(&format!(
                    ".as?({module_prefix}AssistantContent::Text).try(&.value)"
                ));
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
        other => string_lit(&other.to_string()),
    }
}

/// Build a parenthesised Crystal method-call argument list from a JSON array,
/// or empty string for no/non-array args.
/// Return the Crystal method signature for a visitor callback method name.
/// Maps the fixture callback name to the exact abstract class signature.
fn crystal_visitor_method_signature(method_name: &str) -> String {
    match method_name {
        "visit_text" => "def visit_text(ctx : HtmlVisitorVisitorContext, text : String) : VisitResult".to_string(),
        "visit_element_start" => "def visit_element_start(ctx : HtmlVisitorVisitorContext) : VisitResult".to_string(),
        "visit_element_end" => "def visit_element_end(ctx : HtmlVisitorVisitorContext, output : String) : VisitResult".to_string(),
        "visit_link" => "def visit_link(ctx : HtmlVisitorVisitorContext, href : String, text : String, title : String?) : VisitResult".to_string(),
        "visit_image" => "def visit_image(ctx : HtmlVisitorVisitorContext, src : String, alt : String, title : String?) : VisitResult".to_string(),
        "visit_heading" => "def visit_heading(ctx : HtmlVisitorVisitorContext, level : UInt32, text : String, id : String?) : VisitResult".to_string(),
        "visit_code_block" => "def visit_code_block(ctx : HtmlVisitorVisitorContext, lang : String?, code : String) : VisitResult".to_string(),
        "visit_code_inline" => "def visit_code_inline(ctx : HtmlVisitorVisitorContext, code : String) : VisitResult".to_string(),
        "visit_list_item" => "def visit_list_item(ctx : HtmlVisitorVisitorContext, ordered : Bool, marker : String, text : String) : VisitResult".to_string(),
        "visit_list_start" => "def visit_list_start(ctx : HtmlVisitorVisitorContext, ordered : Bool) : VisitResult".to_string(),
        "visit_list_end" => "def visit_list_end(ctx : HtmlVisitorVisitorContext, ordered : Bool, output : String) : VisitResult".to_string(),
        "visit_table_start" => "def visit_table_start(ctx : HtmlVisitorVisitorContext) : VisitResult".to_string(),
        "visit_table_row" => "def visit_table_row(ctx : HtmlVisitorVisitorContext, cells : String, is_header : Bool) : VisitResult".to_string(),
        "visit_table_end" => "def visit_table_end(ctx : HtmlVisitorVisitorContext, output : String) : VisitResult".to_string(),
        "visit_blockquote" => "def visit_blockquote(ctx : HtmlVisitorVisitorContext, content : String, depth : LibC::SizeT) : VisitResult".to_string(),
        "visit_strong" => "def visit_strong(ctx : HtmlVisitorVisitorContext, text : String) : VisitResult".to_string(),
        "visit_emphasis" => "def visit_emphasis(ctx : HtmlVisitorVisitorContext, text : String) : VisitResult".to_string(),
        "visit_strikethrough" => "def visit_strikethrough(ctx : HtmlVisitorVisitorContext, text : String) : VisitResult".to_string(),
        "visit_underline" => "def visit_underline(ctx : HtmlVisitorVisitorContext, text : String) : VisitResult".to_string(),
        "visit_subscript" => "def visit_subscript(ctx : HtmlVisitorVisitorContext, text : String) : VisitResult".to_string(),
        "visit_superscript" => "def visit_superscript(ctx : HtmlVisitorVisitorContext, text : String) : VisitResult".to_string(),
        "visit_mark" => "def visit_mark(ctx : HtmlVisitorVisitorContext, text : String) : VisitResult".to_string(),
        "visit_line_break" => "def visit_line_break(ctx : HtmlVisitorVisitorContext) : VisitResult".to_string(),
        "visit_horizontal_rule" => "def visit_horizontal_rule(ctx : HtmlVisitorVisitorContext) : VisitResult".to_string(),
        "visit_custom_element" => "def visit_custom_element(ctx : HtmlVisitorVisitorContext, tag_name : String, html : String) : VisitResult".to_string(),
        "visit_definition_list_start" => "def visit_definition_list_start(ctx : HtmlVisitorVisitorContext) : VisitResult".to_string(),
        "visit_definition_term" => "def visit_definition_term(ctx : HtmlVisitorVisitorContext, text : String) : VisitResult".to_string(),
        "visit_definition_description" => "def visit_definition_description(ctx : HtmlVisitorVisitorContext, text : String) : VisitResult".to_string(),
        "visit_definition_list_end" => "def visit_definition_list_end(ctx : HtmlVisitorVisitorContext, output : String) : VisitResult".to_string(),
        "visit_form" => "def visit_form(ctx : HtmlVisitorVisitorContext, action : String?, method : String?) : VisitResult".to_string(),
        "visit_input" => "def visit_input(ctx : HtmlVisitorVisitorContext, input_type : String, name : String?, value : String?) : VisitResult".to_string(),
        "visit_button" => "def visit_button(ctx : HtmlVisitorVisitorContext, text : String) : VisitResult".to_string(),
        "visit_audio" => "def visit_audio(ctx : HtmlVisitorVisitorContext, src : String?) : VisitResult".to_string(),
        "visit_video" => "def visit_video(ctx : HtmlVisitorVisitorContext, src : String?) : VisitResult".to_string(),
        "visit_iframe" => "def visit_iframe(ctx : HtmlVisitorVisitorContext, src : String?) : VisitResult".to_string(),
        "visit_details" => "def visit_details(ctx : HtmlVisitorVisitorContext, open : Bool) : VisitResult".to_string(),
        "visit_summary" => "def visit_summary(ctx : HtmlVisitorVisitorContext, text : String) : VisitResult".to_string(),
        "visit_figure_start" => "def visit_figure_start(ctx : HtmlVisitorVisitorContext) : VisitResult".to_string(),
        "visit_figcaption" => "def visit_figcaption(ctx : HtmlVisitorVisitorContext, text : String) : VisitResult".to_string(),
        "visit_figure_end" => "def visit_figure_end(ctx : HtmlVisitorVisitorContext, output : String) : VisitResult".to_string(),
        _ => format!("def {method_name}(ctx : HtmlVisitorVisitorContext, *args : String) : VisitResult"),
    }
}

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
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            // Escape `#{` to prevent Crystal string interpolation
            '#' if chars.peek() == Some(&'{') => {
                out.push_str("\\#");
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Escape a string for use inside a Crystal double-quoted string literal.
/// Crystal uses the same escape sequences as C: `\"` for double-quote,
/// `\\` for backslash, `\n` for newline, `\t` for tab.
fn escape_crystal_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            // Escape `#{` to prevent Crystal string interpolation
            '#' if chars.peek() == Some(&'{') => {
                out.push_str("\\#");
            }
            c => out.push(c),
        }
    }
    out
}

/// Generate a unique Crystal class name for a fixture's visitor.
fn crystal_visitor_class_name(fixture: &Fixture) -> String {
    let sanitized: String = fixture.id.chars().filter(|c| c.is_alphanumeric()).collect();
    if sanitized.starts_with(|c: char| c.is_uppercase()) {
        format!("TestVisitor{sanitized}")
    } else {
        let mut chars = sanitized.chars();
        let first = chars.next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        format!("TestVisitor{first}{}", chars.as_str())
    }
}

/// Emit a Crystal visitor class at the spec-file top level (class declarations
/// are not allowed inside `it` blocks in Crystal).
fn emit_crystal_visitor_class(
    out: &mut String,
    fixture: &Fixture,
    visitor_spec: &crate::e2e::fixture::VisitorSpec,
    module_name: &str,
) {
    let visitor_class = crystal_visitor_class_name(fixture);
    // Wrap in module so that types like HtmlVisitorVisitorContext are in scope.
    out.push_str(&format!("module {module_name}\n"));
    out.push_str(&format!("  class {visitor_class} < HtmlVisitorVisitor\n"));
    for (method_name, action) in &visitor_spec.callbacks {
        let sig = crystal_visitor_method_signature(method_name);
        let body = match action {
            crate::e2e::fixture::CallbackAction::Skip => {
                format!("VisitResult::Skip.new")
            }
            crate::e2e::fixture::CallbackAction::Continue => {
                format!("VisitResult::Continue.new")
            }
            crate::e2e::fixture::CallbackAction::PreserveHtml => {
                format!("VisitResult::PreserveHtml.new")
            }
            crate::e2e::fixture::CallbackAction::Custom { output } => {
                let escaped = output.replace('\\', "\\\\").replace('"', "\\\"");
                format!("VisitResult::Custom.new(\"{escaped}\")")
            }
            crate::e2e::fixture::CallbackAction::CustomTemplate { .. } => {
                format!("VisitResult::Continue.new")
            }
        };
        out.push_str(&format!("    {sig}\n      {body}\n    end\n"));
    }
    out.push_str("  end\n");
    out.push_str("end\n\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── resolve_json_field ─────────────────────────────────────────────

    #[test]
    fn resolve_simple_field() {
        let input = serde_json::json!({"name": "alice"});
        let v = resolve_json_field(&input, "name");
        assert_eq!(v, &serde_json::json!("alice"));
    }

    #[test]
    fn resolve_nested_field() {
        let input = serde_json::json!({"meta": {"title": "hello"}});
        let v = resolve_json_field(&input, "meta.title");
        assert_eq!(v, &serde_json::json!("hello"));
    }

    #[test]
    fn resolve_deeply_nested_field() {
        let input = serde_json::json!({"a": {"b": {"c": 42}}});
        let v = resolve_json_field(&input, "a.b.c");
        assert_eq!(v, &serde_json::json!(42));
    }

    #[test]
    fn resolve_missing_field_returns_null() {
        let input = serde_json::json!({"x": 1});
        let v = resolve_json_field(&input, "y");
        assert_eq!(v, &serde_json::Value::Null);
    }

    #[test]
    fn resolve_path_through_non_object_returns_null() {
        let input = serde_json::json!({"x": "hello"});
        let v = resolve_json_field(&input, "x.y");
        assert_eq!(v, &serde_json::Value::Null);
    }

    #[test]
    fn resolve_empty_path_returns_input() {
        let input = serde_json::json!({"x": 1});
        let v = resolve_json_field(&input, "");
        assert_eq!(v, &input);
    }

    // ── escape_crystal_string ──────────────────────────────────────────

    #[test]
    fn escape_plain_string() {
        assert_eq!(escape_crystal_string("hello"), "hello");
    }

    #[test]
    fn escape_quote() {
        assert_eq!(escape_crystal_string("a\"b"), "a\\\"b");
    }

    #[test]
    fn escape_backslash() {
        assert_eq!(escape_crystal_string("a\\b"), "a\\\\b");
    }

    #[test]
    fn escape_newline() {
        assert_eq!(escape_crystal_string("a\nb"), "a\\nb");
    }

    #[test]
    fn escape_tab() {
        assert_eq!(escape_crystal_string("a\tb"), "a\\tb");
    }

    #[test]
    fn escape_carriage_return() {
        assert_eq!(escape_crystal_string("a\rb"), "a\\rb");
    }

    #[test]
    fn escape_all_specials() {
        assert_eq!(escape_crystal_string("\"\\\n\t\r"), "\\\"\\\\\\n\\t\\r");
    }

    // ── string_lit ─────────────────────────────────────────────────────

    #[test]
    fn string_lit_wraps_in_quotes() {
        assert_eq!(string_lit("hi"), "\"hi\"");
    }

    #[test]
    fn string_lit_escapes_specials() {
        assert_eq!(string_lit("a\"b"), "\"a\\\"b\"");
    }

    // ── crystal_lit ────────────────────────────────────────────────────

    #[test]
    fn crystal_lit_null() {
        assert_eq!(crystal_lit(&serde_json::json!(null)), "nil");
    }

    #[test]
    fn crystal_lit_bool_true() {
        assert_eq!(crystal_lit(&serde_json::json!(true)), "true");
    }

    #[test]
    fn crystal_lit_bool_false() {
        assert_eq!(crystal_lit(&serde_json::json!(false)), "false");
    }

    #[test]
    fn crystal_lit_integer() {
        assert_eq!(crystal_lit(&serde_json::json!(42)), "42");
    }

    #[test]
    fn crystal_lit_float() {
        assert_eq!(crystal_lit(&serde_json::json!(3.14)), "3.14");
    }

    #[test]
    fn crystal_lit_string() {
        assert_eq!(crystal_lit(&serde_json::json!("hello")), "\"hello\"");
    }

    #[test]
    fn crystal_lit_object_falls_back_to_json_string() {
        let lit = crystal_lit(&serde_json::json!({"a": 1}));
        assert!(lit.starts_with('"'));
        assert!(lit.ends_with('"'));
        assert!(lit.contains("\\\"a\\\""));
    }

    #[test]
    fn crystal_lit_array_falls_back_to_json_string() {
        let lit = crystal_lit(&serde_json::json!([1, 2, 3]));
        assert!(lit.starts_with('"'));
        assert!(lit.ends_with('"'));
        assert!(lit.contains("1"));
    }

    // ── field_accessor ─────────────────────────────────────────────────

    #[test]
    fn field_accessor_no_field_uses_result_var() {
        assert_eq!(field_accessor(None, "r"), "r");
    }

    #[test]
    fn field_accessor_single_field() {
        assert_eq!(field_accessor(Some("name"), "res"), "res.name");
    }

    #[test]
    fn field_accessor_nested_path() {
        assert_eq!(field_accessor(Some("meta.title"), "__result"), "__result.meta.title");
    }

    #[test]
    fn field_accessor_pascal_case_to_snake_case() {
        assert_eq!(field_accessor(Some("UserProfile"), "r"), "r.user_profile");
    }

    #[test]
    fn field_accessor_trailing_dot_ignored() {
        assert_eq!(field_accessor(Some("a."), "r"), "r.a");
    }

    #[test]
    fn field_accessor_empty_segments_ignored() {
        assert_eq!(field_accessor(Some("a..b"), "r"), "r.a.b");
    }

    // ── build_method_args ──────────────────────────────────────────────

    #[test]
    fn build_method_args_none_returns_empty() {
        assert_eq!(build_method_args(None), "");
    }

    #[test]
    fn build_method_args_empty_array_returns_empty() {
        let arr = serde_json::json!([]);
        assert_eq!(build_method_args(Some(&arr)), "");
    }

    #[test]
    fn build_method_args_single_element() {
        let arr = serde_json::json!([0]);
        assert_eq!(build_method_args(Some(&arr)), "(0)");
    }

    #[test]
    fn build_method_args_multiple_elements() {
        let arr = serde_json::json!([1, "two", true]);
        assert_eq!(build_method_args(Some(&arr)), "(1, \"two\", true)");
    }
}
