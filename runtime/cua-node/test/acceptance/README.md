# Linux `cua_node` acceptance harness

This lane owns deterministic acceptance evidence only. It does not import runtime production modules, use sandbox metadata, open a network connection, modify the launcher, or claim installed acceptance.

Generate or verify the checked-in local fixtures:

```sh
bun runtime/cua-node/test/acceptance/orchestrator.ts --generate
bun runtime/cua-node/test/acceptance/orchestrator.ts --check
```

Run one command per acceptance gate:

```sh
bun runtime/cua-node/test/acceptance/orchestrator.ts --category=g2
bun runtime/cua-node/test/acceptance/orchestrator.ts --category=g3
bun runtime/cua-node/test/acceptance/orchestrator.ts --category=g4
```

The narrower task selectors are `g2-browser`, `g2-computer`, `g2-media`, `g3-package`, and `g4-installed`. Every run prints an output root and writes `summary.json`, per-task result artifacts, and exact evidence file paths below `out/cua-node-acceptance/` unless `--output=PATH` is supplied.

The default adapters are deterministic fakes. Integrators can point a task at a later real runtime/service command without changing this harness:

```sh
CUA_NODE_ACCEPTANCE_BROWSER_COMMAND='...' bun runtime/cua-node/test/acceptance/orchestrator.ts --category=g2-browser
CUA_NODE_ACCEPTANCE_SKY_COMMAND='...' bun runtime/cua-node/test/acceptance/orchestrator.ts --category=g2-computer
CUA_NODE_ACCEPTANCE_MEDIA_COMMAND='...' bun runtime/cua-node/test/acceptance/orchestrator.ts --category=g2-media
CUA_NODE_ACCEPTANCE_PACKAGE_COMMAND='...' bun runtime/cua-node/test/acceptance/orchestrator.ts --category=g3-package
CUA_NODE_ACCEPTANCE_INSTALLED_COMMAND='...' bun runtime/cua-node/test/acceptance/orchestrator.ts --category=g4-installed
```

`g4-installed` always remains `pending` until the integrator runs a real installed-task command and reviews its redacted evidence. The harness is not the final installed acceptance decision.
