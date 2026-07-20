# Model-facing CUA documentation and persistent Phone JS

Status: implementation design for the post-cutover complete CUA release.

## Outcome

Sky-cua ships one immutable `documentation` component and one persistent Phone JavaScript export with the complete Linux x86-64 glibc generation. Codex Desktop, OpenClaw, and OpenCode consume the same installed bytes and route ordinary work between direct MCP and persistent `node_repl` without checkout paths. Compatibility projections contain routing references, never a copied implementation or second documentation tree.

## Package and runtime design

Phone JavaScript is the `@heliasar/sky-cua/phone` package subpath. The existing root `@heliasar/sky-cua` export remains the Computer Use facade and gains no Phone methods. The subpath reuses the package's socket resolution, NDJSON framing, request metadata, cancellation, structured errors, and deterministic packaging.

The public entrypoint exports `createPhoneClient(options?)`, a lazy `phone` singleton, public request/result types, `PhoneDeviceSession`, `PhoneScreenshot`, and structured Phone errors. Import and construction never start, restart, stop, or configure the daemon or either MCP server. `phone.close()` closes only the caller's socket state. A connected `PhoneDeviceSession` binds the exact returned `session_id` and serial to every device operation, never reconnects implicitly, and becomes locally invalid after explicit disconnect.

The public workflow is:

1. `phone.status()` and `phone.listDevices()` discover host and device truth without a selector.
2. `phone.connect()` returns a bound session and its current capability profile.
3. `session.refreshCapabilities()`, `observe()`, `screenshot()`, pointer, keyboard, accessibility, notifications, apps, companion, and settings operations preserve the daemon's existing structured fields.
4. `PhoneScreenshot.bytes()`, `dataUrl()`, and `emit()` support inline images and owner-local screenshot paths. Path images are read with `node:fs/promises`; `emit()` calls `nodeRepl.emitImage` and response metadata identifies `phoneUse`.
5. `session.disconnect()` invalidates the handle and preserves `keep_wireless`; `phone.close()` never disconnects the device.

The JS surface maps every existing `PhoneRequest` and `PhoneResponse` variant. It preserves backend truth: `backend=none` is an unsuccessful operation; app actions require `success=true`; failed pairing/disconnect remain structured failures; diagnostics accompanying a real fallback backend remain warnings. Reads may reconnect before any write. A disconnect before a request write is `not_dispatched`; a disconnect after a write is `ambiguous`, and no mutation is retried.

## Metadata and provenance

The service envelope becomes backward-compatible `Phone { request, context? }`. The optional `PhoneRequestContext` carries normalized `session_id`, `turn_id`, explicit `caller_provenance` (`codex_desktop`, `openclaw`, `opencode`, or `direct_mcp`), `identity_synthetic`, and `client_info`. The node_repl facade reads current `nodeRepl.requestMeta` on every operation rather than caching it. Direct sky_cua MCP projects equivalent call context. The daemon records this context with Phone operations and sessions; old callers that omit it remain readable.

Supplied Codex metadata remains exact. Generic MCP callers use node_repl's existing stable synthetic process session and per-call turn identities. The Phone facade neither invents another identity nor obscures whether identity was synthetic.

## Documentation information architecture

The immutable component has this canonical shape:

```text
components/documentation/
  skills/{browser-use,computer-use,phone-use}/
  references/{node-repl,browser,computer,phone,toolbox}/
  recipes/{browser,computer,phone,composed}/
  examples/{node-repl,files,images,pdf,ocr,playwright,phone}/
  inventories/{api,capability,example,routing}.json
  README.md
```

The three top-level skills stay compact. Each first decides direct MCP versus persistent node_repl, states the stopping rule, and links to task-oriented installed recipes. Direct single actions remain MCP. Persistent state, reusable setup, JavaScript composition, image/PDF/OCR/file work, Browser API use, and Phone JS use route to node_repl. Browser recipes distinguish `tab.playwright` from standalone Playwright using a system Chrome-family executable.

Shared node_repl references cover `js`, `js_reset`, `js_add_node_module_dir`, persistent bindings and top-level await, `nodeRepl.write`, `emitImage`, response metadata, local files, Node globals/imports, Buffer/ArrayBuffer/Blob/streams/paths/file URLs/data URLs, Sharp WebP/PNG/JPEG, Canvas/Skia, pixelmatch, PDF.js fonts/cmaps, Tesseract language data, standalone Playwright, and composed Browser + Computer + Phone + OCR/PDF/image/file workflows. Examples are copy-safe, installed-path-only, executable under bundled Node 24, and include content assertions and cleanup.

The API inventory is generated from Browser declarations/fixtures, Computer protocol/types, Phone request/response types, and node_repl tool schemas. The capability inventory is generated from `RELEASE.json`, runtime locks, package manifests, and explicit unsupported capabilities. The example inventory binds every runnable example path, hash, size, runtime, expected assertions, and required capabilities. The routing inventory binds every skill/reference/recipe edge and direct-versus-REPL decision.

## Release and installation

The existing schema reservation is authoritative: `documentation.component` plus hashed API, capability, example, and routing inventory pointers. The complete-release builder generates the component deterministically, declares dependencies on `browser-js` and `cua-node-linux-x64-glibc`, adds `model-facing-documentation`, and binds the component and inventory hashes into `RELEASE.json`, provenance, licenses, and SBOM. Each inventory binds its individual files so example tampering fails verification even when the aggregate component is inspected independently.

Full installations include documentation; core-only does not. Installers project compact skills from the exact generation and set `SKY_CUA_DOCUMENTATION_ROOT` to that generation's component. Projection destinations never point at a checkout or an unverified `current`-relative subtree. Codex `PROJECTION.json` points to the canonical routing inventory/hash and carries no Markdown, examples, Phone code, or Browser implementation beyond its already-required exact Browser compatibility bytes.

## Verification and acceptance

Implementation acceptance is one matrix, not package-presence proof:

- TypeScript generation and parity cover every Phone request/response variant, context field, declaration, and public method.
- Phone unit/integration tests cover lazy lifecycle, exact bound session identity, capability refresh, screenshot inline/path/emit, local files, metadata/provenance, structured failures, disconnect invalidation, before-write failure, and ambiguous after-write failure without retry.
- Release tests cover deterministic docs generation, link/routing progressive disclosure, stale versions and checkout paths, inventory coverage, component/profile dependencies, projection pointers, SBOM/provenance/licenses, and tamper of a docs/API/example byte.
- Every example executes under bundled Node 24 from an installed generation and verifies its emitted or written output.
- Ordinary models in Codex Desktop, OpenClaw, and OpenCode start from each main skill, discover the intended recipe, choose direct versus REPL correctly, and execute it.
- Full Phone acceptance covers discovery, connect, capability profile, observe/screenshot/emit, pointer, keyboard, app/notification/accessibility operations where supported, local files, disconnect, service loss, and direct-MCP/Phone-JS semantic parity.
- Composed acceptance uses Browser, Computer, Phone, OCR/PDF/image/file work in one persistent session while preserving separate caller provenance and Browser tab groups.

## Performance budget

Phone JS adds no daemon, MCP, proxy, authentication, or authorization hop. Measure import/bootstrap cold cost separately from warm operations; reuse one persistent node_repl and direct service socket behavior. Warm Phone facade overhead must remain within the complete-stack median/p95 budgets already used for node_repl. Compact skill token size and generated reference bytes are recorded; routing skills stay small enough that ordinary selection does not load toolbox references until needed.

## Completion standard

This phase is complete only when a new immutable release contains verified Phone JS and documentation bytes, all installed examples pass, all three hosts discover and use the canonical routes, Phone direct/REPL parity passes, and final serial host reloads prove the active generation. Dependency READMEs, source-only tests, or compatibility copies are insufficient.
