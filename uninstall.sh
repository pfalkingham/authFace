#!/bin/bash
set -euo pipefail

PURGE=false
if [[ "${1:-}" == "--purge" ]]; then
    PURGE=true
fi

BIN_DIR="/usr/local/bin"
SHARE_DIR="/usr/local/share/face-auth"
CONFIG_DIR="/etc"
PAM_DIR="/etc/pam.d"

echo "Removing binaries..."
rm -f "$BIN_DIR/face-auth"
rm -f "$BIN_DIR/face-enroll"

echo "Removing model and SELinux policy..."
rm -rf "$SHARE_DIR"

echo "Removing config..."
rm -f "$CONFIG_DIR/face-auth.toml"

echo "Restoring PAM configs..."
for service in sudo swaylock gdm-password; do
    conf="$PAM_DIR/$service"
    if [ -f "$conf.face-auth.bak" ]; then
        mv "$conf.face-auth.bak" "$conf"
        echo "Restored $conf from backup"
    else
        sed -i '/face-auth/d' "$conf" 2>/dev/null || true
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
