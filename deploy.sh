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

ACTUAL_USER="${SUDO_USER:-${USER:-$(id -un)}}"

# ---- Undo any previous partial setup ----
echo "Cleaning up any previous partial setup..."

for service in sudo swaylock gdm-password; do
    if [ -f "$PAM_DIR/$service" ]; then
        sed -i '/pam_exec\.so.*face-auth/d' "$PAM_DIR/$service" 2>/dev/null || true
    fi
done

# ---- Build ----
MUSL_TARGET="x86_64-unknown-linux-musl"
ARTIFACT_DIR="target/$MUSL_TARGET/release"

# Locate cargo. This script runs under sudo, and root's PATH normally does not
# include the invoking user's rustup installation, so look there as well.
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

# Build as the invoking user, never as root: cargo fetches crates and runs
# build scripts, and root-owned files left in target/ break their next build.
as_user() {
    if [ -n "${SUDO_USER:-}" ] && [ "$(id -u)" -eq 0 ]; then
        sudo -u "$SUDO_USER" -H "$@"
    else
        "$@"
    fi
}

build_with_cargo() {
    local cargo="$1"
    if ! as_user "$cargo" build --release --target "$MUSL_TARGET" \
            -p face-auth -p face-enroll; then
        echo ""
        echo "Build failed. If the error mentions a missing target, add it with:"
        echo "  rustup target add $MUSL_TARGET"
        return 1
    fi
}

if [ -f "$ARTIFACT_DIR/face-auth" ] && [ -f "$ARTIFACT_DIR/face-enroll" ] \
   && [ -z "${FACE_AUTH_FORCE_BUILD:-}" ]; then
    echo "Using pre-built binaries from $ARTIFACT_DIR/"
elif CARGO_BIN="$(find_cargo)"; then
    echo "Building face-auth with $CARGO_BIN..."
    build_with_cargo "$CARGO_BIN" || exit 1
else
    CONTAINER_ENGINE=""
    for engine in podman docker; do
        command -v "$engine" &>/dev/null && { CONTAINER_ENGINE="$engine"; break; }
    done

    echo "Error: no Rust toolchain found, and no pre-built binaries in $ARTIFACT_DIR/."
    echo ""

    if [ -n "$CONTAINER_ENGINE" ]; then
        # Deliberately not run from here: this script is under sudo, and
        # rootless $CONTAINER_ENGINE driven through `sudo -u` frequently fails
        # on a missing XDG_RUNTIME_DIR. Running it directly is reliable, and
        # keeps the build artifacts owned by you.
        echo "Option 1 — build in a container, no toolchain needed."
        echo "Run this as yourself (NOT with sudo), then re-run sudo ./deploy.sh:"
        echo ""
        echo "  $CONTAINER_ENGINE run --rm -v \"\$PWD\":/src:Z -w /src \\"
        echo "    docker.io/library/rust:alpine \\"
        echo "    sh -c 'apk add --no-cache musl-dev && \\"
        echo "           cargo build --release --target $MUSL_TARGET \\"
        echo "             -p face-auth -p face-enroll'"
        echo ""
        echo "Option 2 — install a Rust toolchain:"
    else
        echo "Install a Rust toolchain:"
    fi

    echo "  Arch/CachyOS:  sudo pacman -S --needed rust"
    echo "  Fedora:        sudo dnf install rust cargo"
    echo "  Or rustup:     curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    echo "  Then:          rustup target add $MUSL_TARGET"
    echo ""
    echo "Either way, re-run sudo ./deploy.sh afterwards."
    exit 1
fi

for bin in face-auth face-enroll; do
    if [ ! -f "$ARTIFACT_DIR/$bin" ]; then
        echo "Error: expected $ARTIFACT_DIR/$bin after the build, but it is missing."
        exit 1
    fi
done

# ---- Install binaries ----
echo "Installing binaries..."
install -Dm755 "$ARTIFACT_DIR/face-auth" "$BIN_DIR/face-auth"
install -Dm755 "$ARTIFACT_DIR/face-enroll" "$BIN_DIR/face-enroll"

# ---- Install models ----
# Staged in a private mktemp directory. A fixed /tmp path can be pre-created by
# another user, who then owns it and can swap the file between the checksum
# check and the install.
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

# GitHub release downloads redirect to a CDN that intermittently resets the
# connection mid-handshake ("TLS connect error: unexpected eof while reading").
# Retry rather than abandoning a half-finished install; --retry-all-errors so
# a reset connection counts, not just a retryable HTTP status.
# verify <file> <expected-sha256> — applied to every model, however it arrived.
# A file staged in models/ is no more trusted than one off the network.
verify() {
    local file="$1" want="$2"
    if ! echo "$want  $file" | sha256sum -c --status -; then
        echo "Error: checksum mismatch for $file"
        echo "  expected: $want"
        echo "  actual:   $(sha256sum "$file" | cut -d' ' -f1)"
        echo "Refusing to install a model that is not the one this release pins."
        return 1
    fi
    echo "  checksum OK: $(basename "$file")"
}

# fetch <url> <dest> <manual-recovery-hint>
fetch() {
    local url="$1" dest="$2" hint="$3"
    curl -fsSL --retry 5 --retry-delay 2 --retry-all-errors \
         --connect-timeout 20 -o "$dest" "$url" && return 0

    echo ""
    echo "Download failed after retries: $url"
    echo ""
    echo "This script prefers a local copy over downloading, so you can place the"
    echo "file yourself and re-run:"
    echo ""
    echo "$hint"
    return 1
}

echo "Installing model..."
MODEL_NAME="w600k_mbf.onnx"
if [ -f "$SHARE_DIR/$MODEL_NAME" ]; then
    echo "Model already installed at $SHARE_DIR/$MODEL_NAME"
elif [ -f "models/$MODEL_NAME" ]; then
    verify "models/$MODEL_NAME" "$MODEL_CHECKSUM" || exit 1
    install -Dm644 "models/$MODEL_NAME" "$SHARE_DIR/$MODEL_NAME"
    echo "Installed model from models/$MODEL_NAME"
else
    echo "Downloading model from InsightFace..."
    fetch "$MODEL_URL" "$WORK_DIR/buffalo_sc.zip" \
        "  mkdir -p models
  curl -fL -o /tmp/buffalo_sc.zip '$MODEL_URL'
  unzip -j /tmp/buffalo_sc.zip $MODEL_NAME -d models/" || exit 1
    unzip -oq "$WORK_DIR/buffalo_sc.zip" -d "$WORK_DIR/"
    verify "$WORK_DIR/$MODEL_NAME" "$MODEL_CHECKSUM" || exit 1
    install -Dm644 "$WORK_DIR/$MODEL_NAME" "$SHARE_DIR/$MODEL_NAME"
    echo "Model downloaded and installed"
fi

echo "Installing face detector model..."
DETECTOR_NAME="version-slim-320.onnx"
if [ -f "$SHARE_DIR/$DETECTOR_NAME" ]; then
    echo "Detector model already installed at $SHARE_DIR/$DETECTOR_NAME"
elif [ -f "models/$DETECTOR_NAME" ]; then
    verify "models/$DETECTOR_NAME" "$DETECTOR_CHECKSUM" || exit 1
    install -Dm644 "models/$DETECTOR_NAME" "$SHARE_DIR/$DETECTOR_NAME"
    echo "Installed detector model from models/$DETECTOR_NAME"
else
    echo "Downloading face detector model..."
    fetch "$DETECTOR_URL" "$WORK_DIR/$DETECTOR_NAME" \
        "  mkdir -p models
  curl -fL -o models/$DETECTOR_NAME '$DETECTOR_URL'" || exit 1
    verify "$WORK_DIR/$DETECTOR_NAME" "$DETECTOR_CHECKSUM" || exit 1
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
