# Native Linux Browser plugin regression

## Context

On 2026-11-05 the selected `browser-use@openai-bundled` alpha2 skill called
`setupAtlasRuntime`, while its client exported `setupBrowserRuntime`. Correcting
that call exposed `Trusted Browser native-pipe access is unavailable` before
page navigation. The user reported that Browser had worked the previous day;
the exact last-known-good client and connection path were not recorded here.

## Investigation

Local package ownership identifies the active app, runtime, and native Browser
client as `chatgpt-desktop 26.1027.51404-1`. `pacman -Qkk chatgpt-desktop`
reported 4,150 files and zero alterations. Package metadata records installation
on 2026-11-03, before the reported working state. The cached AUR build recipe
was for an older version, so it was not used as proof of current contents.
The native app's installed package is the authoritative inspected distribution;
the retired `codex-desktop` rebuild repository is not its owner.

The active runtime exposed `nodeRepl.rpc` and no `nodeRepl.nativePipe`.
The package's native Browser client calls the registered `browser` service over
RPC. Sky's canonical Browser client requires a trusted native pipe and has a
different setup/API contract. The stale cached skill and client therefore
cannot be paired with this runtime. Changing an obsolete client trust hash
does not supply the missing transport.

Importing the installed native client, calling `setupBrowserRuntime()`, selecting
`agent.browsers.get("iab")`, and calling `tabs.list()` all succeeded in the
existing runtime. The tab list was empty in the engineering chat; no existing
tab was claimed, navigated, or changed. This proves setup, selection, and a
read operation, not navigation or screenshot acceptance.

The source contains two selection defects:

- `scripts/standalone_release.py::_sync_codex_bundled_marketplace` projects
  Sky's Browser wrapper into the native app's bundled Browser identity.
- Companion reconciliation and Chrome preflight still used the retired
  `browser-use@openai-bundled` identity, allowing its stale skill to remain
  selected alongside the current native Browser plugin.

Relevant projection and cache files have modification times on 2026-11-05.
The old alpha2 client dates to July, and the native package predates yesterday's
reported success. File times alone do not establish which change triggered the
regression. A selectively inspected November 4 log contains a failed alpha2
setup attempt and a later successful separate CUA path; it does not establish
a successful alpha2/native-RPC pairing.

## Conclusion

The immediate failure is a selected-client/runtime incompatibility. The native
client/service pair works, so the transport repair must preserve native Browser ownership and disable the
retired Browser identity. The subsequently requested Sky additions use the
[modern enhancement layer](2026-11-native-browser-enhancement-baseline.md), not a
second RPC implementation or a change to external `cua-node`.
The precise historical trigger remains unproven.

## Implications

Standalone installation and preflight now layer the bounded Sky helper on the
complete native Browser plugin when installed; Sky's external client and native-pipe trust boundary remain
intact. Regression fixtures cover native selection, replacing an existing Sky
projection, retired identity disablement, and the no-native fallback.

The active plugin projection and configuration were not changed during this
investigation. Approval for that rollout and a fresh-chat catalog/bootstrap
check remain separate from the successful direct native-client probe.
