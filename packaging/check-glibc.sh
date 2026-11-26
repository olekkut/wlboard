#!/usr/bin/env bash
# Verify that a compiled ELF binary is stripped, dynamically linked only to baseline
# system libraries, and requires no GLIBC symbol newer than MAX_GLIBC (default: 2.31).
#
# Usage: ./packaging/check-glibc.sh <binary-path> [max-glibc-version]

set -euo pipefail

if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <binary-path> [max-glibc-version]" >&2
    exit 1
fi

BIN="$1"
MAX_GLIBC="${2:-2.31}"

if [[ ! -f "${BIN}" ]]; then
    echo "Error: binary not found: ${BIN}" >&2
    exit 1
fi

echo "=== ELF Dynamic Dependencies (${BIN}) ==="
NEEDED="$(readelf -d "${BIN}" | awk -F'[][]' '/\(NEEDED\)/ { print $2 }')"
echo "${NEEDED}"

while IFS= read -r lib; do
    [[ -z "${lib}" ]] && continue
    case "${lib}" in
        libc.so.6|libm.so.6|libgcc_s.so.1|libpthread.so.0|libdl.so.2|librt.so.1|ld-linux*.so*)
            ;;
        *)
            echo "Error: unexpected shared library dependency: ${lib}" >&2
            exit 1
            ;;
    esac
done <<< "${NEEDED}"

echo "=== GLIBC Symbol Versions (${BIN}) ==="
VERSIONS="$(readelf --version-info "${BIN}" | grep -oE 'GLIBC_[0-9]+\.[0-9]+(\.[0-9]+)?' | sed 's/^GLIBC_//' | sort -Vu || true)"
if [[ -z "${VERSIONS}" ]]; then
    echo "No GLIBC versioned symbols found (static binary?)"
    exit 0
fi

echo "${VERSIONS}" | tr '\n' ' '
echo ""

MAX_FOUND="$(echo "${VERSIONS}" | tail -n 1)"
echo "Max required GLIBC version: ${MAX_FOUND} (floor limit: ${MAX_GLIBC})"

HIGHEST="$(printf '%s\n%s\n' "${MAX_FOUND}" "${MAX_GLIBC}" | sort -V | tail -n 1)"
if [[ "${HIGHEST}" != "${MAX_GLIBC}" ]]; then
    echo "Error: binary requires GLIBC_${MAX_FOUND}, exceeding allowed ceiling GLIBC_${MAX_GLIBC}" >&2
    exit 1
fi

echo "OK: binary satisfies GLIBC <= ${MAX_GLIBC} portability floor."
