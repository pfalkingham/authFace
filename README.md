# authFace — IR Camera Face Unlock for Linux

<p align="center">
  <img src="data/com.github.pfalkingham.face-auth-gtk.svg" width="128" height="128" alt="authFace logo">
</p>

**Windows Hello–style biometric login for Linux.** IR camera facial authentication via PAM — works on **immutable distros** (Bazzite, Bluefin, Fedora Silverblue, Fedora Kinoite, etc.) with zero system packages, daemons, or layering.

> [!NOTE]
> This repository is a **personal fork** of [pfalkingham/authFace](https://github.com/pfalkingham/authFace). All credit for the original design and implementation goes to the upstream author. This fork adds a few personal-quality-of-life fixes documented below. See [License](#license).

- **Face unlock for sudo, lock screen (GNOME/Sway), and `gdm-password`**
- **~2 seconds** from camera poll to authenticated
- **Static musl binary** — no dependencies, no runtime
- **No daemon, no systemd units, no D-Bus**
- **GUI settings panel** (optional GTK4 app) for camera selection and enrollment
- **Immutable-first** — everything fits in `/usr/local` and `~/.local`, no `/usr` modifications needed

## Upstream Merges & Security Pass

Improvements merged from [SamVivan1/authFace](https://github.com/SamVivan1/authFace):

| Change | Before | Now |
|--------|--------|-----|
| **Multi-IR-camera support** | Returned the *first* IR-named device in `/sys/class/video4linux` | Collects all IR candidates and uses the first that **actually opens** as a GREY capture device |
| **Distro-aware PAM** | Only Fedora (`pam_selinux_permit.so` insertion point) | Also handles Ubuntu/Debian `gdm-password` (`#%PAM-1.0`) |
| **PAM `quiet` flag** | no `quiet` | `pam_exec.so quiet` suppresses `pam_exec` chatter |
| **Detector model download** | Required `models/version-slim-320.onnx` to be present | `deploy.sh` fetches it (now pinned to a commit and SHA-256 verified) |
| **Lock screen scan indicator** | None (silent scan) | `face-auth` writes a status file; a GNOME Shell extension renders scanning/ok/fail |

Followed by a security pass over the whole tree — see [CHANGELOG.md](CHANGELOG.md)
for the full list. The changes that affect how you use it:

- **Enrolment needs root** (`sudo face-enroll`, or one click in the GUI via
  `pkexec`). Face templates are authentication data; when the store was
  world-writable, any local user could enrol a face for an account that had not
  enrolled yet, then log in as it.
- **The login prompt trusts only `/etc/face-auth.toml`.** Your own config can
  make matching stricter, never looser. See [Configuration](#configuration).
- **`face-auth` requires `PAM_USER`** rather than falling back to `USER`,
  `LOGNAME` or `id -un`, and refuses remote (`PAM_RHOST`) sessions.

Upgrading re-secures an existing template store in place, so **no re-enrolment
is needed**.

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

# 2. Enroll your face (templates are root-owned, so this needs sudo)
sudo face-enroll --user $USER

# 3. Test sudo
sudo -k && sudo true   # triggers IR camera → exit 0

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
| **Camera picker** | Dropdown to select between IR cameras (pre-filtered to openable devices) |
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

> **Multiple IR cameras?** The fork's auto-detection finds *all* IR devices and picks the first one that opens. If you have more than one, set `device` explicitly in config to pin a specific one (see [Configuration](#configuration)).

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
# Install GTK development libraries
sudo pacman -S --needed gtk4 libadwaita        # Arch / CachyOS
sudo dnf install gtk4-devel libadwaita-devel   # Fedora
sudo apt install libgtk-4-dev libadwaita-1-dev # Debian / Ubuntu

# Build
cargo build --release -p face-auth-gtk

# Deploy
sudo ./deploy-gui.sh
```

### Without installing a toolchain (container build)

If `deploy.sh` finds no toolchain it prints this command for you to run. It
does not run it for you: `deploy.sh` runs under `sudo`, and rootless podman
driven through `sudo -u` often fails on a missing `XDG_RUNTIME_DIR`.

```bash
podman run --rm -v "$PWD":/src:Z -w /src docker.io/library/rust:alpine \
  sh -c 'apk add --no-cache musl-dev && \
         cargo build --release --target x86_64-unknown-linux-musl \
           -p face-auth -p face-enroll'
sudo ./deploy.sh
```

`rust:alpine` targets musl natively, so the result is the same static binary.
Run the container as your own user (not under `sudo`) so the files in `target/`
stay yours. This does not work for the GTK GUI, which links against the host's
GTK4 and must be built on the host.

### On immutable distros via distrobox

```bash
# Create a Fedora development container
distrobox create --image registry.fedoraproject.org/fedora:latest --name authface-dev
distrobox enter authface-dev

# Inside the container, install build deps (once).
# Note: Fedora does not package a musl std for Rust, so the musl build needs
# rustup rather than the distro `rust` package.
sudo dnf install -y gcc musl-gcc gtk4-devel libadwaita-devel
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y \
  --target x86_64-unknown-linux-musl
source "$HOME/.cargo/env"

# Build
cd ~/Projects/authFace
cargo build --release --target x86_64-unknown-linux-musl -p face-auth -p face-enroll
cargo build --release -p face-auth-gtk

# Exit container, then deploy on host
exit
sudo ./deploy.sh
sudo ./deploy-gui.sh
```

The GUI binary links against GTK4 dynamically, so build it in an environment
whose GTK version matches the host's — a distrobox sharing the host is fine, an
unrelated container image may not be.

> **`sudo ./deploy.sh` and `cargo`:** if you installed Rust with rustup, `cargo`
> lives in `~/.cargo/bin`, which is not on root's `PATH`. The script looks there
> for the invoking user and runs the build as that user rather than as root, so
> `sudo ./deploy.sh` works and does not leave root-owned files in `target/`.

## Deployment

### Core (PAM authentication)

```bash
sudo ./deploy.sh
```

| Step | What | Details |
|------|------|---------|
| Build | Compiles if `cargo` is available | Falls back to pre-built binaries in `target/` |
| Binaries | Installs to `/usr/local/bin` | `face-auth` + `face-enroll` |
| Detection model | Downloads `version-slim-320.onnx` if missing | From Ultra-Light-Fast-Generic-Face-Detector-1MB upstream |
| Recog. model | Downloads from InsightFace | `w600k_mbf.onnx` (~13 MB) to `/usr/local/share/face-auth/` |
| Config | Installs default config | `/etc/face-auth.toml` |
| PAM | Patches PAM service files | Adds `sufficient` `pam_exec.so quiet` to `sudo`, `gdm-password`, `swaylock` |
| SELinux | Compiles and loads policy | Allows `xdm_t` to mmap camera for lock-screen auth |
| Storage | Creates embeddings directory | `/var/lib/face-auth/<user>/` with sticky bit |

GDM lock-screen patching is **distro-aware**:
- **Fedora/Bluefin/Silverblue** — inserts after `pam_selinux_permit.so`
- **Ubuntu/Debian** — inserts after `#%PAM-1.0`

Each PAM file is backed up with a `.face-auth.bak` suffix.

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

The authentication path and the unprivileged tools trust different things.

**During PAM authentication** (`face-auth`, i.e. sudo / lock screen / login):

| Source | Effect |
|--------|--------|
| `/etc/face-auth.toml` (root-owned) | Authoritative for everything |
| `~/.config/face-auth.toml` | May only make authentication **stricter** — see below |
| `FACE_AUTH_*` environment | **Ignored entirely** |

**For `face-enroll` and the settings GUI**, the usual layering applies:
environment variables, then `~/.config/face-auth.toml`, then `/etc/face-auth.toml`.

### What a user may override at the login prompt

A user's own config is read (resolved via `getent passwd`, so it is *their* home
and not whoever happened to invoke the PAM stack), but it is applied as a
narrowing overlay:

| Key | At the login prompt |
|-----|--------------------|
| `threshold`, `detector_threshold` | Honoured only if **>= the system value**. A lower number is ignored. |
| `device` | Honoured only if the path is a real IR capture device on this machine (IR-looking sysfs name, opens as GREY). |
| `scan_duration_ms`, `scan_interval_ms`, `capture_timeout_ms` | Honoured within built-in bounds. |
| `model_path`, `detector_model_path`, `embeddings_dir` | **Ignored** — system policy only. |

This is what stops code running as you — which does not know your password —
from writing a permissive `~/.config/face-auth.toml` and turning your next
`sudo` into a root shell. To *loosen* matching, edit `/etc/face-auth.toml` as
root; the GUI's slider starts at the system value for the same reason.

Example `/etc/face-auth.toml`:
```toml
device = "/dev/video2"   # usually best left unset; see below
threshold = 0.6
model_path = "/usr/local/share/face-auth/w600k_mbf.onnx"
embeddings_dir = "/var/lib/face-auth"
capture_timeout_ms = 5000
```

Environment variable names follow the field names, so the capture timeout is
`FACE_AUTH_CAPTURE_TIMEOUT_MS` (not `FACE_AUTH_CAPTURE_TIMEOUT`).

> **Leave `device` unset unless you must pin it.** UVC cameras normally expose
> a metadata node right beside the capture node under the same name — on the
> reference ASUS FHD webcam, `/dev/video2` captures and `/dev/video3` does not.
> Auto-detection opens each IR-named candidate and takes the first that is
> really a GREY capture device, which gets this right; a hand-written path
> often does not.

The GUI writes camera and threshold changes to `~/.config/face-auth.toml`.

## Enrollment

Face templates live in a root-owned directory (`/var/lib/face-auth`, mode
`0700`), so enrolment is a privileged operation:

```bash
# Replace existing embeddings with a new capture
sudo face-enroll --user $USER

# Append new embeddings to improve recognition across lighting/angles
sudo face-enroll --improve --user $USER
```

CLI options: `--frames`, `--interval`, `--device`, `--threshold`, `--model`,
`--embeddings-dir`, `--improve`, `-v`.

The GUI's **Enroll Face**, **Improve Matching** and **Test Authentication**
buttons run the same helpers through `pkexec`, so you get a graphical
authentication prompt instead of a terminal. polkit's default for
`org.freedesktop.policykit.exec` is `auth_admin_keep`, so consecutive actions
within a few minutes will not re-prompt.

Why this is not user-writable: whatever can write a face template decides whose
face unlocks that account. If your own login could rewrite it, then so could
anything running as you, and a stolen browser session would become a root
shell at the next `sudo`.

## PAM Integration

The deploy script adds a `sufficient` `pam_exec.so quiet` line to:

| Service | File | Insertion point |
|---------|------|----------------|
| `sudo` | `/etc/pam.d/sudo` | After `#%PAM-1.0` |
| `gdm-password` | `/etc/pam.d/gdm-password` | After `pam_selinux_permit.so` (Fedora) / after `#%PAM-1.0` (Ubuntu/Debian) |
| `swaylock` | `/etc/pam.d/swaylock` | After `#%PAM-1.0` |

`sufficient` means: if face-auth exits 0, the user is authenticated immediately.
If it fails (no match, no camera, timeout), PAM falls through to password prompt.

`quiet` suppresses PAM chatter on the lock screen so the unlock UI stays clean.

No `timeout`, `setenv`, or `env_pass` flags are needed — face-auth reads the camera
(not stdin) and resolves `PAM_USER` via its own fallback chain.

## Lock Screen Scan Indicator (GNOME Shell extension)

By default the scan is silent: `face-auth` runs headless inside PAM, so the only
feedback is the camera LED. A companion GNOME Shell extension shows live status
**on the lock screen** while the face is being scanned:

| Status | Indicator |
|--------|-----------|
| Scanning | Pulsing pill with camera icon + "Scanning face…" |
| Success | Green check — "Face recognised" (briefly) |
| Failure | Red error — "Face not recognised — use your password" |

### How it works

1. `face-auth` (the PAM binary) writes a status file to the authenticated user's
   runtime directory while it runs: `/run/user/<uid>/face-auth-status` containing
   `scanning`, then `ok` or `fail`.
2. The extension watches that file with a `Gio.FileMonitor` while the unlock
   UI is on screen, and renders the indicator above it.

   The file is written with `O_NOFOLLOW` because `face-auth` runs as root and
   the runtime directory belongs to the user — otherwise a symlink there would
   aim a root write at any file on the system.

No daemon, no D-Bus server — just a small status file, keeping the zero-footprint
design of the core.

### Install

```bash
# Requires GNOME Shell 45+ (Fedora 39+, Bazzite, Bluefin, Silverblue, Kinoite)
extensions/authface-scan-indicator/install-extension.sh
# Then: Alt+F2 → r (X11) or log out/in (Wayland)
```

Everything lives in `~/.local/share/gnome-shell/extensions/` — immutable-friendly.

> **Note:** this shows on the **session lock screen** (Super+L / auto-lock), not
> on the GDM login/greeter screen. The greeter runs in a separate locked-down
> shell as the `gdm` user and does not expose hooks for third-party indicators.

## How It Works

```
PAM (sudo / gdm-password / swaylock)
  │
  ▼
face-auth (static binary)
  ├─ Resolve PAM_USER via getent (refuses to guess from USER/LOGNAME)
  ├─ Refuse if PAM_RHOST names a remote host
  ├─ Load /etc/face-auth.toml + strictly-narrowing user overlay
  ├─ V4L2 capture from IR camera (640×400 GREY, auto-detected /dev/videoN)
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

The recognition model is **not bundled** in this repository. `deploy.sh` downloads it from
InsightFace's official GitHub releases and verifies the SHA-256 checksum. The detection
model is auto-downloaded from the Ultra-Light-Fast-Generic-Face-Detector-1MB repository.

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

# Debug output from a live sudo attempt
sudo -k; RUST_LOG=face_auth_core=debug,face_auth=debug sudo true

# Check PAM logs
journalctl | grep -i "pam_exec\|face-auth"

# SELinux denials
journalctl -k | grep face-auth | grep denied

# Test a stored face directly (skips PAM; needs root to read templates)
sudo face-auth --verify $USER
echo $?   # 0 = match, 1 = no match, 2 = error

# Raise the capture timeout (note the _MS suffix)
FACE_AUTH_CAPTURE_TIMEOUT_MS=10000 sudo face-auth --verify $USER

# GUI not launching from app menu?
face-auth-gtk    # run from terminal to see errors
```

### "PAM_USER is not set"

`face-auth` no longer guesses the account from `USER`/`LOGNAME`. If you are
invoking it by hand, use `--verify` rather than setting `PAM_USER` yourself.

### "reports pixel format ... requires raw 8-bit GREY"

The selected device is not an IR sensor — it is an ordinary RGB webcam, or the
metadata node that sits next to the real capture node. Let auto-detection pick
one, or check `v4l2-ctl --device /dev/videoN --list-formats`.

### Preview flickers, or "no face detected" every time

Most Windows Hello IR modules **strobe their illuminator**, emitting a lit frame
and a near-black one alternately. Check what yours does:

```bash
cargo run --example frame-stats
```

On the reference ASUS sensor the lit frames mean 71–237 (of 255) and the unlit
ones 2–5, strictly alternating at 15 fps. authFace captures frames in pairs and
keeps the brighter, so this is handled — but if `frame-stats` shows *every*
frame dark, the illuminator is not firing and no amount of software will help.

A dark frame is worse than useless: histogram equalisation stretches its narrow
range across the full scale and turns sensor noise into a high-contrast grey
field, which is what a flickering preview is showing you.

### Which camera will it use?

```bash
cargo run --example detect-camera
```

Lists every V4L2 node, whether its name looks like an IR sensor, whether it
really opens as a GREY capture device, and which one authentication would pick.

### Multi-camera picks the wrong device / no device selected

```bash
# See which IR device is detected
sudo face-auth --verify $USER   # with RUST_LOG=face_auth_core=debug

# Pin a specific camera in your own config (must be a real IR device)
echo 'device = "/dev/video2"' >> ~/.config/face-auth.toml
```

### My threshold change did nothing

A user config may only make matching *stricter*. To loosen it, lower
`threshold` in `/etc/face-auth.toml` as root — see [Configuration](#configuration).

## Security & Limitations

### Trust model

- **Face templates are root-owned.** `/var/lib/face-auth` is mode `0700`,
  root:root, with templates at `0600`. Whatever can write a template decides
  whose face unlocks that account, so enrolment goes through `sudo`/`pkexec`.
- **The PAM path trusts only `/etc/face-auth.toml`.** A user's own config may
  make matching stricter, never looser, and may not redirect the model or
  template paths. `FACE_AUTH_*` environment variables are ignored during
  authentication. See [Configuration](#configuration).
- **Identity comes from `PAM_USER` only.** `face-auth` refuses to run if PAM
  did not set it, rather than falling back to `USER`, `LOGNAME` or `id -un`.
- **Remote sessions are refused.** If `PAM_RHOST` names a non-local host,
  face authentication is declined — the camera is at the console, so otherwise
  whoever is sitting at the desk would authenticate an SSH session.

### Known limitations

- **IR-only, no liveness detection:** an IR camera resists casual photo
  spoofing, but there is no structured-light or dot-projection depth check.
  A high-quality IR-visible print or a 3D mask may bypass verification. This is
  the main residual risk and it is inherent to the approach — treat face unlock
  as a convenience over a password you still have, not as a stronger factor.
- **No rate limiting or lockout.** Every prompt allows a fresh scan window.
  PAM's own `pam_faildelay`/`pam_tally2` are not wired up.
- **`sufficient` bypasses the rest of the auth stack.** A successful match
  satisfies authentication outright; any other `auth` module below the
  face-auth line is skipped. That is the point, but it means the strength of
  the whole stack becomes the strength of the face match.
- **SELinux policy scope:** the lock-screen policy grants `xdm_t` mmap access
  to all V4L2 devices. A trade-off for drop-in compatibility; narrowing it
  requires custom udev device types.
- **x86_64 only:** V4L2 ioctl numbers and struct layouts are hardcoded.
  ARM/aarch64 requires switching to the `v4l` crate.
- **Model integrity:** both ONNX models are pinned by SHA-256 and the detector
  URL is pinned to a commit, not a branch. `deploy.sh` aborts on mismatch.

## Project Structure

```
authFace/
  crates/
    face-auth-core/          # Core library
      src/
        capture.rs           # V4L2 capture + poll() timeout + IR camera auto-detect
        config.rs            # Layered config + narrowing overlay for PAM + per-user load
        detector.rs          # Face detection (RetinaFace-based ONNX model)
        error.rs             # Error types
        inference.rs         # tract-onnx model loading + encoding
        lib.rs               # FaceAuth struct, auth + enroll + scan
        preprocess.rs        # Histogram equalize, resize, normalize
        storage.rs           # Binary embedding I/O (versioned, atomic, 0600)
        user.rs              # NSS lookup + username validation
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
  extensions/
    authface-scan-indicator/ # GNOME Shell lock-screen scan indicator
      extension.js
      metadata.json
      install-extension.sh
  deploy.sh                  # Core auth installer
  deploy-gui.sh              # Optional GUI installer
  uninstall.sh               # Removal script (--gui, --purge flags)
```

## License

MIT

This is a fork of [pfalkingham/authFace](https://github.com/pfalkingham/authFace) (MIT). The
facial recognition model is InsightFace's `w600k_mbf.onnx` (MIT) and the face detector is
`version-slim-320.onnx` (MIT).
