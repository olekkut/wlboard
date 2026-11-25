#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VERSION="0.1.0-1"
ARCH="amd64"
PKG_NAME="wlboard_${VERSION}_${ARCH}.deb"
STAGE_DIR="$(mktemp -d)"

trap 'rm -rf "${STAGE_DIR}"' EXIT

echo "Building release binary..."
cargo build --release --manifest-path "${ROOT_DIR}/Cargo.toml"

BIN_PATH="${ROOT_DIR}/target/release/wlboard"
install -Dm755 "${BIN_PATH}" "${STAGE_DIR}/usr/bin/wlboard"
install -Dm644 "${ROOT_DIR}/packaging/deb/io.github.olekkut.wlboard.desktop" \
    "${STAGE_DIR}/usr/share/applications/io.github.olekkut.wlboard.desktop"
install -Dm644 "${ROOT_DIR}/assets/io.github.olekkut.wlboard.svg" \
    "${STAGE_DIR}/usr/share/icons/hicolor/scalable/apps/io.github.olekkut.wlboard.svg"
install -Dm644 "${ROOT_DIR}/packaging/deb/70-wlboard-uinput.rules" \
    "${STAGE_DIR}/usr/share/wlboard/udev/70-wlboard-uinput.rules"
install -Dm755 "${ROOT_DIR}/packaging/deb/setup-uinput.sh" \
    "${STAGE_DIR}/usr/share/wlboard/udev/setup-uinput.sh"
install -Dm644 "${ROOT_DIR}/LICENSE" \
    "${STAGE_DIR}/usr/share/doc/wlboard/copyright"

install -Dm644 "${ROOT_DIR}/packaging/deb/control" "${STAGE_DIR}/DEBIAN/control"
install -Dm755 "${ROOT_DIR}/packaging/deb/postinst" "${STAGE_DIR}/DEBIAN/postinst"

OUT_DEB="${ROOT_DIR}/${PKG_NAME}"
dpkg-deb --root-owner-group --build "${STAGE_DIR}" "${OUT_DEB}"
sha256sum "${OUT_DEB}"
ls -lh "${OUT_DEB}"
