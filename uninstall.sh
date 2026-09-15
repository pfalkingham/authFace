#!/bin/bash
set -euo pipefail

PURGE=false
GUI_ONLY=false

for arg in "$@"; do
    case "$arg" in
        --purge) PURGE=true ;;
        --gui)   GUI_ONLY=true ;;
    esac
done

# ---- Detect actual user (handles sudo) ----
if [ -n "${SUDO_USER:-}" ]; then
    ACTUAL_USER="$SUDO_USER"
    ACTUAL_HOME=$(getent passwd "$SUDO_USER" | cut -d: -f6)
else
    ACTUAL_USER="${USER:-$(id -un)}"
    ACTUAL_HOME="${HOME:-$(getent passwd "$ACTUAL_USER" | cut -d: -f6)}"
fi

# ---- Check if /usr is writable (immutable FS detection) ----
USR_WRITABLE=false
if touch /usr/share/.face-auth-write-test 2>/dev/null; then
    rm -f /usr/share/.face-auth-write-test
    USR_WRITABLE=true
fi

BIN_DIR="/usr/local/bin"
SHARE_DIR="/usr/local/share/face-auth"
CONFIG_DIR="/etc"
PAM_DIR="/etc/pam.d"

if [ "$USR_WRITABLE" = true ]; then
    ICON_DIR="/usr/share/icons/hicolor/scalable/apps"
    APP_DIR="/usr/share/applications"
    GUI_DATA_DIR="/usr/local/share/face-auth-gtk"
else
    ICON_DIR="${XDG_DATA_HOME:-$ACTUAL_HOME/.local/share}/icons/hicolor/scalable/apps"
    APP_DIR="${XDG_DATA_HOME:-$ACTUAL_HOME/.local/share}/applications"
    GUI_DATA_DIR="${XDG_DATA_HOME:-$ACTUAL_HOME/.local/share}/face-auth-gtk"
fi

if [ "$GUI_ONLY" = true ]; then
    echo "Removing GUI binary..."
    rm -f "$BIN_DIR/face-auth-gtk"
    rm -f "${XDG_BIN_HOME:-$ACTUAL_HOME/.local/bin}/face-auth-gtk"

    echo "Removing desktop file..."
    rm -f "$APP_DIR/com.github.pfalkingham.face-auth-gtk.desktop"

    echo "Removing icon..."
    rm -f "$ICON_DIR/com.github.pfalkingham.face-auth-gtk.svg"

    echo "Removing GUI data..."
    rm -rf "$GUI_DATA_DIR"

    echo ""
    echo "GUI uninstall complete!"
    exit 0
fi

echo "Removing binaries..."
rm -f "$BIN_DIR/face-auth"
rm -f "$BIN_DIR/face-enroll"
rm -f "$BIN_DIR/face-auth-gtk"

echo "Removing model and SELinux policy..."
rm -rf "$SHARE_DIR"

echo "Removing config..."
rm -f "$CONFIG_DIR/face-auth.toml"

echo "Removing desktop entry and icon..."
rm -f "$APP_DIR/com.github.pfalkingham.face-auth-gtk.desktop"
rm -f "${XDG_DATA_HOME:-$ACTUAL_HOME/.local/share}/applications/com.github.pfalkingham.face-auth-gtk.desktop"
rm -f "$ICON_DIR/com.github.pfalkingham.face-auth-gtk.svg"
rm -f "${XDG_DATA_HOME:-$ACTUAL_HOME/.local/share}/icons/hicolor/scalable/apps/com.github.pfalkingham.face-auth-gtk.svg"
rm -rf "$GUI_DATA_DIR"
rm -rf "${XDG_DATA_HOME:-$ACTUAL_HOME/.local/share}/face-auth-gtk"

echo "Restoring PAM configs..."
for service in sudo swaylock gdm-password; do
    conf="$PAM_DIR/$service"
    if [ -f "$conf.face-auth.bak" ]; then
        mv "$conf.face-auth.bak" "$conf"
        echo "Restored $conf from backup"
    else
        # Anchored to our own stanza: a bare /face-auth/d would delete any
        # unrelated line that happens to mention it.
        sed -i '/pam_exec\.so.*face-auth/d' "$conf" 2>/dev/null || true
        echo "Cleaned $conf"
    fi
done

echo "Removing SELinux policy module..."
semodule -r face_auth 2>/dev/null || true

echo ""
if [ "$PURGE" = true ]; then
    echo "Removing user embeddings..."
    rm -rf /var/lib/face-auth/
    echo "Uninstall complete (including user embeddings)."
else
    echo "Uninstall complete!"
    echo "Note: User embeddings in /var/lib/face-auth/ were preserved."
    echo "Remove them manually with: sudo rm -rf /var/lib/face-auth/"
    echo "Or run again with --purge to remove them automatically."
fi
