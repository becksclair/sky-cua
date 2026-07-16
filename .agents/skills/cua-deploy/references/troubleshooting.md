# Lane troubleshooting

Load only after a selected lane fails or produces an unexpected result.

- **Deploy/package failed:** stop dependent sync, install, or git steps; report
  the first failed command and its output before retrying.
- **Freshness check is stale:** run the local deploy again, then rerun
  `deploy_freshness.py`; do not run live tests against the stale binary.
- **Skill links point at the wrong checkout:** rerun
  `scripts/sync_agent_skills.py` from the intended main checkout and verify the
  three owned links.
- **Companion toolchain is absent:** accept the logged skip and existing staged
  APK, or load `android-phone-companion.md` for an explicit override or force.
- **AT-SPI is wedged:** load `rare-operations.md`; refresh only with the
  explicit flag and relaunch affected GTK applications.
- **Target package install attempts a build:** use the extracted bundle’s
  `python3 install.py --mode bundle`; the target needs neither Cargo nor a
  checkout.
- **Codex shows duplicate skills:** retain the global links for other agents;
  inspect the managed Codex config block rather than deleting links.
