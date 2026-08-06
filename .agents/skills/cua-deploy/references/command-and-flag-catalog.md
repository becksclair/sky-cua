# Command catalog

Load only when selecting an exact deployment command.

| Lane | Command | Result |
| --- | --- | --- |
| Checkout build | `python3 install.py build` | Refresh durable outputs and create `dist/sky-cua-linux-x64-glibc.tar.gz` |
| Checkout install | `python3 install.py install` | Build or refresh durable outputs, then replace the fixed install tree and project integrations |
| Extracted archive install | `python3 install.py install` | Validate and install the extracted artifact into the fixed tree |
| MCP restart | `python3 scripts/install_mcp_server.py --host claude-code --restart-runtime` | Refresh the selected standalone MCP runtime without a distribution rebuild |
| Hermes MCP target | `python3 scripts/install_mcp_server.py --host hermes --restart-runtime` | Merge fixed-root `sky_cua` and `node_repl` into Hermes Agent and refresh runtime processes |
| KWin effect | `python3 scripts/install_mcp_server.py --host <host> --kwin-effect` | Refresh the selected MCP installation, then build, install, and reload the KDE agent-cursor effect |
| Freshness | `python3 scripts/deploy_freshness.py --client ~/.local/share/sky-cua/bin/sky-cua-client` | Compare the installed fixed-root client when a freshness check is specifically needed |

The standalone installer exposes no deployment flags or other subcommands.
Never add release IDs, manifest hashes, generation-store paths, activation
verification, rollback, staging, host-selector, or marketplace-selector options.

`python3 install.py build` owns the portable Linux x64 build profile. It uses an
isolated Cargo target directory and pins the runtime build to `x86-64-v3`, so
callers must not supply a machine-specific `RUSTFLAGS` override. Checkout-local
`python3 install.py install` remains a local build and may use the host's normal
Cargo configuration.

Validate the Hermes target with `hermes mcp test sky_cua`,
`hermes mcp test node_repl`, and `python3 scripts/live_hermes_mcp_smoke.py`.
