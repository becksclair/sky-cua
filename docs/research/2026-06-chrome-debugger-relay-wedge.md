# Browser CDP wedge: RESOLVED — the extension's 30s driver-liveness heartbeat

Investigated 2026-06-28, **root cause found and fixed 2026-07-01.**

Current architecture note (2026-07-19): the persistent primary keepalive below
remains the `legacy`-mode fix. In `hybrid`/`strict`, the unified browser control
plane folds heartbeat and operations into one canonical persistent actor; see
[`unified-browser-bridge-control-plane.md`](../features/unified-browser-bridge-control-plane.md).

## RESOLVED 2026-07-01 — read this first; the rest of the doc is superseded history

The wedge is **not** MV3 idle-death, **not** an unfixable proprietary relay bug,
and **not** our Rust. It is the Codex extension's own **driver-liveness
heartbeat**, and it is fixable entirely on our side.

Root cause (read straight from `resources/chrome-extension/codex/1.1.5_0/background.js`,
the extension's `client-heartbeat-alarm`): every **30 seconds** the extension
sends a `ping` request to its **primary** native-host client and waits **3s** for
a reply. If none arrives it runs `chrome.debugger.detach` on **every tab** and
stops all sessions — a cleanup so a crashed driver does not leave the browser
debugged forever. At the time, sky-cua drove the browser through per-operation,
*ephemeral*
connections (`session_id: "sky-cua-mcp"`) that the host does **not** route the
heartbeat to, so nothing sky-cua does answers the ping. Between (and even within)
operations the extension detaches the debugger; the next command then reports
"Detached while handling command." That is the wedge.

Everything the 2026-06-28 investigation observed is consistent with this and was
misread as a relay fault: raw CDP and direct `chrome.debugger` worked because the
detach only removes *our* attachment; the "relay wedge" was the detached session,
not a broken relay. The ~50% intermittency is whether a 30s tick lands in an idle
gap; "settles into a persistent streak" is the agent's think-time growing past
30s; **raising timeouts made it worse** because it lengthened idle windows;
desktop control was never affected because it does not use the debugger.

Live proof (against a real Brave + the store `Codex 1.1.5` extension + OpenAI's
`extension-host`): connecting one client to the host socket with a non-`sky-cua-mcp`
session id (so the host routes the heartbeat to it) received `ping` requests at
1.6s, 31.6s, 61.6s, 91.6s — exactly every 30.0s — and answering each `pong` kept
the debugger attached.

**Legacy fix (shipped): a persistent primary keepalive in the daemon.**
`crates/sky-cua-service/src/browser/keepalive.rs` holds one long-lived connection
to the bridge socket, registered as the primary/driving client, whose only job is
to answer the 30s heartbeat. Started lazily on first browser-bridge use
(`BrowserBridgeExecutor::from_env`), reconnects across host restarts. The
legacy-only tradeoff was that this keepalive and a concurrently running Codex
desktop client could replace one another as primary.

Live-validated 2026-07-01 on real Brave: with the keepalive armed, a
debugger-attached tab survived a 45s idle and a 70s idle (two heartbeat ticks)
— `browser_navigate` returned clean, no "Detached". Control: on a daemon where
the keepalive was not armed, the same op after 45s idle returned "Debugger is
not attached." Nuance: the keepalive is a daemon-process task that arms on the
first browser op in that process, so a daemon restart mid-session leaves a
window until the next op re-arms it (benign in practice — a fresh daemon has no
attached sessions to lose, and the next op both re-arms the keepalive and
re-attaches via recovery). In steady state the daemon is a stable singleton, so
the keepalive persists once armed.

Also shipped 2026-07-01, independent robustness hardening (an adversarial audit of
`crates/sky-cua-service/src/browser/*` and `sky-cua-chrome-host` found real
defects that turned a *recoverable* hiccup into corruption or a terminal hang):
double-input replay on a mid-sequence detach (now gated on `!replay_safe()`);
native host froze under a non-reading SW because it wrote frames under the global
lock (writes now happen after unlock); stale own-reply on a reused stream skipped
not fatal; service frame cap 4 → 64 MiB; probe/IO timeout honors
`SKY_CUA_BROWSER_REQUEST_TIMEOUT_MS`.

Everything below is the original 2026-06-28 investigation, preserved for history.
Its "root layer is the proprietary relay" verdict is **wrong** — the relay was
fine; the debugger had been detached out from under it by the heartbeat.

---

# (historical) Browser CDP wedge: the extension's chrome.debugger relay

Investigated 2026-06-28. Status (historical, since corrected above): **root layer
isolated with certainty; exact trigger and a validated fix still open.**

## Symptom

In the `codex-cua` VM smoke (and any browser-bridge path), after the agent
installs the Codex extension and opens/claims a tab, `browser_open` /
`browser_navigate` / `browser_claim_tab` time out at CDP `Page.enable` /
`Page.navigate`, often with "Detached while handling command". Desktop control
works perfectly throughout. It presents as intermittent (~50%) and can settle
into a long persistent-failure streak. The agent honestly reports `blocked`;
`browser_marker_matches` and `vision_token_matches` come back false while every
desktop ground-truth check passes.

## Verdict (the one thing to remember)

The wedge lives **entirely in the Codex extension's `chrome.debugger` relay** —
the proprietary, OpenAI-bundled extension. It is **not** Chrome, **not** the
`chrome.debugger` API itself, **not** our Rust (`sky-cua-service` /
`sky-cua-chrome-host`), and **not** the VM/host environment. The relay path is:

```
sky-cua-service (bridge) --unix socket--> sky-cua-chrome-host (native host)
  --native messaging--> extension service worker --> chrome.debugger.sendCommand
```

Everything up to and including `chrome.debugger` is healthy. The relay through
the extension's service worker is where commands disappear.

## The decisive isolation (do this first next time)

Two probes, both using **raw CDP over the DevTools websocket**, which we proved
works on the exact Chrome instance that wedges. They live nowhere permanent — the
scripts below are the artifact. The Codex extension id is
`hehggadaopoacecdllhhajmbjkdcmajg`.

### Probe 1 — raw Chrome CDP (no extension, no native host)

Launch Chrome with a raw remote-debugging port and drive `Page.enable` directly.
Requires `--ozone-platform=wayland` (without it Chrome dies with "Missing X
server or $DISPLAY") and `python-websocket-client` on the VM.

```bash
google-chrome --user-data-dir=/tmp/cdp-prof --remote-debugging-port=9222 \
  --remote-allow-origins='*' --no-first-run --no-default-browser-check \
  --ozone-platform=wayland about:blank &
```

```python
import websocket, json, time, urllib.request
ts = json.load(urllib.request.urlopen("http://127.0.0.1:9222/json", timeout=5))
url = next(t["webSocketDebuggerUrl"] for t in ts if t["type"] == "page")
ws = websocket.create_connection(url, timeout=12, suppress_origin=True)
def cmd(method, params=None):
    ws.send(json.dumps({"id": 1, "method": method, "params": params or {}}))
    t0 = time.time()
    while time.time() - t0 < 11:
        m = json.loads(ws.recv())
        if m.get("id") == 1:
            return (time.time() - t0) * 1000, m
    return None, "wedged"
print(cmd("Page.enable"))  # observed: 0 ms OK
```

**Result: `Page.enable` returns in 0 ms.** Chrome and the renderer are perfect.

### Probe 2 — chrome.debugger inside the extension's service worker

Attach raw CDP to the extension's `service_worker` target and `Runtime.evaluate`
`chrome.debugger` *in the SW context*, bypassing the native host and our bridge.
The SW target only exists after the extension is installed, so poll the running
smoke's profile `DevToolsActivePort` (line 1) for the port, then:

```python
import websocket, json, urllib.request, sys
EXT = "hehggadaopoacecdllhhajmbjkdcmajg"; port = sys.argv[1]
ts = json.load(urllib.request.urlopen(f"http://127.0.0.1:{port}/json", timeout=5))
sw = next(t for t in ts if t["type"] == "service_worker" and EXT in t["url"])
ws = websocket.create_connection(sw["webSocketDebuggerUrl"], timeout=20, suppress_origin=True)
ws.send(json.dumps({"id": 1, "method": "Runtime.enable"})); ws.recv()
expr = """new Promise(res => chrome.tabs.create({url:'about:blank'}, t =>
  chrome.debugger.attach({tabId:t.id}, '1.3', () =>
    chrome.debugger.sendCommand({tabId:t.id}, 'Page.enable', {}, r =>
      res({attachErr: chrome.runtime.lastError, pageEnable: r})))))"""
ws.send(json.dumps({"id": 2, "method": "Runtime.evaluate",
    "params": {"expression": expr, "awaitPromise": True, "returnByValue": True}}))
while True:
    m = json.loads(ws.recv())
    if m.get("id") == 2: print(m["result"]["result"]["value"]); break
```

**Result: attach 2 ms, `Page.enable` 12 ms, no error** — on the same Chrome where
the bridge wedges. The extension's `chrome.debugger` API works when poked
directly; only the relay through the SW's message handler fails.

## Conclusively ruled out (with evidence — do not re-investigate)

| Suspect | Verdict | Evidence |
| --- | --- | --- |
| Chrome / renderer | healthy | raw CDP `Page.enable` = 0 ms (Probe 1) |
| `chrome.debugger` API | healthy | direct SW call = 12 ms (Probe 2) |
| Our Rust (bridge / native host) | not the cause | reverted to the known-good revision, failed identically |
| Chrome version | unchanged | 149.0.7827.114, binary from 2026-06-10, no mid-session update |
| VM CPU/memory/disk/inodes | healthy | load ~0, 5.7Gi free, 44% disk |
| Leftover Chrome/codex processes | none | only `sky-cua-input-helper` persists between runs |
| Chrome managed policy (DevTools disable) | none | `/etc/opt/chrome/policies/` empty |
| Wedged long-lived daemon | none | no stale `sky-cua-service`/chrome-host between runs |
| Stuck in-memory/session state | no | survived `systemctl reboot` |
| Deeper stuck state | no | survived a full host restart |
| Desktop / AT-SPI / input path | healthy | every desktop ground-truth check passes on every run |

## False trails (each one cost time — skip them)

- **Timeout tuning makes it worse, not better.** Raising
  `SKY_CUA_BROWSER_REQUEST_TIMEOUT_MS` (e.g. to 45s) *increased* the failure: a
  larger per-command cap just lets the doomed first `Page.enable` wait longer
  before failing. The per-command budget is already fine; the command never
  completes because the relay target is gone.
- **Chrome's verbose log does not contain the detach reason.** We made Chrome
  logging permanent (`--enable-logging --v=1 --vmodule=*debugger*=3,*devtools*=2,
  *service_worker*=1`, surfaced as `chrome-debug.log`). It captures startup,
  extension load, and the SW context creation, but **not** the `chrome.debugger`
  detach reason — Chrome hands that to the extension's `onDetach(source, reason)`
  callback and never `VLOG`s it. Useful artifact; dead end for this reason.
- **The native-host stderr does not carry the trace either.** We also capture
  Chrome's stderr (`chrome-stderr.log`) on the theory that the spawned
  `sky-cua-chrome-host` (which traces under `SKY_CUA_CHROME_HOST_TRACE=1`) would
  inherit it. It does not in practice — only GPU/`on_device_model` noise lands
  there.
- **A reboot / full restart does nothing.** Confirmed across runs 16–17. Do not
  reach for "reboot the VM" — the VM is healthy.
- **`--load-extension` is dead on Chrome 149.** You cannot CLI-load the Codex
  extension (or a throwaway test extension) to reproduce; the smoke's agent-driven
  "Load unpacked" is the only install path.
- **codex-in-the-VM can't do computer-use standalone.** The OpenAI-bundled
  `computer-use` plugin is stubbed on this Linux dev box; the working
  `sky-cua-client` binary only exists inside the smoke's per-run codex-home
  staging, so a hand-launched `codex` reports
  `MCP client for computer-use failed to start`.

## Attempted fixes that did NOT validate

Both reverted (kept in reflog). Do not re-apply blindly.

- **`fix(browser): fail-fast the attach+enable probe`** (`dd3b256`) — a clean
  fix for a real recovery-budget-starvation bug, but it did not change the flake
  (the wedge is persistent, not a recoverable race).
- **`fix(chrome-host): keep the extension service worker alive`** (`b95243f`) —
  a native-host keep-alive thread sending a periodic `keepAlive` notification
  over the native-messaging port. Built and deployed correctly (`keepAlive`
  present in the VM binary), but run23 wedged anyway. A passive native message is
  not what unblocked run20 (see below).

## What we know but couldn't fully explain

The single anomaly that *did* unblock the browser phase, after 9 straight
failures, was running **Probe 2 immediately before the browser phase** (run20):
the probe's tab id `1467551804` and the bridge's next `browser_open` tab
`1467551805` are adjacent, and the browser phase then succeeded end-to-end. Probe
2 does two things a passive keep-alive message does not: it holds a raw-CDP
**debugger attached to the SW target**, and it runs live **`chrome.debugger`
activity**. A later attempt to hold the SW alive with a passive DevTools
`Runtime.evaluate` keep-alive failed (its websocket died — the SW terminated
anyway), which is why the leading hypotheses are:

1. **MV3 service-worker idle-death.** The SW idle-terminates (~30s) during the
   gap between Phase A (install) and Phase B (browser use); a dead SW strands the
   relay. Fits the flake→persistent transition (agent Phase-A timing grew past
   the idle window) and the "Detached" symptom. But the native-host keep-alive
   *message* did not fix it — possibly because the host (and thus the keep-alive)
   is only spawned when the SW connects via `connectNative`, which may be too
   late to prevent the first idle-death.
2. **`chrome.debugger` warm-up / session state in the relay.** The bridge's first
   relayed attach fails cold, and a prior direct attach (Probe 2) "primes" it.

These two are not yet distinguished. The clean disambiguator is: keep the SW
genuinely alive for the whole run (a debugger attached to the SW *target* keeps
it from being terminated) and see whether the bridge still wedges — if it wedges
with a provably-alive SW, idle-death is out and it's relay/handler state.

## Reproduction

```bash
python3 scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 \
  --user skycua --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  --profile codex-cua --desktop-env KDE --skip-host-build
```

Browser-phase failure is frequent and clusters into persistent streaks. Each run
now leaves `chrome-debug.log` and `chrome-stderr.log` in the judge artifact dir.
Check `coverage-summary.json` `ground_truth.browser_marker_matches` /
`vision_token_matches` for the browser-phase outcome.

## Where to take it next

1. **File it against the Codex extension** with the isolation evidence (raw CDP
   works, direct `chrome.debugger` works, only the relay wedges). The fix lives
   in proprietary code; this is the strongest ROI.
2. **Watch the SW console during a wedge** via the raw-CDP-into-SW path (Probe 2's
   transport) — subscribe to `Runtime.consoleAPICalled` / `Log.entryAdded` and
   `chrome.debugger.onDetach` to read the extension's `executeCdp` handler and the
   detach reason. Pins the exact trigger.
3. **A/B the keep-alive idea properly** before abandoning it: hold a debugger
   attached to the SW *target* for the whole run (keeps the SW alive by Chrome's
   debugged-worker rule, not by a message) across several runs vs. control.
