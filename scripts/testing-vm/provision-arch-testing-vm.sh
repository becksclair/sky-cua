#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -ne 0 ]]; then
	printf 'run this provisioner as root inside the Arch testing VM\n' >&2
	exit 77
fi

vm_user="${SKY_CUA_TESTING_VM_USER:-skycua}"
codex_desktop_package="${CODEX_DESKTOP_PACKAGE:-}"
autologin_session="${SKY_CUA_TESTING_VM_SESSION:-cosmic}"
# Keep this pinned to the host-proven OpenCode version unless deliberately
# refreshing the non-Codex harness test surface.
opencode_npm_spec="${OPENCODE_NPM_SPEC:-opencode-ai@1.14.51}"

pacman-key --init
pacman-key --populate archlinux
pacman -Syu --noconfirm

pacman -S --noconfirm --needed \
	alsa-lib \
	alacritty \
	at-spi2-core \
	base-devel \
	bash \
	cairo \
	clang \
	cmake \
	cosmic-session \
	cosmic-terminal \
	dbus \
	evolution-data-server \
	extra-cmake-modules \
	foot \
	gcc \
	git \
	glib2 \
	ghostty \
	gdm \
	gnome-console \
	gnome-shell \
	gnome-terminal \
	greetd \
	grep \
	grim \
	gst-plugin-pipewire \
	gst-plugins-good \
	gtk3 \
	hyprland \
	i3-wm \
	imagemagick \
	jq \
	kconfig \
	kcoreaddons \
	kitty \
	konsole \
	kwin \
	kwindowsystem \
	libcups \
	libdrm \
	libepoxy \
	libglvnd \
	libinput \
	libx11 \
	libxcb \
	libxcomposite \
	libxdamage \
	libxext \
	libxfixes \
	libxrandr \
	libxss \
	libxtst \
	malcontent \
	mesa \
	ninja \
	nodejs \
	npm \
	nss \
	openbox \
	openssh \
	pipewire \
	pipewire-jack \
	pkgconf \
	plasma-desktop \
	python \
	python-dbus \
	python-gobject \
	python-pillow \
	qt6-base \
	qt6-declarative \
	qt6-multimedia-ffmpeg \
	qt6-tools \
	rsync \
	rust \
	seatd \
	slurp \
	socat \
	strace \
	sudo \
	tk \
	vulkan-swrast \
	vulkan-virtio \
	wayland \
	wev \
	weston \
	wireplumber \
	wl-clipboard \
	wmctrl \
	xdg-desktop-portal \
	xdg-desktop-portal-cosmic \
	xdg-desktop-portal-gnome \
	xdg-desktop-portal-gtk \
	xdg-desktop-portal-hyprland \
	xdg-desktop-portal-kde \
	xdg-desktop-portal-wlr \
	xdg-utils \
	xdotool \
	xterm \
	ydotool \
	wezterm \
	xorg-xev \
	xorg-xrandr \
	xorg-server \
	xorg-xauth \
	xorg-xcursorgen \
	xorg-xdpyinfo \
	xorg-xinit \
	xorg-xmessage \
	xorg-xwayland \
	xorg-xwininfo

if ! id -u "${vm_user}" >/dev/null 2>&1; then
	useradd --create-home --shell /bin/bash --groups wheel,video,render,seat,input "${vm_user}"
fi
usermod -aG wheel,video,render,seat,input "${vm_user}"

chmod 0755 /
install -d -m 0700 -o "${vm_user}" -g "${vm_user}" "/home/${vm_user}/.ssh"
install -d -m 0755 -o "${vm_user}" -g "${vm_user}" /workspace
install -d -m 0755 /usr/local/share/sky-cua-testing-vm/profiles

cat >/etc/udev/rules.d/80-sky-cua-uinput.rules <<'EOF'
KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"
EOF
chown root:root /etc/udev/rules.d/80-sky-cua-uinput.rules
chmod 0644 /etc/udev/rules.d/80-sky-cua-uinput.rules

cat >/etc/udev/rules.d/81-sky-cua-ydotool-input.rules <<'EOF'
SUBSYSTEM=="input", ATTRS{name}=="ydotoold virtual device", ENV{ID_INPUT}="1", ENV{ID_INPUT_KEY}="1", ENV{ID_INPUT_KEYBOARD}="1", ENV{ID_INPUT_MOUSE}="1", TAG+="uaccess", TAG+="seat", TAG-="power-switch"
EOF
chown root:root /etc/udev/rules.d/81-sky-cua-ydotool-input.rules
chmod 0644 /etc/udev/rules.d/81-sky-cua-ydotool-input.rules

udevadm control --reload-rules || true
udevadm trigger --subsystem-match=misc --sysname-match=uinput || true
udevadm trigger --subsystem-match=input || true
if [[ -e /dev/uinput ]]; then
	chgrp input /dev/uinput || true
	chmod 0660 /dev/uinput || true
fi

if [[ -f /etc/ssh/sshd_config.d/20-systemd-userdb.conf ]]; then
	mv -f /etc/ssh/sshd_config.d/20-systemd-userdb.conf /etc/ssh/sshd_config.d/20-systemd-userdb.conf.disabled
fi

cat >/etc/ssh/sshd_config.d/20-sky-cua-authorized-keys.conf <<'EOF'
AuthorizedKeysFile .ssh/authorized_keys
LogLevel INFO
EOF
chown root:root /etc/ssh/sshd_config.d/20-sky-cua-authorized-keys.conf
chmod 0644 /etc/ssh/sshd_config.d/20-sky-cua-authorized-keys.conf

if [[ -n "${codex_desktop_package}" ]]; then
	if [[ ! -f "${codex_desktop_package}" ]]; then
		printf 'CODEX_DESKTOP_PACKAGE does not exist: %s\n' "${codex_desktop_package}" >&2
		exit 66
	fi
	pacman -U --noconfirm "${codex_desktop_package}"
fi

# Cache Chrome installation: skip download if the binary already exists.
if [[ ! -x /opt/google/chrome/google-chrome ]]; then
	chrome_deb=/tmp/google-chrome-stable_current_amd64.deb
	chrome_tmp="$(mktemp -d)"
	python -c "from urllib.request import urlretrieve; urlretrieve('https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb', '${chrome_deb}')"
	bsdtar -xf "${chrome_deb}" -C "${chrome_tmp}"
	chrome_data="$(find "${chrome_tmp}" -maxdepth 1 -name 'data.tar.*' -print -quit)"
	test -n "${chrome_data}"
	bsdtar -xf "${chrome_data}" -C /
	ln -sf /opt/google/chrome/google-chrome /usr/bin/google-chrome-stable
	ln -sf /usr/bin/google-chrome-stable /usr/bin/google-chrome
	rm -rf "${chrome_tmp}" "${chrome_deb}"
fi

npm install -g "${opencode_npm_spec}"

# Install Pi and its MCP adapter for integration smoke lanes.
npm install -g @earendil-works/pi-coding-agent pi-mcp-adapter || {
	printf 'warning: Pi global install failed; smoke tests will need local fallback\n' >&2
}

runuser -u "${vm_user}" -- dbus-run-session -- bash -lc '
set -euo pipefail
gsettings set org.gnome.desktop.session idle-delay 0 || true
gsettings set org.gnome.desktop.screensaver lock-enabled false || true
gsettings set org.gnome.desktop.screensaver idle-activation-enabled false || true
gsettings set org.gnome.settings-daemon.plugins.power sleep-inactive-ac-type nothing || true
gsettings set org.gnome.settings-daemon.plugins.power sleep-inactive-ac-timeout 0 || true
'

cat >/usr/local/bin/sky-cua-testing-vm-session <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

session="${1:-${SKY_CUA_TESTING_VM_SESSION:-cosmic}}"
import_session_environment() {
  export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
  export DBUS_SESSION_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS:-unix:path=${XDG_RUNTIME_DIR}/bus}"
  dbus-update-activation-environment --systemd \
    DBUS_SESSION_BUS_ADDRESS \
    XDG_SESSION_ID \
    XDG_SESSION_CLASS \
    XDG_CURRENT_DESKTOP \
    XDG_SESSION_DESKTOP \
    DESKTOP_SESSION \
    XDG_SESSION_TYPE \
    WAYLAND_DISPLAY \
    DISPLAY \
    QT_QPA_PLATFORM \
    GDK_BACKEND \
    XCURSOR_THEME \
    XCURSOR_SIZE || true
  systemctl --user import-environment \
    DBUS_SESSION_BUS_ADDRESS \
    XDG_SESSION_ID \
    XDG_SESSION_CLASS \
    XDG_CURRENT_DESKTOP \
    XDG_SESSION_DESKTOP \
    DESKTOP_SESSION \
    XDG_SESSION_TYPE \
    WAYLAND_DISPLAY \
    DISPLAY \
    QT_QPA_PLATFORM \
    GDK_BACKEND \
    XCURSOR_THEME \
    XCURSOR_SIZE || true
}
run_session() {
  import_session_environment
  exec "$@"
}

case "${session}" in
  cosmic)
    export XDG_CURRENT_DESKTOP=COSMIC
    export XDG_SESSION_DESKTOP=cosmic
    export DESKTOP_SESSION=cosmic
    export XDG_SESSION_TYPE=wayland
    run_session cosmic-session
    ;;
  cosmic-blank|cosmic-transparent)
    export XDG_CURRENT_DESKTOP=COSMIC
    export XDG_SESSION_DESKTOP=cosmic
    export DESKTOP_SESSION=cosmic
    export XDG_SESSION_TYPE=wayland
    export XCURSOR_THEME=sky-cua-blank
    export XCURSOR_SIZE=24
    if [[ -f /workspace/scripts/install_blank_xcursor_theme.py ]]; then
      python3 /workspace/scripts/install_blank_xcursor_theme.py --theme-name sky-cua-blank --size "${XCURSOR_SIZE}" >/dev/null
    fi
    run_session cosmic-session
    ;;
  kde|plasma)
    export XDG_CURRENT_DESKTOP=KDE
    export XDG_SESSION_DESKTOP=KDE
    export DESKTOP_SESSION=plasma
    export XDG_SESSION_TYPE=wayland
    kwriteconfig6 --file kscreenlockerrc --group Daemon --key Autolock --type bool false || true
    kwriteconfig6 --file kscreenlockerrc --group Daemon --key LockOnResume --type bool false || true
    kwriteconfig6 --file kscreenlockerrc --group Daemon --key Timeout 0 || true
    systemctl --user mask plasma-powerdevil.service 2>/dev/null || true
    run_session startplasma-wayland
    ;;
  gnome)
    export XDG_CURRENT_DESKTOP=GNOME
    export XDG_SESSION_DESKTOP=gnome
    export DESKTOP_SESSION=gnome
    export XDG_SESSION_TYPE=wayland
    export XDG_SESSION_CLASS="${XDG_SESSION_CLASS:-user}"
    import_session_environment
    exec gnome-shell --wayland --display-server
    ;;
  hyprland)
    export XDG_CURRENT_DESKTOP=Hyprland
    export XDG_SESSION_DESKTOP=Hyprland
    export DESKTOP_SESSION=Hyprland
    export XDG_SESSION_TYPE=wayland
    run_session Hyprland
    ;;
  i3)
    export XDG_CURRENT_DESKTOP=i3
    export XDG_SESSION_DESKTOP=i3
    export DESKTOP_SESSION=i3
    export XDG_SESSION_TYPE=x11
    import_session_environment
    exec startx /usr/bin/i3
    ;;
  *)
    printf 'unknown testing VM session: %s\n' "${session}" >&2
    exit 64
    ;;
esac
EOF
chmod +x /usr/local/bin/sky-cua-testing-vm-session

cat >/etc/greetd/config.toml <<EOF
[terminal]
vt = 1

[default_session]
user = "${vm_user}"
command = "/usr/local/bin/sky-cua-testing-vm-session ${autologin_session}"
EOF

cat >/etc/gdm/custom.conf <<EOF
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

systemctl enable sshd.service
if [[ "${autologin_session}" == "gnome" ]]; then
	systemctl disable greetd.service >/dev/null 2>&1 || true
	systemctl enable gdm.service
else
	systemctl disable gdm.service >/dev/null 2>&1 || true
	systemctl enable greetd.service
fi
systemctl enable seatd.service
systemctl --global enable ydotool.service

printf 'Arch testing VM provisioned for user %s with default session %s and OpenCode %s\n' "${vm_user}" "${autologin_session}" "${opencode_npm_spec}"
