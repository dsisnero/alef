//! Crystal HTTP test-client rendering via the shared [`client::TestClientRenderer`] trait.

use super::*;
use crate::e2e::codegen::client;
use std::fmt::Write as _;

struct CrystalTestClientRenderer;

impl client::TestClientRenderer for CrystalTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "crystal"
    }

    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        if let Some(reason) = skip_reason {
            let _ = writeln!(out, "    pending \"{fn_name}: {reason}\"");
        } else {
            let _ = writeln!(out, "    it {description:?} do");
        }
    }

    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "    end");
    }

    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method;
        let _ = writeln!(out, "      base_url = ENV[\"SUT_URL\"]? || \"http://127.0.0.1:8012\"");
        let _ = writeln!(
            out,
            "      url = \"{{}}/fixtures/{{}}\" % {{base_url, {:?}}}",
            ctx.path.trim_start_matches("/fixtures/")
        );

        if !ctx.query_params.is_empty() {
            let qs: Vec<String> = ctx
                .query_params
                .iter()
                .map(|(k, v)| match v {
                    serde_json::Value::String(s) => format!("{k}={s}"),
                    other => format!("{k}={other}"),
                })
                .collect();
            let _ = writeln!(out, "      url += \"?{}\"", qs.join("&"));
        }

        let method_requires_body = matches!(method, "POST" | "PUT" | "PATCH");
        let emit_payload = ctx.body.is_some() || method_requires_body;

        let _ = writeln!(out, "      headers = HTTP::Headers.new");

        if emit_payload {
            let ct = ctx.content_type.unwrap_or("application/json");
            let _ = writeln!(out, "      headers[\"Content-Type\"] = {ct:?}");
        }
        for (key, val) in ctx.headers {
            let _ = writeln!(out, "      headers[{key:?}] = {val:?}");
        }

        if let Some(body) = ctx.body {
            let body_str = serde_json::to_string(body).unwrap_or_default();
            let escaped = escape_crystal_str(&body_str);
            let _ = writeln!(out, "      body = \"{escaped}\"");
        } else if emit_payload {
            let _ = writeln!(out, "      body = \"\"");
        }

        if emit_payload {
            let _ = writeln!(
                out,
                "      {rv} = HTTP::Client.{method_lc}(url, headers: headers, body: body)",
                rv = ctx.response_var,
                method_lc = method.to_lowercase(),
            );
        } else {
            let _ = writeln!(
                out,
                "      {rv} = HTTP::Client.{method_lc}(url, headers: headers)",
                rv = ctx.response_var,
                method_lc = method.to_lowercase(),
            );
        }
    }

    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16) {
        let _ = writeln!(out, "      {response_var}.status_code.should eq({status})");
    }

    fn render_assert_header(&self, out: &mut String, response_var: &str, name: &str, expected: &str) {
        let name_lower = name.to_lowercase();
        match expected {
            "<<present>>" => {
                let _ = writeln!(out, "      {response_var}.headers[{name_lower:?}].should_not be_nil");
            }
            "<<absent>>" => {
                let _ = writeln!(out, "      {response_var}.headers[{name_lower:?}]?.should be_nil");
            }
            "<<uuid>>" => {
                let _ = writeln!(
                    out,
                    "      {response_var}.headers[{name_lower:?}].should match(/\\A[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}\\z/i)"
                );
            }
            exact => {
                let _ = writeln!(out, "      {response_var}.headers[{name_lower:?}].should eq({exact:?})");
            }
        }
    }

    fn render_assert_json_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        match expected {
            serde_json::Value::String(s) => {
                let escaped = escape_crystal_str(s);
                let _ = writeln!(out, "      {response_var}.body.should eq({escaped:?})");
            }
            other => {
                let expected_str = serde_json::to_string(other).unwrap_or_default();
                let escaped = escape_crystal_str(&expected_str);
                let _ = writeln!(
                    out,
                    "      JSON.parse({response_var}.body).should eq(JSON.parse({escaped:?}))"
                );
            }
        }
    }

    fn render_assert_partial_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let _ = writeln!(out, "      parsed = JSON.parse({response_var}.body)");
            for (key, val) in obj {
                let eval = serde_json::to_string(val).unwrap_or_default();
                let escaped = escape_crystal_str(&eval);
                let _ = writeln!(out, "      parsed[{key:?}].should eq(JSON.parse({escaped:?}))");
            }
        }
    }

    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        response_var: &str,
        errors: &[crate::e2e::fixture::ValidationErrorExpectation],
    ) {
        let _ = writeln!(out, "      errors = JSON.parse({response_var}.body)[\"errors\"]?.as_a");
        for ve in errors {
            let loc = ve.loc.join(".");
            let _ = writeln!(
                out,
                "      errors.should contain(err) {{ |e| e[\"loc\"].as_a?.join(\".\") == {loc:?} && e[\"msg\"] == {msg:?} }}",
                msg = ve.msg,
            );
        }
    }
}

fn escape_crystal_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
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
    out
}

pub(super) fn render_http_test_spec(out: &mut String, fixture: &Fixture) -> bool {
    client::http_call::render_http_test(out, &CrystalTestClientRenderer, fixture)
}
