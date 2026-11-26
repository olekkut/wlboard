#!/usr/bin/env bash
# Render packaging/aur/PKGBUILD from PKGBUILD.in using a release SHA256SUMS file.
# Usage:
#   ./packaging/aur/update-pkgbuild.sh <version> <path-to-SHA256SUMS> [output-PKGBUILD]
# Example:
#   ./packaging/aur/update-pkgbuild.sh 0.4.0 dist/SHA256SUMS packaging/aur/PKGBUILD

set -euo pipefail

if [[ $# -lt 2 ]]; then
    echo "Usage: $0 <version> <SHA256SUMS-file> [output-path]" >&2
    exit 1
fi

VERSION="${1#v}"
SUMS_FILE="$2"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT_PATH="${3:-${SCRIPT_DIR}/PKGBUILD}"

if [[ ! -f "${SUMS_FILE}" ]]; then
    echo "Error: SHA256SUMS file not found: ${SUMS_FILE}" >&2
    exit 1
fi

SHA_X86_64="$(awk -v f="wlboard-${VERSION}-linux-x86_64.tar.gz" '$2 == f || $2 == ("*" f) { print $1; exit }' "${SUMS_FILE}")"
SHA_AARCH64="$(awk -v f="wlboard-${VERSION}-linux-aarch64.tar.gz" '$2 == f || $2 == ("*" f) { print $1; exit }' "${SUMS_FILE}")"

if [[ -z "${SHA_X86_64}" || -z "${SHA_AARCH64}" ]]; then
    echo "Error: could not find both x86_64 and aarch64 tarball checksums in ${SUMS_FILE}" >&2
    exit 1
fi

sed \
    -e "s|@VERSION@|${VERSION}|g" \
    -e "s|@SHA256_X86_64@|${SHA_X86_64}|g" \
    -e "s|@SHA256_AARCH64@|${SHA_AARCH64}|g" \
    "${SCRIPT_DIR}/PKGBUILD.in" > "${OUT_PATH}"

echo "Rendered ${OUT_PATH} for wlboard-bin ${VERSION}"
echo "  x86_64 : ${SHA_X86_64}"
echo "  aarch64: ${SHA_AARCH64}"
echo "Next steps for AUR publish:"
echo "  cp \"${OUT_PATH}\" \"${SCRIPT_DIR}/wlboard-bin.install\" /path/to/aur-wlboard-bin/"
echo "  cd /path/to/aur-wlboard-bin && makepkg --printsrcinfo > .SRCINFO && git commit -am \"Update to ${VERSION}\" && git push"
