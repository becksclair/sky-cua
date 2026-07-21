# Local deploy details

Load this reference only for the local deploy lane, its skill sync, or a
worktree/linking question.

`scripts/deploy_plugin.py` is the unreleased development compatibility lane. It
installs the built bundle as `sky-cua@local`, points
the computer-use compatibility plugin at it, and refreshes the installed MCP
runtime. A normal build-bearing deploy therefore does not need a separate MCP
restart. It does not create or activate an immutable complete release and must
not be used as the clean-machine or Codex package activation path. `--no-build`
installs the existing bundle and skips the companion build lane.

After a successful local deploy, run:

```bash
python3 scripts/sync_agent_skills.py
```

The sync owns only these links:

```text
~/.agents/skills/computer-use -> <checkout>/skills/computer-use
~/.agents/skills/browser-use  -> <checkout>/skills/browser-use
~/.agents/skills/phone-use    -> <checkout>/skills/phone-use
```

It replaces only sky-cua-owned links and preserves unrelated skills. It also
maintains the managed Codex `[[skills.config]]` entries that disable the
shared-link copies only inside Codex; the plugin-namespaced copies remain
enabled. Do not remove the global links to fix Codex duplication: other agents
use them.

The links point to the checkout from which sync ran. If that was a worktree,
they remain pointed at the worktree; rerun sync from the main checkout to
repoint them.

Build-bearing deploys can rebuild and stage the Android companion before the
bundle is built. Load `android-phone-companion.md` when the deploy prints a
`[companion]` status or companion options are requested.
