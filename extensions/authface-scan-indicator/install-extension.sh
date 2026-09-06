#!/usr/bin/env bash
#
# Install the authFace scan indicator GNOME Shell extension into ~/.local
# (works on immutable distros — no /usr modification needed).
#
set -euo pipefail

UUID="authface-scan-indicator@samvivan.local"
SRC="$(cd "$(dirname "$0")" && pwd)"
EXT_DIR="${XDG_DATA_HOME:-$HOME/.local}/share/gnome-shell/extensions"
DEST="$EXT_DIR/$UUID"

mkdir -p "$EXT_DIR"
rm -rf "$DEST"
mkdir -p "$DEST"
cp "$SRC/extension.js" "$SRC/metadata.json" "$DEST/"

echo "Installed to $DEST"

if command -v gnome-extensions >/dev/null 2>&1 && [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
    echo "Warning: no DBUS_SESSION_BUS_ADDRESS — running outside a graphical session."
    echo "Enable manually inside a session with:  gnome-extensions enable $UUID"
elif command -v gnome-extensions >/dev/null 2>&1; then
    gnome-extensions enable "$UUID" 2>/dev/null \
        && echo "Extension enabled: $UUID" \
        || echo "Please enable manually: gnome-extensions enable $UUID"
fi

cat <<'NOTES'

Done. To apply:
  - Log out and back in, OR
  - Restart GNOME Shell:  Alt+F2  →  r  (X11), or log out/in (Wayland)

Prerequisites:
  - GNOME Shell 45+ (Fedora 39+, Bazzite, Bluefin, Silverblue, Kinoite)
  - A running user session (the lock screen runs inside YOUR session —
    this does NOT show on the GDM login/greeter screen).
NOTES