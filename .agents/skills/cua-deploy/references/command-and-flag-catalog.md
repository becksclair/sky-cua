# Command and flag catalog

Load only when the selected lane needs a non-default command or exact flag.

| Lane | Command | Useful flags |
| --- | --- | --- |
| Local deploy | `python3 scripts/deploy_plugin.py` | `--no-build`, `--symlink`, `--kwin-effect`, `--no-companion`, `--force-companion`, `--refresh-accessibility`, `--local-install-host` |
| Complete release | `python3 scripts/build_complete_release.py` | `--output-root`, `--core-source`, `--cua-node-component`, `--producer-commit`, `--no-fat-archive` |
| Target activation | `python3 install.py install --manifest-sha256 <sha256>` | `--store-root`, `--profile`, `--native-messaging-home`, `--bin-dir`, optional `--host` integration flags |
| Idempotent repair | `python3 install.py ensure --manifest-sha256 <sha256>` | same activation roots/profile/integration flags |
| Activation proof | `python3 install.py verify-activation --manifest-sha256 <sha256>` | `--store-root`, `--profile`, `--native-messaging-home`, `--bin-dir` |
| MCP restart | `python3 scripts/install_mcp_server.py` | `--host`, `--restart-runtime`, `--refresh-accessibility`, `--kwin-effect` |
| Skill sync | `python3 scripts/sync_agent_skills.py` | no lane flags |
| Freshness | `python3 scripts/deploy_freshness.py` | `--client bin/sky-cua-client` |

Use deploy `--no-build` only when the existing development bundle is the
requested input; it does not run the companion build lane. Raw
`scripts/release_generation.py install` is internal-only. The legacy
`scripts/package.py` installer is not a complete release activation command.
