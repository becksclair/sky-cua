# VM commands

Use `uv run python` when the project environment is needed. These templates
keep the SSH transport consistent; add only the profile-specific options
shown.

```bash
ssh_args=(
  --host 127.0.0.1 --port 22222 --user skycua
  --ssh-option StrictHostKeyChecking=no
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts
)
runner=(uv run python scripts/run_gui_testing_vm_smoke.py "${ssh_args[@]}")
```

## Select and verify a real session

```bash
ssh -p 22222 \
  -o StrictHostKeyChecking=no \
  -o UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  skycua@127.0.0.1 \
  'cd /workspace && sudo scripts/testing-vm/select-session.sh plasma'

ssh -p 22222 \
  -o StrictHostKeyChecking=no \
  -o UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  skycua@127.0.0.1 \
  'pgrep -a "kwin_wayland|gnome-shell|Hyprland|cosmic-session|cosmic-comp|i3|Xorg"; ls -l /run/user/1000/wayland-* 2>/dev/null || true'
```

Replace `plasma` with `gnome`, `cosmic`, `hyprland`, or `i3` as needed. Use
the matching `--desktop-env` and `--wayland-display` values from the router;
i3 derives its real X11 display inside the guest.

## Runner commands

```bash
"${runner[@]}" --profile all
"${runner[@]}" --profile curated
"${runner[@]}" --profile wayland-layer-shell-overlay \
  --desktop-env Hyprland --wayland-display wayland-1
"${runner[@]}" --profile wayland-pointer-scaled \
  --desktop-env COSMIC --wayland-display wayland-1
"${runner[@]}" --profile kde-kwin-effect-system-install \
  --vm-name testing-vm --libvirt-uri qemu:///session \
  --desktop-env KDE --wayland-display wayland-0
"${runner[@]}" --profile opencode-mcp --sync-opencode-settings
"${runner[@]}" --profile pi-mcp --sync-pi-settings
```

For `all`, append `--sync-opencode-settings` and/or
`--sync-pi-settings` only when those authenticated lanes are in scope. Add
`--sync-codex-settings` only when Codex settings are needed. These flags are
not implied by building or by checkout sync.

Do not add `--skip-host-build` or `--skip-sync` to a normal run. Use them only
when the exact build stamp and VM checkout identity were already verified.
The runner refreshes the guest portal stack when `--desktop-env` is supplied;
do not skip checkout sync when a session switch requires that refresh.

## Optional viewer

Only when a smoke task also needs a persistent viewer:

```bash
cp scripts/testing-vm/virt-viewer-testing-vm.service \
  ~/.config/systemd/user/virt-viewer-testing-vm.service
systemctl --user daemon-reload
systemctl --user enable --now virt-viewer-testing-vm.service
systemctl --user status virt-viewer-testing-vm.service
```

Viewer setup or status alone is outside this skill's trigger.
