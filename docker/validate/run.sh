#!/usr/bin/env bash
# Build the headless validation image and run install.py from a release tarball
# inside a container, then assert the fixed install tree and stable projections.
# Host auth/config is optional and must be explicitly requested; default
# validation uses an ephemeral container home.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
image="sky-cua-validate:latest"
tarball=""
with_host_auth=0

usage() {
    cat >&2 <<'EOF'
Usage: run.sh --tarball <path-to-sky-cua-*.tar.gz> [--with-host-auth|--no-host-auth]

  --tarball         Archive built by python3 install.py build (required).
  --with-host-auth  Copy portable host Codex auth/config into a writable
                    temp mounts. Off by default; use only for trusted tarballs.
  --no-host-auth    Keep validation isolated from host auth/config (default).
EOF
    exit 2
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --tarball) tarball="$2"; shift 2 ;;
        --with-host-auth) with_host_auth=1; shift ;;
        --no-host-auth) with_host_auth=0; shift ;;
        -h|--help) usage ;;
        *) echo "unknown argument: $1" >&2; usage ;;
    esac
done
[[ -n "$tarball" && -f "$tarball" ]] || { echo "error: --tarball must point at an existing file" >&2; usage; }
tarball="$(cd "$(dirname "$tarball")" && pwd)/$(basename "$tarball")"

echo "==> building $image"
docker build -t "$image" "$here"

mounts=(-v "${tarball}:/work/package.tar.gz:ro")

# Without host auth the container uses its own ephemeral /root, so there is
# nothing on the host to bind-mount or clean. With host auth, stage copies of
# the portable settings into writable temp dirs and bind-mount them; the
# container runs as root, so clean any root-owned leftovers via the image
# before dropping the temp dir.
if [[ "$with_host_auth" == 1 ]]; then
    workdir="$(mktemp -d)"
    cleanup() {
        docker run --rm -v "${workdir}:/cleanup" "$image" \
            chown -R "$(id -u):$(id -g)" /cleanup >/dev/null 2>&1 || true
        rm -rf "$workdir" 2>/dev/null || true
    }
    trap cleanup EXIT

    codex_mount="${workdir}/codex"
    mkdir -p "$codex_mount"
    mounts+=(-v "${codex_mount}:/root/.codex")

    # Codex: copy the portable settings subset only (never logs/db/state).
    for name in auth.json config.toml config.json version.json; do
        [[ -f "${HOME}/.codex/${name}" ]] && cp -a "${HOME}/.codex/${name}" "${codex_mount}/" || true
    done
fi

echo "==> running install + assertions (headless, under a session bus)"
docker run --rm "${mounts[@]}" "$image" dbus-run-session -- bash -euo pipefail -c '
    tar xzf /work/package.tar.gz -C /opt
    pkg="$(find /opt -maxdepth 1 -type d -name "sky-cua-*" | head -1)"
    echo "package: ${pkg}"
    python3 "${pkg}/install.py" install
    SKY_CUA_PACKAGE_ROOT="${pkg}" python3 /work/assert_install.py
'
echo "==> validation passed"
