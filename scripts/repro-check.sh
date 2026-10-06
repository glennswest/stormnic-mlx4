#!/bin/bash
# Build this commit twice and fail unless the two stormnic-mlx4.efi images are
# byte-identical (#13). The builds differ in what changes between sc-build
# jobs and between builders (stormbootx's nic-drivers build): the checkout
# directory and CARGO_HOME. Both builds use the same cargo cache, copied, so no
# network is needed beyond what `--locked` already fetched.
#
#   sc-build scripts/repro-check.sh
#
# Runs on the build box only (sc-build); scratch goes under $TMPDIR, which
# sc-build puts on the job's own drive.
set -euo pipefail

say() { printf '==> %s\n' "$*"; }
here=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/repro.XXXXXX")
export RUSTUP_HOME=${RUSTUP_HOME:-$HOME/.rustup}
home0=${CARGO_HOME:-$HOME/.cargo}
target=x86_64-unknown-uefi

# Make sure every dependency is in the original CARGO_HOME first.
(cd "$here" && cargo fetch --locked --target "$target" >/dev/null)

# Build $1 (a checkout dir name) with its own CARGO_HOME, print the image path.
build() {
    local name=$1 src="$work/$1/src/stormnic-mlx4" ch="$work/$1/cargo-home"
    mkdir -p "$src" "$ch"
    git -C "$here" archive HEAD | tar -x -C "$src"
    cp -a "$home0/registry" "$ch/"
    (cd "$src" && CARGO_HOME="$ch" CARGO_TARGET_DIR="$src/target" \
        cargo build --locked --offline --release --target "$target" >&2)
    echo "$src/target/$target/release/stormnic-mlx4.efi"
}

a=$(build a)
b=$(build build-in-a-longer-directory-name)
say "A: $a"
say "B: $b"
sha256sum "$a" "$b" | sed 's|  .*/\([^/]*/src/\)|  \1|'

# PE COFF TimeDateStamp (the linker may write the build time there).
stamp() { local pe; pe=$(od -An -tu4 -j 60 -N4 "$1" | tr -d ' '); od -An -tu4 -j $((pe + 8)) -N4 "$1" | tr -d ' '; }
say "PE TimeDateStamp: A $(stamp "$a"), B $(stamp "$b")"

# Absolute build paths embedded in each image.
paths="$work|/build/|/registry/src/|/\.cargo/|/rustc/|/home/"
for x in "A $a" "B $b"; do
    f=${x#* }
    n=$(strings -n 6 "$f" | grep -c -E "$paths" || true)
    say "${x%% *}: $n embedded path string(s)"
    strings -n 6 "$f" | grep -E "$paths" | sort -u | head -8 | sed 's/^/    /'
done

if cmp -s "$a" "$b"; then
    say "reproducible: the two images are identical ($(stat -c %s "$a") bytes)"
else
    say "NOT reproducible: $(cmp -l "$a" "$b" | wc -l) byte(s) differ"
    exit 1
fi
