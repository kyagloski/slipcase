#!/usr/bin/env bash
# Build script for Linux (and cross-compiling to Windows).
#
#   ./build.sh                 optimised build for this machine  -> dist/slipcase-linux-x86_64
#   ./build.sh linux-portable  build inside Docker on an old glibc so it runs on most distros
#   ./build.sh windows         cross-build a static .exe (needs `cargo install cargo-xwin`, or mingw-w64)
#   ./build.sh all             native + windows
#   NO_GAMEPAD=1 ./build.sh    build without controller support (drops the libudev dependency)
#
# What "static" means here: all Rust code, the HTTP/TLS stack, image decoders and the
# engine are linked into the single binary. The only things loaded at run time are the
# things that can never be static on a desktop: libc, and the GPU driver / windowing
# libraries (Vulkan, X11/Wayland), which are dlopen()ed by design.
set -euo pipefail
cd "$(dirname "$0")"

NAME=slipcase
FEATURES=()
[[ "${NO_GAMEPAD:-0}" == "1" ]] && FEATURES=(--no-default-features)
mkdir -p dist

# Copy next to the target and rename over it: works even while the old binary is running
# (a plain cp onto a running executable fails with "Text file busy").
install_bin() {
    cp "$1" "$2.tmp" && mv -f "$2.tmp" "$2"
}

native() {
    if [[ "${NO_GAMEPAD:-0}" != "1" ]] && ! pkg-config --exists libudev 2>/dev/null; then
        echo "!! libudev not found. Install it (Debian/Ubuntu: libudev-dev, Fedora: systemd-devel, Arch: systemd-libs)"
        echo "   or build without controller support:  NO_GAMEPAD=1 ./build.sh"
        exit 1
    fi
    cargo build --release "${FEATURES[@]}"
    install_bin "target/release/$NAME" "dist/$NAME-linux-$(uname -m)"
    echo "-> dist/$NAME-linux-$(uname -m)"
}

portable() {
    command -v docker >/dev/null || { echo "docker is required for linux-portable"; exit 1; }
    docker run --rm -v "$PWD":/src -w /src \
        -e CARGO_TARGET_DIR=/src/target-portable -e CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback \
        rust:1-bullseye bash -c "apt-get update -qq && apt-get install -y -qq pkg-config libudev-dev >/dev/null && cargo build --release ${FEATURES[*]:-}"
    install_bin "target-portable/release/$NAME" "dist/$NAME-linux-portable-x86_64"
    echo "-> dist/$NAME-linux-portable-x86_64 (glibc 2.31+)"
}

windows() {
    local target=x86_64-pc-windows-msvc
    if command -v cargo-xwin >/dev/null; then
        rustup target add $target
        cargo xwin build --release --target $target "${FEATURES[@]}"
    else
        target=x86_64-pc-windows-gnu
        rustup target add $target
        cargo build --release --target $target "${FEATURES[@]}"
    fi
    install_bin "target/$target/release/$NAME.exe" "dist/$NAME-windows-x86_64.exe"
    echo "-> dist/$NAME-windows-x86_64.exe"
}

case "${1:-native}" in
    native|linux) native ;;
    linux-portable) portable ;;
    windows) windows ;;
    all) native; windows ;;
    *) sed -n '2,14p' "$0"; exit 1 ;;
esac

echo
echo "Linked libraries:"; ldd "dist/$NAME-linux-$(uname -m)" 2>/dev/null | awk '{print "  " $1}' || true
