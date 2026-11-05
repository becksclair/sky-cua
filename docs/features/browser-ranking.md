# Optional browser target ranking

## Status

Implemented, opt-in. Default off. Local verification: 2026-11-05.

## Summary

`browser_rank` resolves a target-selection intent against a fresh browser
observation. It tries explicit deterministic matching, Jev at 0.8, then Terra.
It returns an observation-bound reference for the normal browser action tools;
ranking itself performs no click or typing.

## Contract surface

Set top-level `browser_ranking = true` in the existing
`~/.config/sky-cua/sky-cua.toml` for a durable opt-in across hosts. Alternatively,
enable `SKY_CUA_BROWSER_RANKING=1` in the MCP process environment; an explicit
environment setting takes precedence, including `0` to disable it. The tool is
absent by default and when the browser surface is disabled. Direct calls while
disabled are rejected. The installer forwards explicit opt-in configuration.

Inputs:

- `tab_id`: owned/claimed tab from the existing browser tools.
- `intent`: the target-selection task, known before invoking a large model.
- `target`: optional; existing `user_chrome` target.
- `exact_name`: optional literal observed label. `exact_role` can refine it and
  requires `exact_name`. Free-form intent is never treated as an exact label.

Results include `status`, `source` (`deterministic`, `jev`, or `terra`), `tab_id`,
`appshot_id`, `fallback_reason`, and per-provider/total `timing_ms`.
`status="selected"` includes `ref`; `unresolved` never includes an actionable ref.
Jev selections also include their selected probability.

Use the selected `ref` and `appshot_id` directly with `browser_input`. Existing
tab ownership, document generation, ref resolution and actionability checks
still apply. Successful ranking is not proof of task completion.

## Behavior

The client obtains structured browser semantics without sending screenshots to
models. Only enabled controls with finite positive bounds and observed CSS
visibility are candidates. Transparent native input hit areas remain eligible
because applications commonly draw their visible checkbox UI separately.
Model inputs contain labels, literal roles/tags, bounds, link destinations as
candidate context, the observed page text once in `state.page_text`, and
temporary choice IDs; they exclude executable refs and snapshot tokens. Sending
page text once keeps payloads bounded on candidate-heavy pages; see
[shared page text](../research/2026-11-jev-shared-page-text.md).

A unique explicit exact match skips both providers. Empty candidate sets return
unresolved without a provider request. Otherwise Jev 1.13 is accepted only for
a known non-abstain choice with finite probability in `[0.8, 1]`. Abstention,
low/invalid probability, unknown choices, malformed replies, and transport or
credential errors fall through to Terra (`gpt-6.1-terra`, medium). Terra failure
or abstention returns unresolved. No automatic model-generated selector or
coordinate fallback is used.

After selecting a candidate the client observes again. It must find the same
document generation, opaque ref, name and role in a usable state; otherwise it returns
`target_changed` without a ref. It does not rebind a changed target by label.

### Provider configuration

Jev uses `OPENCODE_API_KEY`, then `OPENCODE_ZEN_API_KEY`, then the existing
`opencode` API-key entry in `$XDG_DATA_HOME/opencode/auth.json` (default
`~/.local/share/opencode/auth.json`). The store is read only when no environment
key is supplied; other providers and credential types are ignored. A failed
request does not retry with another credential. It uses the existing OpenCode
System One endpoint. Enabling this feature authorizes sending
the supplied intent and browser text/candidate metadata to these model routes.
Keep credentials in the environment or provider store, not source or command-line values.

Terra uses `codex` on PATH and the existing login under `CODEX_HOME` (otherwise
`~/.codex`). A cached `gpt-6.1-terra` entry in `models_cache.json` is required.
The adapter creates an isolated temporary home with copied auth and a restricted
model catalog. Shell, editing, browsing, apps, plugins, skills, collaboration and
other tools are disabled; the outgoing tool-free request configuration was
verified against the installed CLI. The catalog preserves model behavior except
tool exposure. Temporary credentials and outputs are removed after the request.

The Jev request has a 30-second total HTTP budget. The Terra subprocess is
bounded to 60 seconds, with bounded output and kill/reap cleanup. Hosts invoking
the tool should allow the combined provider and observation budgets. Unsupported
Codex configuration, missing cache/login, or provider errors return unresolved
rather than switching accounts or providers silently.

### Reusable Rust client

`sky-cua-model-client` is provider-neutral: `ModelClient::post_json` accepts an
endpoint, headers and arbitrary JSON and returns JSON plus elapsed time. It
reuses HTTP connections, bounds responses and timeouts, and exposes safe errors
without URLs, bodies or credentials. Provider payload/response formats remain
in the thin caller adapters. See the crate README for a direct-call example.

## Source paths

- `crates/sky-cua-model-client/`: generic JSON transport and offline tests.
- `crates/sky-cua-client/src/browser_rank_models.rs`: Jev adapter and shared pool.
- `crates/sky-cua-client/src/browser_rank_models/terra.rs`: tool-free Terra adapter.
- `crates/sky-cua-client/src/mcp_tools/browser_ranking.rs`: routing and live ref validation.
- `crates/sky-cua-client/src/mcp_tools/definitions/`: opt-in registry and schemas.
- `scripts/live_browser_ranking_smoke.py`: actual MCP/Chrome workflow acceptance.

## Verification

Offline tests cover transport reuse, timeout/error/body bounds, exact matching,
threshold boundaries, malformed/unknown choices, provider fallback, disabled
registration/dispatch, changed refs/documents, and Terra isolation/cleanup. Native request
capture for the tool-free Codex configuration is retained under
`artifacts/jev-integration-toolset/`.

The live smoke uses a disposable Chromium profile and unmodified TodoMVC
application through the real extension/native-host/service path. It checks the
disabled default, positive-size CSS-hidden control exclusion, deterministic and Jev selection, Terra fallback, abstention,
and independent final task state. Local evidence is stored under
`artifacts/jev-browser-integration/`.

The passing integration run is `e2e5/results.json`: two deterministic selections
made no model calls; Jev selected the add field and Active filter at probabilities
1.0 and 0.99; Terra selected the individual checkbox after Jev abstained. All five
returned targets were correct. A sixth, absent-target request returned unresolved
after both models abstained. Final task completion and filtering matched the
independent DOM oracle. The run also checked a positive-size hidden button was
excluded while a transparent hit target remained usable.

These are functional acceptance results, not a new latency comparison. The Jev
selections took 775 ms and 447 ms including observations; the Terra checkbox
fallback took 3297 ms. The prior paired pilot remains the comparative evidence.

The same workflow also passed against the staged release client and service:
`artifacts/jev-browser-integration/e2e-release/results.json`. The release source
fingerprint, binary equality, environment forwarding and bundled skill content
were checked. `cargo fmt --check`, workspace Clippy and all 1,996 Rust tests
passed; Ruff, basedpyright and all 1,101 Python tests passed. The bundle is local
test output, not an installation or a global opt-in change.

The subsequent installed acceptance is
`artifacts/jev-browser-deploy/installed-e2e/results.json`. It passed with the
ranking environment override and both OpenCode key variables removed, proving
the durable machine setting and provider-store lookup through the installed
client, service and native host. Jev handled two semantic selections; Terra
handled the individual checkbox after Jev abstained. The absent-target request
returned unresolved. No wrong targets or actions were observed. The updated
configuration and credential regressions bring the Rust suite to 2,002 passing
tests; all 1,101 Python tests, formatting, Clippy and type checks passed.

The shared page-text payload was re-measured against the same TodoMVC evidence:
a controlled Jev A/B kept 36/36 accepted selections correct with zero wrong
accepts, a replicated paired workflow saved 25.2% with Terra calls 9→3, and the
live smoke passed with unchanged routing. Details:
[shared page text](../research/2026-11-jev-shared-page-text.md).

## Known limitations

This is target ranking for caller-known intents, not task planning or outcome
judging. It does not cover native desktop or phone use. Useful savings require
replacing an otherwise necessary large-model selection call; invoking it after
that work is already done adds overhead. Native provider/codex transport can
fail or time out; callers must handle unresolved results.

The initial live workload is small and controlled. The prior real-browser pilot
needed Terra for unnamed checkbox selection, so retaining the confidence gate
and fallback is essential. No feature default or installed runtime is changed
by enabling it only in an isolated test process.

## Related

- [Browser MCP tools](browser-mcp-tools.md)
- [Real browser workflow results](../research/2026-11-jev-real-browser-workflow.md)
- [Model transport](../../crates/sky-cua-model-client/README.md)
