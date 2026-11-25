#!/usr/bin/env bash
set -euo pipefail

# Configure kernel uinput permissions for wlboard on KDE Plasma Wayland
# (Hyprland and Sway use Wayland's native zwp_virtual_keyboard_v1 automatically).

RULE_DST="/etc/udev/rules.d/70-wlboard-uinput.rules"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "Installing udev rule to ${RULE_DST}..."
sudo install -Dm644 "${SCRIPT_DIR}/70-wlboard-uinput.rules" "${RULE_DST}"

echo "Ensuring uinput kernel module loads on boot..."
echo "uinput" | sudo tee /etc/modules-load.d/wlboard-uinput.conf >/dev/null
sudo modprobe uinput || true

echo "Reloading udev rules..."
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=misc --sysname-match=uinput || true

echo "Done! /dev/uinput is ready for wlboard."
