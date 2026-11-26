#!/usr/bin/env bash
# Unified local & CI multi-distro packager for wlboard.
# Builds .deb, .rpm, .pkg.tar.zst (via nfpm) and generic .tar.gz for x86_64 or aarch64.
#
# Usage:
#   ./packaging/build-packages.sh [options]
#
# Options:
#   --version <ver>     Package version without leading 'v' (default: Cargo.toml workspace version)
#   --arch <arch>       Target arch: x86_64 or aarch64 (default: host uname -m)
#   --bin <path>        Path to prebuilt stripped wlboard binary (default: builds target/release/wlboard)
#   --out-dir <dir>     Output directory (default: dist)
#   --formats <list>    Comma-separated subset of: deb,rpm,archlinux,tarball,all (default: all)
#   --checksums         Write/update SHA256SUMS inside --out-dir

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION=""
ARCH="$(uname -m)"
BIN_PATH=""
OUT_DIR="${ROOT_DIR}/dist"
FORMATS="all"
WRITE_SUMS=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --version)
            VERSION="${2#v}"
            shift 2
            ;;
        --arch)
            ARCH="$2"
            shift 2
            ;;
        --bin)
            BIN_PATH="$2"
            shift 2
            ;;
        --out-dir)
            OUT_DIR="$2"
            shift 2
            ;;
        --formats)
            FORMATS="$2"
            shift 2
            ;;
        --checksums)
            WRITE_SUMS=1
            shift
            ;;
        -h|--help)
            sed -n '2,17p' "$0"
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

if [[ -z "${VERSION}" ]]; then
    VERSION="$(awk -F'"' '/^version = / { print $2; exit }' "${ROOT_DIR}/Cargo.toml")"
fi

case "${ARCH}" in
    x86_64|amd64)
        ARCH="x86_64"
        NFPM_ARCH="amd64"
        ;;
    aarch64|arm64)
        ARCH="aarch64"
        NFPM_ARCH="arm64"
        ;;
    *)
        echo "Unsupported arch '${ARCH}'. Use x86_64 or aarch64." >&2
        exit 1
        ;;
esac

if [[ -z "${BIN_PATH}" ]]; then
    echo "Building release binary for host architecture..."
    cargo build --release --manifest-path "${ROOT_DIR}/Cargo.toml"
    BIN_PATH="${ROOT_DIR}/target/release/wlboard"
fi

if [[ ! -f "${BIN_PATH}" ]]; then
    echo "Error: binary not found at ${BIN_PATH}" >&2
    exit 1
fi

mkdir -p "${OUT_DIR}"
ABS_BIN_PATH="$(cd "$(dirname "${BIN_PATH}")" && pwd)/$(basename "${BIN_PATH}")"
ABS_OUT_DIR="$(cd "${OUT_DIR}" && pwd)"

if [[ "${FORMATS}" == "all" ]]; then
    FORMATS="deb,rpm,archlinux,tarball"
fi

NFPM_BIN="$(command -v nfpm || true)"
if [[ -z "${NFPM_BIN}" && -x "${HOME}/go/bin/nfpm" ]]; then
    NFPM_BIN="${HOME}/go/bin/nfpm"
fi
if [[ -z "${NFPM_BIN}" ]] && command -v go >/dev/null 2>&1; then
    GO_NFPM="$(go env GOPATH)/bin/nfpm"
    if [[ -x "${GO_NFPM}" ]]; then
        NFPM_BIN="${GO_NFPM}"
    fi
fi

build_nfpm_pkg() {
    local packager="$1"
    local ext="$2"
    local target_file="${ABS_OUT_DIR}/wlboard-${VERSION}-linux-${ARCH}.${ext}"
    local tmp_cfg
    tmp_cfg="$(mktemp)"
    trap 'rm -f "${tmp_cfg}"' RETURN

    if [[ -z "${NFPM_BIN}" ]]; then
        echo "Error: nfpm not found in PATH or GOPATH/bin" >&2
        exit 1
    fi

    sed \
        -e "s|\${VERSION}|${VERSION}|g" \
        -e "s|\${NFPM_ARCH}|${NFPM_ARCH}|g" \
        -e "s|\${BIN_PATH}|${ABS_BIN_PATH}|g" \
        "${ROOT_DIR}/packaging/nfpm.yaml" > "${tmp_cfg}"

    echo "Packaging ${target_file} (${packager}, arch=${NFPM_ARCH})..."
    (
        cd "${ROOT_DIR}"
        "${NFPM_BIN}" package \
            --config "${tmp_cfg}" \
            --packager "${packager}" \
            --target "${target_file}"
    )
}

build_tarball() {
    local pkg_stem="wlboard-${VERSION}-linux-${ARCH}"
    local target_file="${ABS_OUT_DIR}/${pkg_stem}.tar.gz"
    local stage_dir
    stage_dir="$(mktemp -d)"
    trap 'rm -rf "${stage_dir}"' RETURN

    local root="${stage_dir}/${pkg_stem}"
    install -d -m 0755 "${root}"
    install -m 0755 "${ABS_BIN_PATH}" "${root}/wlboard"
    install -m 0755 "${ROOT_DIR}/packaging/deb/wlboard-im" "${root}/wlboard-im"
    install -m 0644 "${ROOT_DIR}/packaging/deb/io.github.olekkut.wlboard.desktop" "${root}/io.github.olekkut.wlboard.desktop"
    install -m 0644 "${ROOT_DIR}/packaging/deb/io.github.olekkut.wlboard-im.desktop" "${root}/io.github.olekkut.wlboard-im.desktop"
    install -m 0644 "${ROOT_DIR}/assets/io.github.olekkut.wlboard.svg" "${root}/io.github.olekkut.wlboard.svg"
    install -m 0644 "${ROOT_DIR}/packaging/deb/70-wlboard-uinput.rules" "${root}/70-wlboard-uinput.rules"
    install -m 0644 "${ROOT_DIR}/packaging/common/wlboard-uinput.conf" "${root}/wlboard-uinput.conf"
    install -m 0755 "${ROOT_DIR}/packaging/deb/setup-uinput.sh" "${root}/setup-uinput.sh"
    install -m 0755 "${ROOT_DIR}/packaging/deb/setup-sddm.sh" "${root}/setup-sddm.sh"
    install -m 0755 "${ROOT_DIR}/packaging/deb/wlboard-sddm-kwin" "${root}/wlboard-sddm-kwin"
    install -m 0644 "${ROOT_DIR}/LICENSE" "${root}/LICENSE"
    install -m 0644 "${ROOT_DIR}/README.md" "${root}/README.md"
    install -m 0755 "${ROOT_DIR}/packaging/tarball/install.sh" "${root}/install.sh"
    install -m 0755 "${ROOT_DIR}/packaging/tarball/uninstall.sh" "${root}/uninstall.sh"

    echo "Packaging ${target_file}..."
    tar --sort=name --owner=0 --group=0 --numeric-owner \
        -C "${stage_dir}" -czf "${target_file}" "${pkg_stem}"
}

IFS=',' read -r -a fmt_arr <<< "${FORMATS}"
for fmt in "${fmt_arr[@]}"; do
    case "${fmt}" in
        deb)
            build_nfpm_pkg deb deb
            ;;
        rpm)
            build_nfpm_pkg rpm rpm
            ;;
        archlinux|arch|pkg)
            build_nfpm_pkg archlinux pkg.tar.zst
            ;;
        tarball|tar|targz)
            build_tarball
            ;;
        *)
            echo "Unknown format '${fmt}'. Use deb, rpm, archlinux, tarball, or all." >&2
            exit 1
            ;;
    esac
done

if [[ "${WRITE_SUMS}" -eq 1 ]]; then
    (
        cd "${ABS_OUT_DIR}"
        shopt -s nullglob
        files=(wlboard-*.deb wlboard-*.rpm wlboard-*.pkg.tar.zst wlboard-*.tar.gz)
        if [[ ${#files[@]} -gt 0 ]]; then
            LC_ALL=C sha256sum "${files[@]}" > SHA256SUMS
            echo "Wrote ${ABS_OUT_DIR}/SHA256SUMS"
        fi
    )
fi

ls -lh "${ABS_OUT_DIR}"
