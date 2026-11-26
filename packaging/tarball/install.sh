#!/bin/sh
# POSIX installer for wlboard generic Linux release tarballs.
# Supports:
#   PREFIX=/usr/local   (default; override e.g. PREFIX=/usr or PREFIX="$HOME/.local")
#   DESTDIR=            (optional staging root for packaging or dry-run tests)
#   --with-udev         (also install /usr/lib/udev/rules.d/70-wlboard-uinput.rules
#                        and /usr/lib/modules-load.d/wlboard-uinput.conf)

set -eu

SCRIPT_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)"
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
Usage: ./install.sh [--with-udev] [--prefix=DIR] [--destdir=DIR]

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

# Trim trailing slash from PREFIX (except root '/')
case "$PREFIX" in
    /) ;;
    */) PREFIX="${PREFIX%/}" ;;
esac

TARGET_ROOT="${DESTDIR}${PREFIX}"

install -d -m 0755 \
    "${TARGET_ROOT}/bin" \
    "${TARGET_ROOT}/share/applications" \
    "${TARGET_ROOT}/share/icons/hicolor/scalable/apps" \
    "${TARGET_ROOT}/share/wlboard/udev" \
    "${TARGET_ROOT}/share/wlboard/scripts" \
    "${TARGET_ROOT}/share/doc/wlboard" \
    "${TARGET_ROOT}/share/licenses/wlboard"

install -m 0755 "${SCRIPT_DIR}/wlboard" "${TARGET_ROOT}/bin/wlboard"

# Rewrite /usr/bin paths when installing to a non-/usr prefix so launchers resolve cleanly
sed "s|/usr/bin/wlboard|${PREFIX}/bin/wlboard|g" "${SCRIPT_DIR}/wlboard-im" \
    > "${TARGET_ROOT}/bin/wlboard-im"
chmod 0755 "${TARGET_ROOT}/bin/wlboard-im"

sed "s|/usr/bin/wlboard|${PREFIX}/bin/wlboard|g" "${SCRIPT_DIR}/io.github.olekkut.wlboard.desktop" \
    > "${TARGET_ROOT}/share/applications/io.github.olekkut.wlboard.desktop"
chmod 0644 "${TARGET_ROOT}/share/applications/io.github.olekkut.wlboard.desktop"

sed "s|/usr/bin/wlboard-im|${PREFIX}/bin/wlboard-im|g" "${SCRIPT_DIR}/io.github.olekkut.wlboard-im.desktop" \
    > "${TARGET_ROOT}/share/applications/io.github.olekkut.wlboard-im.desktop"
chmod 0644 "${TARGET_ROOT}/share/applications/io.github.olekkut.wlboard-im.desktop"

install -m 0644 "${SCRIPT_DIR}/io.github.olekkut.wlboard.svg" \
    "${TARGET_ROOT}/share/icons/hicolor/scalable/apps/io.github.olekkut.wlboard.svg"
install -m 0644 "${SCRIPT_DIR}/70-wlboard-uinput.rules" \
    "${TARGET_ROOT}/share/wlboard/udev/70-wlboard-uinput.rules"
install -m 0755 "${SCRIPT_DIR}/setup-uinput.sh" \
    "${TARGET_ROOT}/share/wlboard/udev/setup-uinput.sh"
install -m 0755 "${SCRIPT_DIR}/setup-sddm.sh" \
    "${TARGET_ROOT}/share/wlboard/scripts/setup-sddm.sh"
install -m 0755 "${SCRIPT_DIR}/wlboard-sddm-kwin" \
    "${TARGET_ROOT}/share/wlboard/scripts/wlboard-sddm-kwin"
install -m 0644 "${SCRIPT_DIR}/LICENSE" \
    "${TARGET_ROOT}/share/doc/wlboard/copyright"
install -m 0644 "${SCRIPT_DIR}/LICENSE" \
    "${TARGET_ROOT}/share/licenses/wlboard/LICENSE"
if [ -f "${SCRIPT_DIR}/README.md" ]; then
    install -m 0644 "${SCRIPT_DIR}/README.md" "${TARGET_ROOT}/share/doc/wlboard/README.md"
fi

if [ "$WITH_UDEV" -eq 1 ]; then
    UDEV_DST="${DESTDIR}/usr/lib/udev/rules.d"
    MOD_DST="${DESTDIR}/usr/lib/modules-load.d"
    if [ -n "$DESTDIR" ] || [ "$(id -u)" -eq 0 ]; then
        install -d -m 0755 "$UDEV_DST" "$MOD_DST"
        install -m 0644 "${SCRIPT_DIR}/70-wlboard-uinput.rules" "${UDEV_DST}/70-wlboard-uinput.rules"
        install -m 0644 "${SCRIPT_DIR}/wlboard-uinput.conf" "${MOD_DST}/wlboard-uinput.conf"
        if [ -z "$DESTDIR" ]; then
            if command -v modprobe >/dev/null 2>&1; then
                modprobe uinput 2>/dev/null || true
            fi
            if [ -d /run/udev ] && command -v udevadm >/dev/null 2>&1; then
                udevadm control --reload-rules 2>/dev/null || true
                udevadm trigger --subsystem-match=misc --sysname-match=uinput 2>/dev/null || true
            fi
        fi
    elif command -v sudo >/dev/null 2>&1; then
        sudo install -d -m 0755 "$UDEV_DST" "$MOD_DST"
        sudo install -m 0644 "${SCRIPT_DIR}/70-wlboard-uinput.rules" "${UDEV_DST}/70-wlboard-uinput.rules"
        sudo install -m 0644 "${SCRIPT_DIR}/wlboard-uinput.conf" "${MOD_DST}/wlboard-uinput.conf"
        if command -v modprobe >/dev/null 2>&1; then
            sudo modprobe uinput 2>/dev/null || true
        fi
        if [ -d /run/udev ] && command -v udevadm >/dev/null 2>&1; then
            sudo udevadm control --reload-rules 2>/dev/null || true
            sudo udevadm trigger --subsystem-match=misc --sysname-match=uinput 2>/dev/null || true
        fi
    else
        echo "Warning: --with-udev requires root or sudo; skipped udev rule installation." >&2
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

echo "Installed wlboard to ${TARGET_ROOT}"
