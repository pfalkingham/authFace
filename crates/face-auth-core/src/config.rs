use config::{Config, File, Environment};
use dirs;
use serde::Deserialize;
use std::path::PathBuf;

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
            scan_interval_ms: Some(200),
        }
    }
}

impl FaceAuthConfig {
    pub fn load() -> anyhow::Result<Self> {
        Self::load_for_user(None)
    }

    pub fn load_for_user(user: Option<&str>) -> anyhow::Result<Self> {
        let mut builder = Config::builder();

        // System config (lower priority)
        let system_config = PathBuf::from("/etc/face-auth.toml");
        if system_config.exists() {
            builder = builder.add_source(File::from(system_config));
        }

        // User config (higher priority, overrides system)
        let mut user_config_path = None;
        if let Some(username) = user {
            if let Ok(output) = std::process::Command::new("getent").arg("passwd").arg(username).output() {
                if output.status.success() {
                    if let Ok(line) = String::from_utf8(output.stdout) {
                        let parts: Vec<&str> = line.trim().split(':').collect();
                        if parts.len() >= 6 {
                            let home = PathBuf::from(parts[5]);
                            let cfg = home.join(".config/face-auth.toml");
                            if cfg.exists() {
                                user_config_path = Some(cfg);
                            }
                        }
                    }
                }
            }
        }

        if user_config_path.is_none() {
            if let Some(config_dir) = dirs::config_dir() {
                let user_config = config_dir.join("face-auth.toml");
                if user_config.exists() {
                    user_config_path = Some(user_config);
                }
            }
        }

        if let Some(cfg) = user_config_path {
            builder = builder.add_source(File::from(cfg));
        }

        builder = builder.add_source(Environment::with_prefix("FACE_AUTH"));

        let config: FaceAuthConfig = builder.build()?.try_deserialize()?;
        Ok(config)
    }

    pub fn device(&self) -> String {
        self.device
            .clone()
            .or_else(|| detect_ir_camera())
            .unwrap_or_else(|| "/dev/video3".to_string())
    }

    pub fn threshold(&self) -> f32 {
        self.threshold.unwrap_or(0.6)
    }

    pub fn model_path(&self) -> String {
        self.model_path
            .clone()
            .or_else(|| {
                std::env::var("FACE_AUTH_MODEL_PATH").ok()
            })
            .unwrap_or_else(|| "/usr/local/share/face-auth/w600k_mbf.onnx".to_string())
    }

    pub fn capture_timeout_ms(&self) -> i32 {
        self.capture_timeout_ms.unwrap_or(5000) as i32
    }

    pub fn embeddings_dir(&self) -> PathBuf {
        self.embeddings_dir
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/lib/face-auth"))
    }

    pub fn detector_model_path(&self) -> String {
        self.detector_model_path
            .clone()
            .or_else(|| std::env::var("FACE_AUTH_DETECTOR_MODEL_PATH").ok())
            .unwrap_or_else(|| "/usr/local/share/face-auth/version-slim-320.onnx".to_string())
    }

    pub fn detector_threshold(&self) -> f32 {
        self.detector_threshold.unwrap_or(0.5)
    }

    pub fn scan_duration_ms(&self) -> u64 {
        self.scan_duration_ms.unwrap_or(5000)
    }

    pub fn scan_interval_ms(&self) -> u64 {
        self.scan_interval_ms.unwrap_or(200)
    }
}

fn detect_ir_camera() -> Option<String> {
    use std::fs;
    let base = std::path::Path::new("/sys/class/video4linux");
    if let Ok(entries) = fs::read_dir(base) {
        for entry in entries.flatten() {
            let name_path = entry.path().join("name");
            if let Ok(name) = fs::read_to_string(&name_path) {
                if name.to_lowercase().contains("ir") || name.to_lowercase().contains("infrared") {
                    if let Some(device_name) = entry.file_name().to_str() {
                        return Some(format!("/dev/{}", device_name));
                    }
                }
            }
        }
    }
    None
}