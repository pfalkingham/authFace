#!/bin/bash
set -euo pipefail

MODEL_URL="https://github.com/deepinsight/insightface/releases/download/v0.7/buffalo_sc.zip"
MODEL_CHECKSUM="9cc6e4a75f0e2bf0b1aed94578f144d15175f357bdc05e815e5c4a02b319eb4f"

# Pinned to the commit that introduced the file, not to a moving branch: a
# `master` URL silently changes what gets installed. The checksum is the real
# gate; the pin keeps it from breaking on an unrelated upstream commit.
DETECTOR_URL="https://raw.githubusercontent.com/Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB/0f9ca4a9fc80170fd505168fd1132b837141f7df/models/onnx/version-slim-320.onnx"
DETECTOR_CHECKSUM="e9adbd0f920ddcce9368434c4d34d72520dc0c19b526fd44b4ef49bde2c3b1a8"

BIN_DIR="/usr/local/bin"
SHARE_DIR="/usr/local/share/face-auth"
CONFIG_DIR="/etc"
PAM_DIR="/etc/pam.d"
VAR_DIR="/var/lib/face-auth"
SELINUX_DIR="/usr/local/share/face-auth/selinux"

PAM_LINE="auth       sufficient  pam_exec.so quiet /usr/local/bin/face-auth"

if [ "$(id -u)" -ne 0 ]; then
    echo "Error: this script installs into /usr/local, /etc and /var/lib — run it with sudo."
    exit 1
fi

ACTUAL_USER="${SUDO_USER:-$USER}"

# ---- Undo any previous partial setup ----
echo "Cleaning up any previous partial setup..."

for service in sudo swaylock gdm-password; do
    if [ -f "$PAM_DIR/$service" ]; then
        sed -i '/pam_exec\.so.*face-auth/d' "$PAM_DIR/$service" 2>/dev/null || true
    fi
done

# ---- Build ----
if command -v cargo &>/dev/null; then
    echo "Building face-auth..."
    cargo build --release --target x86_64-unknown-linux-musl -p face-auth -p face-enroll
elif [ -f "target/x86_64-unknown-linux-musl/release/face-auth" ]; then
    echo "Using pre-built binaries from target/..."
else
    echo "Error: cargo not found and no pre-built binaries in target/"
    echo "Build first: distrobox enter face-auth-dev -- cargo build --release --target x86_64-unknown-linux-musl"
    exit 1
fi

# ---- Install binaries ----
echo "Installing binaries..."
install -Dm755 target/x86_64-unknown-linux-musl/release/face-auth "$BIN_DIR/face-auth"
install -Dm755 target/x86_64-unknown-linux-musl/release/face-enroll "$BIN_DIR/face-enroll"

# ---- Install models ----
# Staged in a private mktemp directory. A fixed /tmp path can be pre-created by
# another user, who then owns it and can swap the file between the checksum
# check and the install.
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

echo "Installing model..."
MODEL_NAME="w600k_mbf.onnx"
if [ -f "$SHARE_DIR/$MODEL_NAME" ]; then
    echo "Model already installed at $SHARE_DIR/$MODEL_NAME"
elif [ -f "models/$MODEL_NAME" ]; then
    install -Dm644 "models/$MODEL_NAME" "$SHARE_DIR/$MODEL_NAME"
    echo "Installed model from models/$MODEL_NAME"
else
    echo "Downloading model from InsightFace..."
    curl -fsSL -o "$WORK_DIR/buffalo_sc.zip" "$MODEL_URL"
    unzip -oq "$WORK_DIR/buffalo_sc.zip" -d "$WORK_DIR/"
    echo "Verifying checksum..."
    echo "$MODEL_CHECKSUM  $WORK_DIR/$MODEL_NAME" | sha256sum -c - || {
        echo "Error: Checksum mismatch! The model may be corrupted or tampered with."
        exit 1
    }
    install -Dm644 "$WORK_DIR/$MODEL_NAME" "$SHARE_DIR/$MODEL_NAME"
    echo "Model downloaded and installed"
fi

echo "Installing face detector model..."
DETECTOR_NAME="version-slim-320.onnx"
if [ -f "$SHARE_DIR/$DETECTOR_NAME" ]; then
    echo "Detector model already installed at $SHARE_DIR/$DETECTOR_NAME"
elif [ -f "models/$DETECTOR_NAME" ]; then
    install -Dm644 "models/$DETECTOR_NAME" "$SHARE_DIR/$DETECTOR_NAME"
    echo "Installed detector model from models/$DETECTOR_NAME"
else
    echo "Downloading face detector model..."
    curl -fsSL -o "$WORK_DIR/$DETECTOR_NAME" "$DETECTOR_URL"
    echo "Verifying checksum..."
    echo "$DETECTOR_CHECKSUM  $WORK_DIR/$DETECTOR_NAME" | sha256sum -c - || {
        echo "Error: Detector checksum mismatch! Refusing to install."
        exit 1
    }
    install -Dm644 "$WORK_DIR/$DETECTOR_NAME" "$SHARE_DIR/$DETECTOR_NAME"
    echo "Detector model downloaded and installed"
    echo "Note: if face-auth reports that this model will not load, simplify it:"
    echo "  python3 -m onnxsim $SHARE_DIR/$DETECTOR_NAME $SHARE_DIR/$DETECTOR_NAME"
fi

echo "Installing config..."
if [ -f "$CONFIG_DIR/face-auth.toml" ]; then
    echo "Keeping existing $CONFIG_DIR/face-auth.toml"
else
    install -Dm644 config/face-auth.toml.example "$CONFIG_DIR/face-auth.toml"
fi

# ---- PAM setup ----
echo "Installing PAM configs..."
for service in sudo swaylock gdm-password; do
    conf="$PAM_DIR/$service"
    if [ ! -f "$conf" ]; then
        echo "Warning: $conf not found, skipping"
        continue
    fi
    cp "$conf" "$conf.face-auth.bak"

    if [ "$service" = "gdm-password" ] && grep -q "pam_selinux_permit\.so" "$conf"; then
        # Insert after pam_selinux_permit.so (Fedora lock screen)
        sed -i "/^auth.*pam_selinux_permit\.so\$/a $PAM_LINE" "$conf"
    else
        # Insert after #%PAM-1.0, which must remain the first line
        sed -i "/^#%PAM-1\.0/a $PAM_LINE" "$conf"
    fi

    # sed silently does nothing when the anchor is absent, which would leave
    # the service unconfigured while the script still reported success.
    if grep -q "pam_exec\.so.*face-auth" "$conf"; then
        echo "Updated $conf (backup at $conf.face-auth.bak)"
    else
        echo "Warning: could not find an insertion point in $conf."
        echo "         Add this line manually, after the first line:"
        echo "           $PAM_LINE"
    fi
done

# ---- SELinux policy (for lock screen) ----
if command -v checkmodule &>/dev/null && command -v semodule_package &>/dev/null; then
    echo "Installing SELinux policy module for lock-screen camera access..."
    mkdir -p "$SELINUX_DIR"
    cp selinux/face-auth.te "$SELINUX_DIR/face_auth.te"
    checkmodule -M -m -o "$SELINUX_DIR/face_auth.mod" "$SELINUX_DIR/face_auth.te"
    semodule_package -o "$SELINUX_DIR/face_auth.pp" -m "$SELINUX_DIR/face_auth.mod"
    semodule -i "$SELINUX_DIR/face_auth.pp"
    echo "SELinux policy installed"
else
    echo "Warning: SELinux tools not found. To enable lock-screen support, install:"
    echo "  sudo dnf install policycoreutils"
    echo "Then compile and install the policy from selinux/face-auth.te"
fi

# ---- Embeddings directory ----
#
# Face templates are authentication data. Anything that can write them can
# choose whose face unlocks an account, so the store is root-owned and 0700 and
# enrolment goes through sudo/pkexec. Earlier versions made this 1777 with
# user-owned subdirectories, which let any local user create a template
# directory for an account that had not enrolled yet.
echo "Securing embeddings directory..."
install -d -o root -g root -m 0700 "$VAR_DIR"

if [ -n "$(find "$VAR_DIR" -mindepth 1 -maxdepth 1 -print -quit 2>/dev/null)" ]; then
    # Existing templates are kept; only their ownership and modes change, so
    # nobody has to enrol again after upgrading.
    #
    # This directory may have been world-writable before this release, so treat
    # what is in it as untrusted. Anything that is not a regular file or a
    # directory (symlinks especially) is removed first: `chown -R` dereferences
    # symlinks, so a planted link could otherwise redirect it at a file
    # elsewhere on the system.
    echo "Re-securing existing templates (no re-enrolment needed)..."

    STRAY="$(find "$VAR_DIR" -mindepth 1 ! -type d ! -type f -print 2>/dev/null || true)"
    if [ -n "$STRAY" ]; then
        echo "Removing unexpected entries from $VAR_DIR:"
        echo "$STRAY" | sed 's/^/  /'
        find "$VAR_DIR" -mindepth 1 ! -type d ! -type f -delete 2>/dev/null || true
    fi

    # -h so the chown applies to entries themselves, never through a link.
    find "$VAR_DIR" -mindepth 1 \( -type d -o -type f \) -exec chown -h root:root {} +
    find "$VAR_DIR" -mindepth 1 -type d -exec chmod 0700 {} +
    find "$VAR_DIR" -mindepth 1 -type f -exec chmod 0600 {} +

    echo "If this system had the old world-writable store and you want to be"
    echo "certain no one planted a template, purge and re-enrol:"
    echo "  sudo rm -rf $VAR_DIR && sudo ./deploy.sh"
fi

echo ""
echo "=== Install complete! ==="
echo ""
echo "Enrol your face (enrolment writes a root-owned store, so it needs sudo):"
echo ""
echo "  sudo face-enroll --user $ACTUAL_USER"
echo ""
echo "Then test:"
echo "  sudo -k && sudo true    # should authenticate via face"
echo "  (lock screen: Super+L, then press a key to unlock)"
echo ""
echo "To uninstall: sudo ./uninstall.sh"
