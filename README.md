# authFace

IR camera facial authentication for Linux. Works on immutable distros (Bluefin, Silverblue, etc.) because it's just a static binary + PAM config — no packages, no daemons, no layering.

## Features

- **Windows Hello–style IR camera auth** for sudo and GNOME lock screen
- **Password fallback** — never get locked out
- **Static musl binary** (~20 MB, zero dependencies)
- **No daemon, no systemd, no D-Bus** — just `pam_exec.so`
- **Configurable** via `/etc/face-auth.toml`, `~/.config/face-auth.toml`, or env vars

## Quick Start

```bash
sudo ./deploy.sh          # install binaries, model, PAM configs
face-enroll --user $USER  # capture 5 face embeddings
sudo true                 # triggers IR camera → authenticates
```

Lock screen: `Super+L`, then press any key — camera fires, unlocks automatically.

## Manual Build

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl -p face-auth -p face-enroll
sudo ./deploy.sh
```

## Enrollment

```bash
face-enroll --user $USER
```

Options: `--frames`, `--interval`, `--device`, `--threshold`, `-v`.

## PAM Integration

The deploy script adds a `sufficient` `pam_exec.so` line to:

| Service | File | Notes |
|---------|------|-------|
| `sudo` | `/etc/pam.d/sudo` | Interactive sudo |
| `gdm-password` | `/etc/pam.d/gdm-password` | GNOME lock screen |
| `swaylock` | `/etc/pam.d/swaylock` | Sway lock screen |

`sufficient` means face-auth success = authenticated; failure = fall through to password.

## SELinux

On Fedora/Bluefin with enforcing SELinux, the GNOME lock screen (`xdm_t` domain) needs a policy to access the IR camera. The deploy script installs it automatically:

```
allow xdm_t v4l_device_t:chr_file map;
```

To remove: `sudo semodule -r face_auth`

## Uninstall

```bash
sudo ./uninstall.sh
```

Restores PAM backups, removes binaries, model, config, and SELinux policy. Preserves `/var/lib/face-auth/` (your embeddings).

## How It Works

```
PAM (sudo/gdm-password)
  │
  ▼
face-auth (static binary)
  ├─ V4L2 capture from IR camera (/dev/video3, GREY format)
  ├─ Histogram equalize → resize 112×112 → normalize to [-1, 1]
  ├─ tract-onnx inference (MobileFaceNet, 512-d embedding)
  ├─ Cosine similarity vs stored embeddings (threshold 0.6)
  └─ Exit 0 (match) or exit 1 (no match → password prompt)
```

Model: InsightFace `w600k_mbf.onnx` (MobileFaceNet @ WebFace600K, ~13 MB). Auto-downloaded by `deploy.sh`.

## Troubleshooting

```bash
# Find IR camera
ls /sys/class/video4linux/*/name

# Grant video group access
sudo usermod -aG video $USER

# Debug output
RUST_LOG=face_auth_core=debug face-enroll --user $USER

# Check PAM logs
journalctl | grep -i "pam_exec\|face-auth"

# SELinux denials
journalctl -k | grep face-auth | grep denied
```

## License

MIT
