# syntax=docker/dockerfile:1
# Headless validation image for the sky-cua release package. Proves install.py
# installs the fixed standalone layout without a desktop session. No credentials
# are baked in; optional auth/config bind mounts remain explicit at run time.
FROM archlinux:latest

# Runtime libraries support the packaged binaries plus Python for install.py and
# the post-install assertions.
RUN pacman -Syu --noconfirm --needed \
        at-spi2-core dbus gst-plugins-base gst-plugins-good gstreamer \
        libxkbcommon wayland xdg-desktop-portal ydotool \
        python \
    && pacman -Scc --noconfirm

WORKDIR /work
COPY assert_install.py /work/assert_install.py
