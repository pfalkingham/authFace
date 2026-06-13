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
}

impl Default for FaceAuthConfig {
    fn default() -> Self {
        Self {
            device: None,
            threshold: Some(0.6),
            model_path: None,
            embeddings_dir: None,
        }
    }
}

impl FaceAuthConfig {
    pub fn load() -> anyhow::Result<Self> {
        let mut builder = Config::builder();

        // System config (lower priority)
        let system_config = PathBuf::from("/etc/face-auth.toml");
        if system_config.exists() {
            builder = builder.add_source(File::from(system_config));
        }

        // User config (higher priority, overrides system)
        if let Some(config_dir) = dirs::config_dir() {
            let user_config = config_dir.join("face-auth.toml");
            if user_config.exists() {
                builder = builder.add_source(File::from(user_config));
            }
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

    pub fn embeddings_dir(&self) -> PathBuf {
        self.embeddings_dir
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/lib/face-auth"))
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