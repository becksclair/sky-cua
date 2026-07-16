# Command and flag catalog

Load only when the selected lane needs a non-default command or exact flag.

| Lane | Command | Useful flags |
| --- | --- | --- |
| Local deploy | `python3 scripts/deploy_plugin.py` | `--no-build`, `--symlink`, `--kwin-effect`, `--no-companion`, `--force-companion`, `--refresh-accessibility`, `--local-install-host` |
| Release package | `python3 scripts/package.py` | `--no-build`, `--platform`, `--version-from-tag [TAG]`, `--release-dir` |
| Target install | `python3 install.py` | `--agents`, `--mode {auto,repo,bundle}`, `--bundle-root`, `--target-dir`, `--kwin-effect`, `--skip-system-deps`, `--dry-run` |
| MCP restart | `python3 scripts/install_mcp_server.py` | `--host`, `--restart-runtime`, `--refresh-accessibility`, `--kwin-effect` |
| Skill sync | `python3 scripts/sync_agent_skills.py` | no lane flags |
| Freshness | `python3 scripts/deploy_freshness.py` | `--client bin/sky-cua-client` |

Use `--no-build` only when the existing bundle is the requested input. The
package `--no-build` variant reuses the existing bundle; deploy `--no-build`
does not run the companion build lane.
