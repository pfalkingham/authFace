# Changelog

## [Unreleased]

### Fixed
- User detection now uses a fallback chain (`PAM_USER` → `USER` → `LOGNAME` → `id -un`), no `setenv`/`env_pass` flags needed
- Removed `timeout=10` from PAM stanzas (causes pam_exec to block on stdin; face-auth reads the camera, not stdin)
- `deploy.sh` no longer wipes `/var/lib/face-auth/` on redeploy (preserves enrolled users)
- Model checksum mismatch now aborts deployment instead of continuing with potentially corrupted model
- User config (`~/.config/face-auth.toml`) now correctly overrides system config (`/etc/face-auth.toml`)
- `face-enroll` validates that the target user exists before attempting enrollment
- Camera buffer mmap changed to `PROT_READ` only (principle of least privilege)
- Pinned `image` crate to `0.25.4` (addresses known soundness issues in 0.25.x)

### Added
- `uninstall.sh --purge` flag to optionally remove user embeddings
- Architecture and security limitation documentation in README
- CHANGELOG.md

### Changed
- Clarified SELinux policy scope and trade-offs in documentation
- Added x86_64-only architecture warning to V4L2 capture module
