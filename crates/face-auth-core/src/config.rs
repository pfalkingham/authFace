use crate::user;
use anyhow::{bail, Context, Result};
use config::{Config, Environment, File};
use serde::Deserialize;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub const SYSTEM_CONFIG_PATH: &str = "/etc/face-auth.toml";

/// Default embeddings location. Must stay root-owned and 0700 — see deploy.sh.
pub const DEFAULT_EMBEDDINGS_DIR: &str = "/var/lib/face-auth";

// Bounds applied to every config source. A threshold near zero accepts any
// face at all, so the floor is enforced rather than merely documented.
const THRESHOLD_RANGE: std::ops::RangeInclusive<f32> = 0.3..=1.0;
const DETECTOR_THRESHOLD_RANGE: std::ops::RangeInclusive<f32> = 0.05..=1.0;
const CAPTURE_TIMEOUT_RANGE: std::ops::RangeInclusive<u64> = 100..=30_000;
const SCAN_DURATION_RANGE: std::ops::RangeInclusive<u64> = 500..=30_000;
const SCAN_INTERVAL_RANGE: std::ops::RangeInclusive<u64> = 0..=5_000;

#[derive(Debug, Deserialize, Clone)]
pub struct FaceAuthConfig {
    pub device: Option<String>,
    pub threshold: Option<f32>,
    pub model_path: Option<String>,
    pub embeddings_dir: Option<String>,
    pub capture_timeout_ms: Option<u64>,
    pub detector_model_path: Option<String>,
    pub detector_threshold: Option<f32>,
    pub scan_duration_ms: Option<u64>,
    pub scan_interval_ms: Option<u64>,
}

impl Default for FaceAuthConfig {
    fn default() -> Self {
        Self {
            device: None,
            threshold: Some(0.6),
            model_path: None,
            embeddings_dir: None,
            capture_timeout_ms: Some(5000),
            detector_model_path: None,
            detector_threshold: Some(0.5),
            scan_duration_ms: Some(5000),
            scan_interval_ms: Some(0),
        }
    }
}

/// Read a file, refusing to follow a symlink at the final component.
///
/// Used for config files under a user's home: the authentication helper runs
/// as root, and a symlink there would otherwise aim root's read at a file the
/// user cannot open themselves.
fn read_nofollow(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)?;
    Ok(buf)
}

impl FaceAuthConfig {
    /// Full layered load: system file, then the calling user's file, then
    /// `FACE_AUTH_*` environment overrides.
    ///
    /// Every source here is writable by whoever runs the process, so this is
    /// for unprivileged tools only — `face-enroll` and the settings GUI. The
    /// authentication path must use [`FaceAuthConfig::load_for_auth`].
    pub fn load() -> Result<Self> {
        let mut builder = Config::builder();

        let system_config = PathBuf::from(SYSTEM_CONFIG_PATH);
        if system_config.exists() {
            builder = builder.add_source(File::from(system_config));
        }

        if let Some(config_dir) = dirs::config_dir() {
            let user_config = config_dir.join("face-auth.toml");
            if user_config.exists() {
                builder = builder.add_source(File::from(user_config));
            }
        }

        // try_parsing so numeric keys coerce from their string env values.
        builder = builder.add_source(Environment::with_prefix("FACE_AUTH").try_parsing(true));

        let config: FaceAuthConfig = builder.build()?.try_deserialize()?;
        config.validate()?;
        Ok(config)
    }

    /// Configuration for the PAM authentication path.
    ///
    /// Trust model: only the root-owned system file may decide *what* is
    /// checked — camera device, models, and the embeddings directory. The
    /// target user's own config is consulted for comfort settings, and for
    /// thresholds it may only ever tighten the system value, never relax it.
    /// The environment is ignored outright.
    ///
    /// Without this split, anything running as the user (malware that never
    /// learned their password) could drop a `~/.config/face-auth.toml` with a
    /// permissive threshold, or repoint `embeddings_dir` at a directory it
    /// controls, and turn their next `sudo` into root.
    pub fn load_for_auth(username: &str) -> Result<Self> {
        let mut builder = Config::builder();
        let system_config = PathBuf::from(SYSTEM_CONFIG_PATH);
        if system_config.exists() {
            builder = builder.add_source(File::from(system_config));
        }
        let mut config: FaceAuthConfig = builder.build()?.try_deserialize()?;
        config.validate().context("invalid system config")?;

        if let Some(overlay) = load_user_overlay(username) {
            config.apply_user_overlay(&overlay);
        }

        Ok(config)
    }

    /// Merge the safe subset of a user's config over a system baseline.
    fn apply_user_overlay(&mut self, overlay: &FaceAuthConfig) {
        // Thresholds: accept only values at least as strict as the system's.
        if let Some(t) = overlay.threshold {
            if t.is_finite() && t >= self.threshold() && THRESHOLD_RANGE.contains(&t) {
                self.threshold = Some(t);
            } else {
                tracing::debug!(
                    requested = t,
                    floor = self.threshold(),
                    "ignoring user threshold: would weaken authentication"
                );
            }
        }
        if let Some(t) = overlay.detector_threshold {
            if t.is_finite()
                && t >= self.detector_threshold()
                && DETECTOR_THRESHOLD_RANGE.contains(&t)
            {
                self.detector_threshold = Some(t);
            }
        }

        // Timing preferences cannot weaken a match decision, only how long the
        // user is willing to wait, so they are honoured within bounds.
        if let Some(v) = overlay.capture_timeout_ms {
            if CAPTURE_TIMEOUT_RANGE.contains(&v) {
                self.capture_timeout_ms = Some(v);
            }
        }
        if let Some(v) = overlay.scan_duration_ms {
            if SCAN_DURATION_RANGE.contains(&v) {
                self.scan_duration_ms = Some(v);
            }
        }
        if let Some(v) = overlay.scan_interval_ms {
            if SCAN_INTERVAL_RANGE.contains(&v) {
                self.scan_interval_ms = Some(v);
            }
        }

        // Which IR sensor to use is a preference on a multi-camera machine, so
        // it is honoured — but only after confirming the path really is an IR
        // capture device here. Taken unchecked it would let an unprivileged
        // setting aim authentication at any video source at all.
        if let Some(dev) = overlay.device.as_deref() {
            if crate::capture::is_ir_capture_device(dev) {
                self.device = Some(dev.to_string());
            } else {
                tracing::warn!(device = dev, "ignoring user camera: not an IR capture device");
            }
        }

        // model_path, detector_model_path and embeddings_dir stay system
        // policy: each decides what gets compared against what.
    }

    pub fn validate(&self) -> Result<()> {
        let t = self.threshold();
        if !t.is_finite() || !THRESHOLD_RANGE.contains(&t) {
            bail!(
                "threshold {} outside safe range {}..={}",
                t,
                THRESHOLD_RANGE.start(),
                THRESHOLD_RANGE.end()
            );
        }
        let d = self.detector_threshold();
        if !d.is_finite() || !DETECTOR_THRESHOLD_RANGE.contains(&d) {
            bail!("detector_threshold {} outside safe range", d);
        }
        if !self.embeddings_dir().is_absolute() {
            bail!("embeddings_dir must be an absolute path");
        }
        Ok(())
    }

    pub fn device(&self) -> String {
        self.device
            .clone()
            .or_else(crate::capture::detect_ir_camera)
            .unwrap_or_else(|| "/dev/video0".to_string())
    }

    pub fn threshold(&self) -> f32 {
        self.threshold.unwrap_or(0.6)
    }

    pub fn model_path(&self) -> String {
        self.model_path
            .clone()
            .unwrap_or_else(|| "/usr/local/share/face-auth/w600k_mbf.onnx".to_string())
    }

    pub fn capture_timeout_ms(&self) -> i32 {
        self.capture_timeout_ms
            .unwrap_or(5000)
            .clamp(*CAPTURE_TIMEOUT_RANGE.start(), *CAPTURE_TIMEOUT_RANGE.end()) as i32
    }

    pub fn embeddings_dir(&self) -> PathBuf {
        self.embeddings_dir
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_EMBEDDINGS_DIR))
    }

    pub fn detector_model_path(&self) -> String {
        self.detector_model_path
            .clone()
            .unwrap_or_else(|| "/usr/local/share/face-auth/version-slim-320.onnx".to_string())
    }

    pub fn detector_threshold(&self) -> f32 {
        self.detector_threshold.unwrap_or(0.5)
    }

    pub fn scan_duration_ms(&self) -> u64 {
        self.scan_duration_ms
            .unwrap_or(5000)
            .clamp(*SCAN_DURATION_RANGE.start(), *SCAN_DURATION_RANGE.end())
    }

    /// Extra delay between scan attempts.
    ///
    /// Defaults to zero: the loop is already paced by the camera, which
    /// delivers 15 frames a second while a single attempt costs ~235ms of
    /// inference, so it cannot spin. The old 200ms default added most of a
    /// second across a handful of attempts for nothing.
    pub fn scan_interval_ms(&self) -> u64 {
        self.scan_interval_ms
            .unwrap_or(0)
            .clamp(*SCAN_INTERVAL_RANGE.start(), *SCAN_INTERVAL_RANGE.end())
    }
}

/// Read `~/.config/face-auth.toml` for the account being authenticated.
///
/// Resolved through NSS rather than `$HOME`, which under `pam_exec` belongs to
/// whoever invoked the stack rather than to the target user. Any failure is
/// non-fatal: the system configuration alone is a valid setup.
fn load_user_overlay(username: &str) -> Option<FaceAuthConfig> {
    let info = user::lookup(username)
        .map_err(|e| tracing::debug!("no passwd entry for '{username}': {e}"))
        .ok()?;

    let path = info.home.join(".config/face-auth.toml");
    let contents = read_nofollow(&path)
        .map_err(|e| tracing::debug!("no readable user config at {}: {e}", path.display()))
        .ok()?;

    // Cap the size so a huge or binary file cannot become a parsing problem.
    if contents.len() > 64 * 1024 {
        tracing::warn!("user config {} too large, ignoring", path.display());
        return None;
    }

    match toml::from_str::<FaceAuthConfig>(&contents) {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            tracing::warn!("ignoring malformed user config {}: {e}", path.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system_baseline() -> FaceAuthConfig {
        FaceAuthConfig {
            threshold: Some(0.6),
            detector_threshold: Some(0.5),
            embeddings_dir: Some(DEFAULT_EMBEDDINGS_DIR.to_string()),
            device: Some("/dev/video3".to_string()),
            model_path: Some("/usr/local/share/face-auth/w600k_mbf.onnx".to_string()),
            ..FaceAuthConfig::default()
        }
    }

    #[test]
    fn user_overlay_cannot_weaken_threshold() {
        let mut cfg = system_baseline();
        let overlay = FaceAuthConfig {
            threshold: Some(0.1),
            ..FaceAuthConfig::default()
        };
        cfg.apply_user_overlay(&overlay);
        assert_eq!(
            cfg.threshold(),
            0.6,
            "user must not be able to relax matching"
        );
    }

    #[test]
    fn user_overlay_may_tighten_threshold() {
        let mut cfg = system_baseline();
        let overlay = FaceAuthConfig {
            threshold: Some(0.8),
            ..FaceAuthConfig::default()
        };
        cfg.apply_user_overlay(&overlay);
        assert_eq!(cfg.threshold(), 0.8);
    }

    #[test]
    fn user_overlay_cannot_redirect_lookups() {
        let mut cfg = system_baseline();
        let overlay = FaceAuthConfig {
            embeddings_dir: Some("/home/mallory/faces".to_string()),
            model_path: Some("/home/mallory/evil.onnx".to_string()),
            detector_model_path: Some("/home/mallory/evil2.onnx".to_string()),
            ..FaceAuthConfig::default()
        };
        cfg.apply_user_overlay(&overlay);
        assert_eq!(cfg.embeddings_dir(), PathBuf::from(DEFAULT_EMBEDDINGS_DIR));
        assert!(cfg.model_path().starts_with("/usr/local/share"));
        assert!(cfg.detector_model_path().starts_with("/usr/local/share"));
    }

    #[test]
    fn user_overlay_rejects_a_camera_that_is_not_an_ir_device() {
        // No such node exists in the test environment, so the validation in
        // is_ir_capture_device must reject it and keep the system choice.
        let mut cfg = system_baseline();
        let overlay = FaceAuthConfig {
            device: Some("/dev/video99".to_string()),
            ..FaceAuthConfig::default()
        };
        cfg.apply_user_overlay(&overlay);
        assert_eq!(cfg.device, Some("/dev/video3".to_string()));

        for bogus in ["/etc/passwd", "../dev/video0", "/dev/../etc/passwd", "video0"] {
            let mut cfg = system_baseline();
            cfg.apply_user_overlay(&FaceAuthConfig {
                device: Some(bogus.to_string()),
                ..FaceAuthConfig::default()
            });
            assert_eq!(cfg.device, Some("/dev/video3".to_string()), "accepted {bogus}");
        }
    }

    #[test]
    fn user_overlay_honours_timing_within_bounds() {
        let mut cfg = system_baseline();
        let overlay = FaceAuthConfig {
            scan_duration_ms: Some(8000),
            scan_interval_ms: Some(999_999),
            ..FaceAuthConfig::default()
        };
        cfg.apply_user_overlay(&overlay);
        assert_eq!(cfg.scan_duration_ms(), 8000);
        assert_eq!(cfg.scan_interval_ms(), 0, "out-of-range value ignored");
    }

    #[test]
    fn validate_rejects_permissive_threshold() {
        let cfg = FaceAuthConfig {
            threshold: Some(0.0),
            ..FaceAuthConfig::default()
        };
        assert!(cfg.validate().is_err());

        let cfg = FaceAuthConfig {
            threshold: Some(f32::NAN),
            ..FaceAuthConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_relative_embeddings_dir() {
        let cfg = FaceAuthConfig {
            embeddings_dir: Some("relative/path".to_string()),
            ..FaceAuthConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn empty_config_deserialises_to_all_defaults() {
        // load_for_auth builds from zero sources when /etc/face-auth.toml is
        // absent. If that errored, face-auth would refuse to start on a system
        // with no system config rather than falling back to defaults.
        let cfg: FaceAuthConfig = Config::builder().build().unwrap().try_deserialize().unwrap();
        assert!(cfg.validate().is_ok());
        assert_eq!(cfg.threshold(), 0.6);
        assert_eq!(cfg.embeddings_dir(), PathBuf::from(DEFAULT_EMBEDDINGS_DIR));
    }

    #[test]
    fn defaults_are_valid() {
        assert!(FaceAuthConfig::default().validate().is_ok());
    }
}
