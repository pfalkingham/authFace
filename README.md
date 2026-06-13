# authFace

IR camera facial authentication for Linux. Inspired by Howdy, but built to work on immutable distros (Bluefin, Silverblue, etc.) because it's just a static binary + PAM config — no packages, no daemons, no layering.

## Features

- **Windows Hello–style IR camera auth** for sudo and GNOME lock screen
- **Password fallback** — never get locked out
- **Static musl binary** (~20 MB, zero runtime dependencies)
- **No daemon, no systemd, no D-Bus** — just `pam_exec.so`
- **Configurable** via `/etc/face-auth.toml`, `~/.config/face-auth.toml`, or env vars
- **Built-in capture timeout** (5s default) — camera hang won't lock you out

## Quick Start

```bash
# 1. Install everything
sudo ./deploy.sh

# 2. Enroll your face
face-enroll --user $USER

# 3. Test sudo
sudo true              # triggers IR camera → exit 0

# 4. Test lock screen
# Super+L, then press any key — camera fires, unlocks automatically
```

## Requirements

### Hardware

- **IR camera** exposing GREY format (Windows Hello compatible, e.g. Shinetech ASUS FHD webcam)
- **Linux kernel** with `uvcvideo` (standard on all distros)

### Software (target system — where you deploy)

- PAM with `pam_exec.so` (standard on all distros)
- SELinux (Fedora/Bluefin/Silverblue) — deploy script installs policy automatically
- `policycoreutils` for SELinux policy compilation (installed by default on Fedora)

### Software (build system — where you compile)

You need a Rust toolchain with the `x86_64-unknown-linux-musl` target.

## Building from Source

### On any Linux (direct)

```bash
# Install Rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add musl target
rustup target add x86_64-unknown-linux-musl

# Clone and build
git clone https://github.com/pfalkingham/authFace.git
cd authFace
cargo build --release --target x86_64-unknown-linux-musl -p face-auth -p face-enroll

# Deploy
sudo ./deploy.sh
```

### On immutable distros via distrobox

```bash
# Create a Fedora development container
distrobox create --image docker.io/library/fedora:40 --name authface-dev
distrobox enter authface-dev

# Inside the container, install build deps (once)
sudo dnf install -y rust cargo gcc gcc-c++ musl-gcc cmake

# Clone and build
cd ~/Projects
git clone https://github.com/pfalkingham/authFace.git
cd authFace
cargo build --release --target x86_64-unknown-linux-musl -p face-auth -p face-enroll

# Exit container, then deploy on host
exit
sudo ./deploy.sh
```

## Deployment

The `deploy.sh` script must be run as root:

```bash
sudo ./deploy.sh
```

It does the following:

| Step | What | Details |
|------|------|---------|
| Build | Compiles if `cargo` is available | Falls back to pre-built binaries in `target/` |
| Binaries | Installs to `/usr/local/bin` | `face-auth` + `face-enroll` |
| Model | Downloads from InsightFace | `w600k_mbf.onnx` (~13 MB) to `/usr/local/share/face-auth/` |
| Config | Installs default config | `/etc/face-auth.toml` |
| PAM | Patches PAM service files | Adds `sufficient` `pam_exec.so` to `sudo`, `gdm-password`, `swaylock` |
| SELinux | Compiles and loads policy | Allows `xdm_t` to mmap camera for lock-screen auth |
| Storage | Creates embeddings directory | `/var/lib/face-auth/<user>/` with sticky bit |

Each PAM file is backed up with a `.face-auth.bak` suffix.

### Uninstall

```bash
sudo ./uninstall.sh
```

Restores PAM backups, removes binaries, model, config, and SELinux policy. Use `--purge` to also remove embeddings.

## Configuration

Priority (highest first):

1. **Environment variables**: `FACE_AUTH_DEVICE`, `FACE_AUTH_THRESHOLD`, `FACE_AUTH_MODEL_PATH`, `FACE_AUTH_EMBEDDINGS_DIR`, `FACE_AUTH_CAPTURE_TIMEOUT`
2. **User config**: `~/.config/face-auth.toml`
3. **System config**: `/etc/face-auth.toml`
4. **Defaults**: auto-detected camera, threshold 0.6, 5s capture timeout

Example `/etc/face-auth.toml`:
```toml
device = "/dev/video3"
threshold = 0.6
model_path = "/usr/local/share/face-auth/w600k_mbf.onnx"
embeddings_dir = "/var/lib/face-auth"
capture_timeout_ms = 5000
```

## Enrollment

```bash
face-enroll --user $USER
```

Options: `--frames`, `--interval`, `--device`, `--threshold`, `--model`, `-v`.

## PAM Integration

The deploy script adds a `sufficient` `pam_exec.so` line to:

| Service | File | Insertion point |
|---------|------|----------------|
| `sudo` | `/etc/pam.d/sudo` | After `#%PAM-1.0` |
| `gdm-password` | `/etc/pam.d/gdm-password` | After `pam_selinux_permit.so` |
| `swaylock` | `/etc/pam.d/swaylock` | After `#%PAM-1.0` |

`sufficient` means: if face-auth exits 0, the user is authenticated immediately.
If it fails (no match, no camera, timeout), PAM falls through to password prompt.

No `timeout`, `setenv`, or `env_pass` flags are needed — face-auth reads the camera
(not stdin) and resolves `PAM_USER` via its own fallback chain.

## How It Works

```
PAM (sudo / gdm-password / swaylock)
  │
  ▼
face-auth (static binary)
  ├─ V4L2 capture from IR camera (640×400 GREY, /dev/video3)
  │   └─ poll() with 5s timeout — exits cleanly if camera hangs
  ├─ Histogram equalization
  ├─ Resize to 112×112, normalize to [-1, 1]
  ├─ tract-onnx inference (MobileFaceNet, 512-d embedding)
  ├─ Cosine similarity vs stored embeddings (default threshold 0.6)
  └─ Exit 0 (match) or exit 1 (no match → password prompt)
```

### Model

Uses InsightFace **`w600k_mbf.onnx`** (MobileFaceNet @ WebFace600K, ~13 MB, 512-d output)
from the `buffalo_sc` model pack. Licensed under MIT (InsightFace is MIT-licensed).

The model is **not bundled** in this repository. `deploy.sh` downloads it directly from
InsightFace's official GitHub releases and verifies the SHA-256 checksum. You can also
download it manually:

```bash
sudo mkdir -p /usr/local/share/face-auth
curl -Lo /tmp/buffalo_sc.zip \
  https://github.com/deepinsight/insightface/releases/download/v0.7/buffalo_sc.zip
unzip -p /tmp/buffalo_sc.zip w600k_mbf.onnx | \
  sudo tee /usr/local/share/face-auth/w600k_mbf.onnx > /dev/null
```

## SELinux

On Fedora/Bluefin/Silverblue with SELinux enforcing, the GNOME lock screen runs in the
`xdm_t` domain. This domain cannot `mmap` video devices by default. The deploy script
installs a minimal policy module:

```
allow xdm_t v4l_device_t:chr_file map;
```

To remove: `sudo semodule -r face_auth`

If the deploy script reported missing SELinux tools:
```bash
sudo dnf install -y policycoreutils
sudo checkmodule -M -m -o face_auth.mod selinux/face-auth.te
sudo semodule_package -o face_auth.pp -m face_auth.mod
sudo semodule -i face_auth.pp
```

## Troubleshooting

```bash
# Find your IR camera
ls /sys/class/video4linux/*/name

# Grant video group access (log out/in after)
sudo usermod -aG video $USER

# Debug output
RUST_LOG=face_auth_core=debug sudo -k && sudo true

# Check PAM logs
journalctl | grep -i "pam_exec\|face-auth"

# SELinux denials
journalctl -k | grep face-auth | grep denied

# Test binary directly (skips PAM)
sudo env PAM_USER=$USER USER=$USER HOME=$HOME /usr/local/bin/face-auth
echo $?   # 0 = success, 1 = failure

# Increase capture timeout (default 5000ms)
FACE_AUTH_CAPTURE_TIMEOUT=10000 sudo -k && sudo true
```

## Security & Limitations

- **IR-only, no liveness detection:** Uses IR camera (not RGB), which resists casual
  photo spoofing. Does not perform structured-light or dot-projection depth checks.
  High-quality IR-transparent prints or 3D masks may bypass verification.
- **SELinux policy scope:** The lock-screen policy grants `xdm_t` mmap access to all
  V4L2 devices. This is a trade-off for drop-in compatibility; narrowing it requires
  custom udev device types.
- **x86_64 only:** V4L2 ioctl numbers and struct layouts are hardcoded for x86_64.
  ARM/aarch64 requires switching to the `v4l` crate.
- **Model integrity:** `deploy.sh` verifies SHA-256 checksum and aborts on mismatch.

## Project Structure

```
authFace/
  crates/
    face-auth-core/        # Core library
      src/
        capture.rs         # V4L2 capture + poll() timeout
        config.rs          # Layered config (system -> user -> env)
        error.rs           # Error types
        inference.rs       # tract-onnx model loading + encoding
        lib.rs             # FaceAuth struct, auth + enroll
        preprocess.rs      # Histogram equalize, resize, normalize
        storage.rs         # Binary embedding I/O (versioned, atomic)
        verify.rs          # Cosine similarity
    face-auth/             # PAM binary (stdin-less, PAM_USER fallback)
    face-enroll/           # Enrollment CLI
  config/
    face-auth.toml.example # Documented config template
  pam/                     # PAM stanza templates
  selinux/
    face-auth.te           # SELinux policy source
  deploy.sh                # Installation script
  uninstall.sh             # Removal script
```

## TODO

A UI for setup would probably be good.

## License

MIT
