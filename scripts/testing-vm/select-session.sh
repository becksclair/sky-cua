#!/usr/bin/env bash
set -euo pipefail

session="${1:-}"
vm_user="${SKY_CUA_TESTING_VM_USER:-skycua}"

case "$session" in
  cosmic|kde|plasma|gnome|hyprland|i3)
    ;;
  *)
    printf 'usage: %s {cosmic|kde|plasma|gnome|hyprland|i3}\n' "$0" >&2
    exit 64
    ;;
esac

if [[ "$session" == "kde" ]]; then
  session=plasma
fi

cleanup_session_processes() {
  pkill -u "$vm_user" -x sky-cua-service 2>/dev/null || true
  pkill -u "$vm_user" -f '(^|/)sky-cua-overlay-host( |$)' 2>/dev/null || true
  pkill -u "$vm_user" -x sky-cua-overlay 2>/dev/null || true
  pkill -u "$vm_user" -x kwin_wayland 2>/dev/null || true
  pkill -u "$vm_user" -x kwin_wayland_wr 2>/dev/null || true
  pkill -u "$vm_user" -f 'kwin_wayland_wrapper' 2>/dev/null || true
  pkill -u "$vm_user" -x Hyprland 2>/dev/null || true
  pkill -u "$vm_user" -x cosmic-comp 2>/dev/null || true
  pkill -u "$vm_user" -x cosmic-session 2>/dev/null || true
  pkill -u "$vm_user" -x cosmic-randr 2>/dev/null || true
  pkill -u "$vm_user" -x gnome-shell 2>/dev/null || true
  pkill -u "$vm_user" -x gnome-session-b 2>/dev/null || true
  pkill -u "$vm_user" -f 'gnome-session-binary' 2>/dev/null || true
  pkill -u "$vm_user" -x Xorg 2>/dev/null || true
  pkill -u "$vm_user" -x i3 2>/dev/null || true
  pkill -u "$vm_user" -x xdg-desktop-por 2>/dev/null || true
  pkill -u "$vm_user" -f '(^|/)xdg-desktop-portal( |$)' 2>/dev/null || true
  pkill -u "$vm_user" -f '(^|/)xdg-desktop-portal-[^/ ]+( |$)' 2>/dev/null || true
  pkill -u "$vm_user" -x xdg-document-po 2>/dev/null || true
  pkill -u "$vm_user" -f '(^|/)xdg-document-portal( |$)' 2>/dev/null || true
  pkill -u "$vm_user" -x xdg-permission- 2>/dev/null || true
  pkill -u "$vm_user" -f '(^|/)xdg-permission-store( |$)' 2>/dev/null || true
}

if [[ "$session" == "gnome" ]]; then
  sudo tee /etc/gdm/custom.conf >/dev/null <<EOF
[daemon]
WaylandEnable=true
AutomaticLoginEnable=True
AutomaticLogin=${vm_user}
DefaultSession=gnome.desktop

[security]

[xdmcp]

[chooser]

[debug]
Enable=false
EOF
  cleanup_session_processes
  sudo systemctl disable --now greetd.service >/dev/null 2>&1 || true
  sudo systemctl reset-failed gdm.service >/dev/null 2>&1 || true
  sudo systemctl enable gdm.service >/dev/null 2>&1 || true
  sudo systemctl restart gdm.service
else
  sudo tee /etc/greetd/config.toml >/dev/null <<EOF
[terminal]
vt = 1

[default_session]
user = "${vm_user}"
command = "/usr/local/bin/sky-cua-testing-vm-session ${session}"
EOF
  cleanup_session_processes
  sudo systemctl disable --now gdm.service >/dev/null 2>&1 || true
  sudo systemctl reset-failed greetd.service >/dev/null 2>&1 || true
  sudo systemctl enable greetd.service >/dev/null 2>&1 || true
  sudo systemctl restart greetd.service
fi

printf 'selected testing VM session: %s\n' "$session"
