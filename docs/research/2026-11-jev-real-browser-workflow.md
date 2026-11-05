# Jev versus Terra in a real browser task workflow

## Context

On 2026-11-05, test whether Jev at 0.8 actually avoids Terra calls and improves
complete browser workflow time. The earlier browser fixtures used Luna; this
comparison uses **gpt-6.1-terra, medium**, through the installed sky-cua browser
path, following the unsuccessful [real native workflow](2026-11-jev-real-edit-save-results.md).

## Investigation

### Application and real consumer path

The application is the unmodified [TodoMVC React distributed build](https://todomvc.com/examples/react/dist/),
downloaded as HTML, JavaScript, CSS, and its base helper. Asset hashes were frozen
before inference. It runs from loopback in a disposable Chromium profile with
the real Codex extension and installed sky-cua native host. Each workflow gets a
distinct loopback origin and fresh tab, ensuring empty task state. This is a
controlled local copy of a real application, not the previously authored target
fixture. Ancillary learn-sidebar/favicon resources were not mirrored.

All observations and actions use the actual installed MCP path: `browser_open`,
`observe(surface="browser")`, and fresh-fenced `browser_input` with opaque refs.
No standalone Playwright driver or direct CDP action performs the workflow.
Independent CDP evaluation is read-only and used solely for outcome/target proof.
Model transcripts contain only final messages, with no model tool execution.

The caller fixes the next intent before inference:

1. Select the task-entry field and add “Pack travel bag”.
2. Select the individual task checkbox and complete it, leaving the bulk-toggle
   control alone.
3. Add “Buy train ticket” using the already established, unique input-field
   identity. This repeated-field step is deterministic on both routes.
4. Select the view showing only unfinished tasks.

Baseline calls Terra at the three semantic selection points. Hybrid asks Jev at
each point, accepts a known non-abstain choice only at probability >=0.8, and
actually calls Terra otherwise. No Terra planning call precedes these choices.
The router can therefore remove a real request rather than add Jev after Terra
has already selected an action.

Models receive the same actual observed names, roles (or literal HTML tags when
roles are absent), bounds, visibility, disabled state, link destinations, and page
text. Context is observed page text, not invented fieldset/group descriptions.
The current snapshot omits input type and checkbox labels: the individual and
bulk inputs are unnamed. No screenshots or private oracle metadata go to models.

Before action, a fresh sky-cua observation must retain the selected opaque ref
and name. A private oracle rejects and counts an incorrect selected target before
execution; any such selection fails the gate. For the unnamed checkbox, this
oracle compares observed bounds with the actual task-checkbox DOM rectangle,
without parsing the opaque ref or feeding the rectangle to the model.

This TodoMVC build keeps tasks in memory rather than local storage. The independent
proof reads both task labels and native checkbox `checked` values before filtering:
exactly the first task completed, second task unfinished. After filtering it
requires `#/active` and only the second task visible. No persistence is claimed.

### Frozen protocol and measured results

One full warm-up per route was excluded, then three paired workflows ran in
alternating route order. Browser/extension startup is excluded; timing starts
before opening the task tab and includes navigation, all observations, model
decisions, real fallbacks, actions, and independent final proof. No verification
suite ran concurrently with the measured workflows.

| Measurement, three measured workflows per route | Terra baseline | Jev then Terra |
| --- | ---: | ---: |
| Correct complete workflows | 3/3 | 3/3 |
| Wrong accepted target selections/actions | 0 | 0 |
| Actual Terra requests | 9 | 3 |
| Actual Jev requests | 0 | 9 |
| Total time | 70.924 s | 52.002 s |
| Median workflow | 23.668 s | 17.314 s |
| Total saving | — | **26.7%** |

Pair times (baseline / hybrid) were 22.722 / 17.511 s, 23.668 / 17.314 s,
and 24.534 / 17.178 s. Every hybrid pair won, by **22.9%, 26.8%, and 30.0%**.
Six of nine Terra calls were genuinely avoided (**66.7%**). Total model requests
increase from nine to twelve in the hybrid route because nine cheap Jev calls
replace six Terra calls and precede three remaining Terra calls.

The fixed gate passed: every task proof correct, zero wrong accepted targets,
and at least 20% total workflow savings. Reported Jev cost for nine measured calls
was $0.00115014; Terra billing was unavailable. Terra timing includes the existing
Codex CLI consumer and its context, not a matched direct-API comparison.

### The gate and fallback were necessary

Jev correctly accepted task entry and the Active filter in every measured run:
entry probability 1.0 and filter probabilities 0.98–0.99. **Its raw checkbox
proposals were wrong in all three runs**: the other unnamed input rather than
the individual checkbox specified by the intent. Their selected probabilities
were 0.59, 0.48, and 0.55, below 0.8, so none executed. Terra selected the correct
individual checkbox each time.

Thus Jev's raw target accuracy is only 6/9, while its accepted targets are 6/6
correct. This is evidence for the complete gated cascade on this task, not for
replacing Terra or assuming Jev understands arbitrary controls. Lowering to 0.7
would not add coverage on these saved responses.

### Setup findings and validation

The installed runtime passed the source-freshness gate. Initial disposable bridge
setup failed because the generated native-host Unix socket path exceeded Linux's
length limit; a short temporary socket directory fixed it. The inherited browser
selection was Chrome, so the isolated MCP process was explicitly set to Chromium
to match the launched browser. Its actual host executable/PID was verified.
The native-host manifest was written only inside the disposable profile; shared
browser configuration was untouched. These are harness setup fixes, not runtime
or model behavior changes.

The deterministic preflight completed the full workflow before any model call.
Offline checks cover projection, unnamed-checkbox geometry validation, exact DOM
task proof, malformed/extra tasks, and paired summaries. Root Ruff formatting/lint,
basedpyright, and **1,100 pytest tests passed**. All measured individual records,
frozen input/asset hashes, actual model-call counts, and outcomes were verified.
The runner exited 0 and removed its browser profile, native host/service processes,
and temporary judge credentials. No Rust changes, deployment, or commit was made.

Source and evidence:

- `scripts/jev_real_browser.py`: disposable actual browser bridge and workflow.
- `scripts/_jev_browser_workflow.py`, `scripts/test_jev_browser_workflow.py`:
  projection, independent proof, and 37 offline tests.
- `artifacts/jev-real-browser/vendor/`: unmodified downloaded assets.
- `artifacts/jev-real-browser/pilot/frozen.json`, `frozen.sha256`: protocol,
  model settings, asset hashes, and source hashes written before calls.
- Same directory: `results.json`, `summary.json`, `verification.json`, warm-up
  and individual pair evidence, requests/responses, native actions, and DOM proofs.
- `browser-proof.webp`: final real browser page showing the unfinished task only.

## Conclusion

**This browser workflow demonstrates real savings against Terra:** correct
completion, six avoided Terra calls, and 26.7% less measured end-to-end time.
The result is stronger evidence for browser ranking than the earlier Luna
fixture comparison or threshold-only timing projections.

The supported behavior is gated ranking with Terra fallback. The three rejected
wrong proposals are a material limit, not an incidental detail.

## Implications

The browser result supports continuing a narrowly scoped optional ranking route
at 0.8, preserving fresh-reference resolution, fallback, and independent outcome
checks. It does not establish a default change, calibrated probabilities, phone
use, outcome judging, or general autonomous-agent savings. It is one small app
and three measured pairs with fixed caller-known intents. It also does not show
an advantage over an app-specific deterministic TodoMVC script. No provider call
belongs in the deterministic Rust resolver on the basis of this experiment alone.
