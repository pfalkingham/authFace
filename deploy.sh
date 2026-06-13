#!/bin/bash
set -euo pipefail

MODEL_URL="https://github.com/deepinsight/insightface/releases/download/v0.7/buffalo_sc.zip"
MODEL_CHECKSUM="9cc6e4a75f0e2bf0b1aed94578f144d15175f357bdc05e815e5c4a02b319eb4f"

BIN_DIR="/usr/local/bin"
SHARE_DIR="/usr/local/share/face-auth"
CONFIG_DIR="/etc"
PAM_DIR="/etc/pam.d"
VAR_DIR="/var/lib/face-auth"
SELINUX_DIR="/usr/local/share/face-auth/selinux"

ACTUAL_USER="${SUDO_USER:-$USER}"

# ---- Undo any previous partial setup ----
echo "Cleaning up any previous partial setup..."

rm -rf "$VAR_DIR"

for service in sudo swaylock gdm-password; do
    if [ -f "$PAM_DIR/$service" ]; then
        sed -i '/^auth\s\+sufficient\s\+pam_exec\.so.*face-auth/d' "$PAM_DIR/$service" 2>/dev/null || true
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

# ---- Install model ----
echo "Installing model..."
MODEL_NAME="w600k_mbf.onnx"
if [ -f "$SHARE_DIR/$MODEL_NAME" ]; then
    echo "Model already installed at $SHARE_DIR/$MODEL_NAME"
elif [ -f "models/$MODEL_NAME" ]; then
    install -Dm644 "models/$MODEL_NAME" "$SHARE_DIR/$MODEL_NAME"
    echo "Installed model from models/$MODEL_NAME"
else
    echo "Downloading model from InsightFace..."
    mkdir -p /tmp/face-auth-model
    curl -L -o /tmp/face-auth-model/buffalo_sc.zip "$MODEL_URL"
    unzip -o /tmp/face-auth-model/buffalo_sc.zip -d /tmp/face-auth-model/
    echo "Verifying checksum..."
    echo "$MODEL_CHECKSUM  /tmp/face-auth-model/$MODEL_NAME" | sha256sum -c - || {
        echo "Warning: Checksum mismatch! The model may be corrupted."
        echo "Continuing anyway..."
    }
    install -Dm644 "/tmp/face-auth-model/$MODEL_NAME" "$SHARE_DIR/$MODEL_NAME"
    rm -rf /tmp/face-auth-model
    echo "Model downloaded and installed"
fi

# ---- Install config ----
echo "Installing config..."
install -Dm644 config/face-auth.toml.example "$CONFIG_DIR/face-auth.toml"

# ---- PAM setup ----
echo "Installing PAM configs..."
for service in sudo swaylock gdm-password; do
    conf="$PAM_DIR/$service"
    if [ ! -f "$conf" ]; then
        echo "Warning: $conf not found, skipping"
        continue
    fi
    cp "$conf" "$conf.face-auth.bak"
    sed -i '/pam_exec\.so.*face-auth/d' "$conf"

    if [ "$service" = "gdm-password" ]; then
        # Insert after pam_selinux_permit.so line (lock screen)
        sed -i '/^auth.*pam_selinux_permit\.so$/a auth       sufficient  pam_exec.so /usr/local/bin/face-auth' "$conf"
    else
        # Insert after #%PAM-1.0 (must remain first line)
        sed -i '/^#%PAM-1\.0/a auth       sufficient  pam_exec.so /usr/local/bin/face-auth' "$conf"
    fi
    echo "Updated $conf (backup at $conf.face-auth.bak)"
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
echo "Creating embeddings directory..."
mkdir -p "$VAR_DIR/$ACTUAL_USER"
chmod 1777 "$VAR_DIR"
chown -R "$ACTUAL_USER:$ACTUAL_USER" "$VAR_DIR/$ACTUAL_USER"

echo ""
echo "=== Install complete! ==="
echo ""
echo "Run this command to enroll your face:"
echo ""
echo "  face-enroll --user $ACTUAL_USER"
echo ""
echo "Then test:"
echo "  sudo true           # should authenticate via face"
echo "  (lock screen: Super+L, then press a key to unlock)"
echo ""
echo "To uninstall: sudo ./uninstall.sh"
