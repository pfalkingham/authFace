# authFace — IR Camera Face Unlock for Linux

<p align="center">
  <img src="data/com.github.pfalkingham.face-auth-gtk.svg" width="128" height="128" alt="authFace logo">
</p>

**Windows Hello–style biometric login for Linux.** IR camera facial authentication via PAM — works on **immutable distros** (Bazzite, Bluefin, Fedora Silverblue, Fedora Kinoite, etc.) with zero system packages, daemons, or layering.

- **Face unlock for sudo, lock screen (GNOME/Sway), and `gdm-password`**
- **~2 seconds** from camera poll to authenticated
- **Static musl binary** — no dependencies, no runtime
- **No daemon, no systemd units, no D-Bus**
- **GUI settings panel** (optional GTK4 app) for camera selection and enrollment
- **Immutable-first** — everything fits in `/usr/local` and `~/.local`, no `/usr` modifications needed

## Features

- **Windows Hello–compatible IR camera support** — raw GREY format, no RGB camera needed
- **Automatic password fallback** — if face auth fails, times out, or no camera, PAM falls through to password
- **Static musl binary** (~20 MB, zero runtime dependencies) — copy to any Linux system
- **No daemon, no systemd, no D-Bus** — just `pam_exec.so` triggered by PAM
- **Configurable** via `/etc/face-auth.toml`, `~/.config/face-auth.toml`, or environment variables
- **Built-in capture timeout** (5s default) — camera hang won't lock you out
- **GTK4 settings GUI** — select IR camera, adjust threshold, preview live feed, enroll, improve matching, and test face recognition
- **Works on immutable distros** — no `rpm-ostree layer`, no package installs, no `/usr` modification

## Quick Start

```bash
# 1. Install core authentication (PAM, models, binaries)
sudo ./deploy.sh

# 2. Enroll your face
face-enroll --user $USER

# 3. Test sudo
sudo true              # triggers IR camera → exit 0

# 4. (Optional) Install the settings GUI
sudo ./deploy-gui.sh

# 5. Launch the GUI from app menu: "Face Authentication Settings"
#    or run: face-auth-gtk
```

## GUI — Face Authentication Settings

A native GTK4/libadwaita settings panel for configuring and testing face unlock:

| Feature | Description |
|---------|-------------|
| **Live IR preview** | Real-time camera feed with face-detection overlay |
| **Camera picker** | Dropdown to select between IR cameras |
| **Threshold slider** | Adjust similarity threshold (0.1–0.95) — higher = stricter match |
| **Enroll** | Captures 5 frames and stores face embeddings (replaces existing) |
| **Improve Matching** | Captures 5 more frames and appends to existing embeddings |
| **Test** | Captures a single frame and compares against enrolled embeddings |
| **Automatic config save** | Camera and threshold changes persist to `~/.config/face-auth.toml` |

The GUI is optional and deployed separately (no GTK dependencies bundled with the core auth binary).

## Requirements

### Hardware

- **IR camera** exposing raw GREY format (Windows Hello compatible, e.g. Shinetech ASUS FHD webcam)
- **Linux kernel** with `uvcvideo` (standard on all distros)

### Software (target system — where you deploy)

- PAM with `pam_exec.so` (standard on all distros)
- SELinux (Fedora/Bluefin/Silverblue) — deploy script installs policy automatically
- `policycoreutils` for SELinux policy compilation (installed by default on Fedora)
- For the GUI: GTK4 + libadwaita runtime libraries (system-installed, not bundled)

### Software (build system — where you compile)

You need a Rust toolchain. For the core auth (musl), add `x86_64-unknown-linux-musl` target.
For the GUI (dynamic GTK), the host target is sufficient.

## Building from Source

### Core auth (static musl — no runtime deps)

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

### GUI (dynamic GTK — needs GTK4 + libadwaita devel packages)

```bash
# Install GTK development libraries (Fedora)
sudo dnf install gtk4-devel libadwaita-devel

# Build
cargo build --release -p face-auth-gtk

# Deploy
sudo ./deploy-gui.sh
```

### On immutable distros via distrobox

```bash
# Create a Fedora development container
distrobox create --image docker.io/library/fedora:40 --name authface-dev
distrobox enter authface-dev

# Inside the container, install build deps (once)
sudo dnf install -y rust cargo gcc gcc-c++ musl-gcc cmake gtk4-devel libadwaita-devel

# Clone and build
cd ~/Projects
git clone https://github.com/pfalkingham/authFace.git
cd authFace
cargo build --release --target x86_64-unknown-linux-musl -p face-auth -p face-enroll
cargo build --release -p face-auth-gtk

# Exit container, then deploy on host
exit
sudo ./deploy.sh
sudo ./deploy-gui.sh
```

## Deployment

### Core (PAM authentication)

```bash
sudo ./deploy.sh
```

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

#### Choosing a recognition model

By default `deploy.sh` installs `w600k_mbf.onnx` (MobileFaceNet, ~13 MB) — fast and low-overhead.
For stronger/more discriminative embeddings at the cost of a larger download, pass `--model=r50`
to use `w600k_r50.onnx` (ResNet50, ~166 MB) instead, from InsightFace's `buffalo_l` pack:

```bash
sudo ./deploy.sh --model=r50
```

Both models share the same interface (112×112×3 input, 512-d normalized embedding output), so no
code changes are needed to switch. Embeddings are **not** portable between models — re-run
`face-enroll` after switching.

### GUI (optional settings panel)

```bash
sudo ./deploy-gui.sh
```

Automatically detects whether `/usr` is writable:
- **Mutable systems**: installs to `/usr/local/bin`, `/usr/share/applications/`, `/usr/share/icons/`
- **Immutable systems**: installs to `~/.local/bin`, `~/.local/share/applications/`, `~/.local/share/icons/`

Launch from the application menu: **Face Authentication Settings**, or run `face-auth-gtk`.

### Uninstall

```bash
# Remove everything (core + GUI + models + config)
sudo ./uninstall.sh

# Remove only the optional GUI
sudo ./uninstall.sh --gui

# Remove everything including face embeddings
sudo ./uninstall.sh --purge
```

Restores PAM backups, removes binaries, models, config, SELinux policy, desktop entries, and icons.

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


The GUI automatically writes camera and threshold changes to `~/.config/face-auth.toml`.

## Enrollment

```bash
# Replace existing embeddings with new capture
face-enroll --user $USER

# Append new embeddings to improve recognition across lighting/angles
face-enroll --improve --user $USER
```

CLI options: `--frames`, `--interval`, `--device`, `--threshold`, `--model`, `--improve`, `-v`.

The GUI's **Enroll Face** button replaces embeddings; **Improve Matching** appends to them.

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
  ├─ Face detection (RetinaFace-derived ONNX model)
  ├─ Resize to 112×112, normalize to [-1, 1]
  ├─ tract-onnx inference (MobileFaceNet, 512-d embedding)
  ├─ Cosine similarity vs stored embeddings (default threshold 0.6)
  └─ Exit 0 (match) or exit 1 (no match → password prompt)
```

## Model

Uses InsightFace **`w600k_mbf.onnx`** (MobileFaceNet @ WebFace600K, ~13 MB, 512-d output)
from the `buffalo_sc` model pack, plus **`version-slim-320.onnx`** for face detection.
Licensed under MIT (InsightFace is MIT-licensed).

The models are **not bundled** in this repository. `deploy.sh` downloads them directly from
InsightFace's official GitHub releases and verifies the SHA-256 checksum.

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
# List available IR cameras
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

# GUI not launching from app menu?
face-auth-gtk    # run from terminal to see errors
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
    face-auth-core/          # Core library
      src/
        capture.rs           # V4L2 capture + poll() timeout
        config.rs            # Layered config (system → user → env)
        detector.rs          # Face detection (RetinaFace-based ONNX model)
        error.rs             # Error types
        inference.rs         # tract-onnx model loading + encoding
        lib.rs               # FaceAuth struct, auth + enroll + scan
        preprocess.rs        # Histogram equalize, resize, normalize
        storage.rs           # Binary embedding I/O (versioned, atomic)
        verify.rs            # Cosine similarity
    face-auth/               # PAM binary (stdin-less, PAM_USER fallback)
    face-enroll/             # Enrollment CLI
    face-auth-gtk/           # GTK4 settings GUI
  config/
    face-auth.toml.example   # Documented config template
  data/
    desktop file + icon      # App launcher assets
  selinux/
    face-auth.te             # SELinux policy source
  deploy.sh                  # Core auth installer
  deploy-gui.sh              # Optional GUI installer
  uninstall.sh               # Removal script (--gui, --purge flags)
```

## License

MIT
