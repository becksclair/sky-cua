# Complete release activation

The public installer activates one immutable release under
`~/.local/share/sky-cua/current`. Normal consumers discover that active release;
they do not require `SKY_CUA_RELEASE_ROOT`, `NODE_REPL_NODE_PATH`,
`NODE_REPL_NODE_MODULE_DIRS`, or Browser client path variables.

Activation installs stable `sky-cua-release` and, when the selected platform's
cua-node component is present, `node_repl` commands in `~/.local/bin` and the
standalone store's compatibility bin directory. Every link resolves through
`current`, so a generation rollover changes the target without rewriting the
consumer command. Consumers can request the complete verified runtime selection
with:

```sh
sky-cua-release resolve-active
```

The JSON result includes the active release and manifest identities, Node and
node_repl paths, direct Node module roots, Browser client path, and trusted
Browser client hashes. Resolution first verifies the activation receipt,
generation, native messaging manifests, compatibility links, and running
process identity. It does not mutate the installation.

Environment variables may remain optional explicit overrides for development
or diagnostics. They are not part of the normal installed discovery path.
