# sky-cua `node_repl` runtime

This package is the sky-cua-owned source for the Linux x86-64 glibc
`node_repl` MCP server. Bun builds the first-party TypeScript host and Node
24.14.0 runs the built host plus its private persistent VM kernel child.

The public MCP surface is exactly `js`, `js_reset`, and
`js_add_node_module_dir`. Supplied MCP `_meta` objects are cloned without
augmentation. When a caller omits `_meta`, the MCP process creates one stable
session identity, a fresh turn identity for each `tools/call`, and records
`caller_provenance`, `initialize.clientInfo`, and `identity_synthetic: true`.
`SKY_CUA_MCP_CALLER_PROVENANCE` accepts only `codex_desktop`, `openclaw`,
`opencode`, or `direct_mcp`.

The package does not start or restart the sky-cua daemon. The bundled
`@heliasar/sky-cua` facade connects directly to the already-running service.
The canonical sibling `@heliasar/browser-use` package is an integration input,
not copied source; package-local Browser trust tests consume its exact built
bytes through `CUA_NODE_BROWSER_CLIENT_PATH` or the repository default.

An installed `bin/node_repl` is self-contained: the native launcher executes
its sibling `bin/node`, and the host discovers the adjacent runtime manifest,
module root, and trusted Browser hashes. Release, Node, module-root, and Browser
client environment variables are optional diagnostic or development overrides,
not normal startup requirements.

Run the focused source gates from this directory:

    bun install --frozen-lockfile
    bun run verify

Runtime verification and live acceptance require an assembled immutable
runtime generation and explicit paths:

    bun run verify:runtime -- --root=/absolute/path/to/cua_node \
      --enforce-lock=runtime-lock.json \
      --enforce-lock=native-assets.lock.json
    bun run acceptance:repl -- --runtime-root=/absolute/path/to/cua_node --json
    bun run acceptance:full -- --runtime-root=/absolute/path/to/cua_node \
      --target=linux-x64 --network=disabled --empty-user-cache --json

The original migration input was Codex Desktop commit
`65c69a3f1afc9f81274189901bc72e80682ea03a` plus its preserved dirty
`runtime/cua-node/**` files on 2026-07-20. The pre-copy inventory covered 151
tracked, modified, or untracked runtime/provenance files and had SHA-256 list
digest `f00172d7dbc9a58683c92039998b2cee5107286f882179386f9513f8de18c19c`.
The recovered upstream Linux `resources/node_repl` evidence binary was
10,416,696 bytes with SHA-256
`ac14a0d1483a2b33622c96bb12ecdab9fd8b31fb73a63ec6d2ba51dfeebfa27a`;
it is evidence only and is not a runtime dependency.
