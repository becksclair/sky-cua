# VM smoke troubleshooting

Use these only for the matching failure. Preserve the original error and
artifact path before changing session, transport, or freshness options.

| Symptom | Smallest next action | Do not conclude |
| --- | --- | --- |
| `Could not resolve hostname testing-vm` | Rerun the same runner command with the `127.0.0.1:22222` transport in `commands.md` | That the profile or product failed |
| Wrong Wayland socket or stale compositor | Select the target guest session, then verify compositor processes and `/run/user/1000/wayland-*` | That nested Docker/Xvfb is an acceptance substitute |
| Portal behavior is wrong after a session switch | Rerun with normal checkout sync and the requested `--desktop-env`/display so the runner refreshes the portal stack | That a stale portal proves a runtime regression |
| KWin system-install lacks `host-framebuffer-ready.json` or exits early | Confirm Plasma/KDE `wayland-0`, then inspect the host framebuffer proof and `host-summary.json` | That remote stdout alone is pixel evidence |
| KWin cleanup appears dirty | Check current `sky-cua-overlay-host`, `service.sock`, and `agent-cursor.sock` targets; report any residue | Historical `sky-cua-overlay` names as current residue |
| ydotool readiness is unclear | Check that the path is a Unix socket or prove it with a real `ydotool key ...` call; it may be a datagram socket | Unix stream-connect failure as the product diagnosis |
| OpenCode/Pi/provider setup fails before tool evidence | Preserve the exact provider/auth/model error and classify it as an external blocker; retry only the requested lane after fixing inputs | A runtime or overlay regression without product evidence |

After the smallest targeted correction, return to `SKILL.md`'s Stop and report
rule. Do not fall back to the retired nested harness or silently change
`--skip-*` freshness semantics.
