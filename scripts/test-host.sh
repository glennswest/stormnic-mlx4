#!/usr/bin/env bash
# Host tests of the UEFI-independent modules against simulated firmware.
# Run only on the remote build box, through sc-build after pushing.
set -euo pipefail
mkdir -p "${CARGO_TARGET_DIR:-target}"
for t in bars; do
    rustc --edition=2021 --test "test/$t.rs" -o "${CARGO_TARGET_DIR:-target}/$t-tests"
    "${CARGO_TARGET_DIR:-target}/$t-tests"
done
