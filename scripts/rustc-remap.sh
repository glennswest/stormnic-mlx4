#!/bin/sh
# rustc wrapper (#13): make the image independent of where it is built.
# `.cargo/config.toml` names it, so every `cargo build` in this checkout uses
# it, including stormbootx's nic-drivers build.
#
# rustc embeds absolute source paths in panic and log locations (the `uefi`
# crate's `file!()`): each dependency's directory under CARGO_HOME, and the
# toolchain's sysroot. Both differ between sc-build jobs and between builders,
# so the image did too. This remaps them to fixed names. Cargo calls a wrapper
# as `WRAPPER RUSTC ARGS...`.
rustc=$1
shift
# Only real crate compiles set CARGO_MANIFEST_DIR; cargo's own probes
# (`rustc -vV`, `--print`) go through unchanged.
if [ -z "${CARGO_MANIFEST_DIR:-}" ]; then
    exec "$rustc" "$@"
fi
sysroot=$("$rustc" --print sysroot)
# Most specific last: rustc applies the last matching prefix.
exec "$rustc" "$@" \
    --remap-path-prefix="$sysroot=/rust" \
    --remap-path-prefix="${CARGO_HOME:-$HOME/.cargo}=/cargo" \
    --remap-path-prefix="$CARGO_MANIFEST_DIR=/src/${CARGO_PKG_NAME:-crate}-${CARGO_PKG_VERSION:-0}"
