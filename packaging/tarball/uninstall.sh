#!/bin/sh
# POSIX uninstaller for wlboard generic Linux release tarballs.
# Supports PREFIX (default /usr/local), DESTDIR, and --with-udev.

set -eu

PREFIX="${PREFIX:-/usr/local}"
DESTDIR="${DESTDIR:-}"
WITH_UDEV=0

for arg in "$@"; do
    case "$arg" in
        --with-udev)
            WITH_UDEV=1
            ;;
        --prefix=*)
            PREFIX="${arg#--prefix=}"
            ;;
        --destdir=*)
            DESTDIR="${arg#--destdir=}"
            ;;
        -h|--help)
            cat <<'EOF'
Usage: ./uninstall.sh [--with-udev] [--prefix=DIR] [--destdir=DIR]

Environment variables:
  PREFIX    Installation prefix (default: /usr/local)
  DESTDIR   Optional staging root prepended to installation paths
EOF
            exit 0
            ;;
        *)
            echo "Unknown option: $arg" >&2
            exit 1
            ;;
    esac
done

case "$PREFIX" in
    /) ;;
    */) PREFIX="${PREFIX%/}" ;;
esac

TARGET_ROOT="${DESTDIR}${PREFIX}"

rm -f \
    "${TARGET_ROOT}/bin/wlboard" \
    "${TARGET_ROOT}/bin/wlboard-im" \
    "${TARGET_ROOT}/share/applications/io.github.olekkut.wlboard.desktop" \
    "${TARGET_ROOT}/share/applications/io.github.olekkut.wlboard-im.desktop" \
    "${TARGET_ROOT}/share/icons/hicolor/scalable/apps/io.github.olekkut.wlboard.svg" \
    "${TARGET_ROOT}/share/wlboard/udev/70-wlboard-uinput.rules" \
    "${TARGET_ROOT}/share/wlboard/udev/setup-uinput.sh" \
    "${TARGET_ROOT}/share/wlboard/scripts/setup-sddm.sh" \
    "${TARGET_ROOT}/share/wlboard/scripts/wlboard-sddm-kwin" \
    "${TARGET_ROOT}/share/doc/wlboard/copyright" \
    "${TARGET_ROOT}/share/doc/wlboard/README.md" \
    "${TARGET_ROOT}/share/licenses/wlboard/LICENSE"

rmdir "${TARGET_ROOT}/share/wlboard/udev" 2>/dev/null || true
rmdir "${TARGET_ROOT}/share/wlboard/scripts" 2>/dev/null || true
rmdir "${TARGET_ROOT}/share/wlboard" 2>/dev/null || true
rmdir "${TARGET_ROOT}/share/doc/wlboard" 2>/dev/null || true
rmdir "${TARGET_ROOT}/share/licenses/wlboard" 2>/dev/null || true

if [ "$WITH_UDEV" -eq 1 ]; then
    RULE_PATH="${DESTDIR}/usr/lib/udev/rules.d/70-wlboard-uinput.rules"
    MOD_PATH="${DESTDIR}/usr/lib/modules-load.d/wlboard-uinput.conf"
    if [ -n "$DESTDIR" ] || [ "$(id -u)" -eq 0 ]; then
        rm -f "$RULE_PATH" "$MOD_PATH"
        if [ -z "$DESTDIR" ] && [ -d /run/udev ] && command -v udevadm >/dev/null 2>&1; then
            udevadm control --reload-rules 2>/dev/null || true
        fi
    elif command -v sudo >/dev/null 2>&1; then
        sudo rm -f "$RULE_PATH" "$MOD_PATH"
        if [ -d /run/udev ] && command -v udevadm >/dev/null 2>&1; then
            sudo udevadm control --reload-rules 2>/dev/null || true
        fi
    fi
fi

if [ -z "$DESTDIR" ]; then
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q "${TARGET_ROOT}/share/applications" 2>/dev/null || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q -t -f "${TARGET_ROOT}/share/icons/hicolor" 2>/dev/null || true
    fi
fi

echo "Uninstalled wlboard from ${TARGET_ROOT}"
