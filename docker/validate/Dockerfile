# syntax=docker/dockerfile:1
# Headless validation image for the sky-cua release package. Proves install.py
# wires sky-cua up for Codex (file-based compat setup) and OpenCode without a
# desktop session. No credentials are baked in - auth/config bind-mount at run.
FROM archlinux:latest

# Runtime libraries mirror installer.py SYSTEM_PACKAGES["pacman"], plus python
# (install.py + assertions) and node/npm (opencode). dbus gives the MCP server a
# session bus for the stdio handshake.
RUN pacman -Syu --noconfirm --needed \
        at-spi2-core dbus gst-plugins-base gst-plugins-good gstreamer \
        libxkbcommon wayland xdg-desktop-portal ydotool \
        python nodejs npm \
    && pacman -Scc --noconfirm

# OpenCode pinned to the host-proven version (matches the testing-vm provisioner).
ARG OPENCODE_NPM_SPEC=opencode-ai@1.14.51
RUN npm install -g "${OPENCODE_NPM_SPEC}"

WORKDIR /work
COPY assert_install.py /work/assert_install.py
