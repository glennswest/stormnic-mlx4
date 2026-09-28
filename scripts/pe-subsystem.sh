#!/bin/sh
# Print a PE image's Subsystem and fail unless it is 11
# (IMAGE_SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER). 10 is an EFI application.
#   scripts/pe-subsystem.sh target/x86_64-unknown-uefi/release/stormnic-mlx4.efi
set -eu
f=${1:?usage: $0 IMAGE.efi}
u16() { od -An -tu2 -j "$1" -N2 "$f" | tr -d ' '; }
u32() { od -An -tu4 -j "$1" -N4 "$f" | tr -d ' '; }
pe=$(u32 60)                 # e_lfanew
sub=$(u16 $((pe + 4 + 20 + 68)))  # PE sig + COFF header + OptionalHeader.Subsystem
echo "$f: subsystem $sub"
[ "$sub" = 11 ]
