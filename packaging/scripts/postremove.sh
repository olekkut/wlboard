#!/bin/sh
# Cross-distro post-remove scriptlet for wlboard (.deb, .rpm, .pkg.tar.zst).
# Refreshes desktop, icon, and udev caches safely without failing in containers or chroots.

if [ -d /run/udev ] && command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules 2>/dev/null || true
fi

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications 2>/dev/null || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor 2>/dev/null || true
fi

true
