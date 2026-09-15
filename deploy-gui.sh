#!/bin/bash
set -euo pipefail

SHARE_DIR="/usr/local/share/face-auth"

# ---- Determine actual user (handles sudo) ----
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

if [ "$USR_WRITABLE" = true ]; then
    BIN_DIR="/usr/local/bin"
    APP_DIR="/usr/share/applications"
    ICON_DIR="/usr/share/icons/hicolor/scalable/apps"
    DATA_DIR="/usr/local/share/face-auth-gtk"
else
    echo "Detected read-only /usr, using per-user paths for $ACTUAL_USER..."
    BIN_DIR="${XDG_BIN_HOME:-$ACTUAL_HOME/.local/bin}"
    APP_DIR="${XDG_DATA_HOME:-$ACTUAL_HOME/.local/share}/applications"
    ICON_DIR="${XDG_DATA_HOME:-$ACTUAL_HOME/.local/share}/icons/hicolor/scalable/apps"
    DATA_DIR="${XDG_DATA_HOME:-$ACTUAL_HOME/.local/share}/face-auth-gtk"
fi

# ---- Ensure PATH includes our bin dir ----
if ! echo "$PATH" | tr ':' '\n' | grep -qx "$BIN_DIR"; then
    echo "Note: $BIN_DIR is not in PATH."
    echo "Add this to ~/.bashrc or ~/.zshrc for $ACTUAL_USER:"
    echo "  export PATH=\"\$PATH:$BIN_DIR\""
fi

# ---- Build (non-musl, dynamic GTK) ----
#
# No container fallback here, unlike deploy.sh: this binary links dynamically
# against the host's GTK4 and libadwaita, so one built inside a container will
# not reliably load on the host.

# Root's PATH normally does not include the invoking user's rustup install.
find_cargo() {
    if command -v cargo &>/dev/null; then
        command -v cargo
        return 0
    fi
    if [ -n "${SUDO_USER:-}" ]; then
        local user_home
        user_home="$(getent passwd "$SUDO_USER" | cut -d: -f6)"
        if [ -n "$user_home" ] && [ -x "$user_home/.cargo/bin/cargo" ]; then
            echo "$user_home/.cargo/bin/cargo"
            return 0
        fi
    fi
    return 1
}

# Build as the invoking user; root-owned files in target/ break their next build.
as_user() {
    if [ -n "${SUDO_USER:-}" ] && [ "$(id -u)" -eq 0 ]; then
        sudo -u "$SUDO_USER" -H "$@"
    else
        "$@"
    fi
}

if [ -f "target/release/face-auth-gtk" ] && [ -z "${FACE_AUTH_FORCE_BUILD:-}" ]; then
    echo "Using pre-built binary from target/release/"
elif CARGO_BIN="$(find_cargo)"; then
    echo "Building face-auth-gtk with $CARGO_BIN..."
    as_user "$CARGO_BIN" build --release -p face-auth-gtk || {
        echo ""
        echo "Build failed. The GUI needs GTK4 and libadwaita development files:"
        echo "  Arch/CachyOS:  sudo pacman -S --needed gtk4 libadwaita"
        echo "  Fedora:        sudo dnf install gtk4-devel libadwaita-devel"
        echo "  Debian/Ubuntu: sudo apt install libgtk-4-dev libadwaita-1-dev"
        exit 1
    }
else
    echo "Error: no Rust toolchain found and no pre-built binary in target/release/"
    echo ""
    echo "Install one, then re-run:"
    echo "  Arch/CachyOS:  sudo pacman -S --needed rust gtk4 libadwaita"
    echo "  Fedora:        sudo dnf install rust cargo gtk4-devel libadwaita-devel"
    echo "  Or rustup:     curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    echo ""
    echo "The GUI is optional — core face unlock works without it."
    exit 1
fi

# ---- Install binary ----
echo "Installing binary to $BIN_DIR/face-auth-gtk..."
install -Dm755 target/release/face-auth-gtk "$BIN_DIR/face-auth-gtk"

# ---- Install .desktop file ----
echo "Installing desktop file to $APP_DIR/..."
install -Dm644 data/com.github.pfalkingham.face-auth-gtk.desktop "$APP_DIR/com.github.pfalkingham.face-auth-gtk.desktop"

# ---- Install icon ----
echo "Installing icon to $ICON_DIR/..."
install -Dm644 data/com.github.pfalkingham.face-auth-gtk.svg "$ICON_DIR/com.github.pfalkingham.face-auth-gtk.svg"

# ---- Ensure model directory exists ----
echo "Ensuring model directory exists..."
mkdir -p "$SHARE_DIR"

echo ""
echo "=== GUI install complete! ==="
echo ""
echo "Launch from the application menu: Face Authentication Settings"
echo "Or run: face-auth-gtk (if $BIN_DIR is in PATH)"
