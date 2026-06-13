pub mod capture;
pub mod config;
pub mod error;
pub mod inference;
pub mod preprocess;
pub mod storage;
pub mod verify;

pub use crate::config::FaceAuthConfig;
use crate::inference::FaceEncoder;
use crate::storage::EmbeddingStore;
use crate::verify::verify_embedding;
use anyhow::Result;

pub struct FaceAuth {
    config: FaceAuthConfig,
    encoder: FaceEncoder,
}

impl FaceAuth {
    pub fn new(config: FaceAuthConfig) -> Result<Self> {
        let encoder = FaceEncoder::new(&config.model_path())?;
        Ok(Self { config, encoder })
    }
    
    pub fn authenticate(&mut self, user: &str) -> Result<bool> {
        let frame = crate::capture::capture_ir_frame(&self.config.device(), self.config.capture_timeout_ms())?;
        let mut frame = frame;
        crate::preprocess::histogram_equalize(&mut frame);
        let input = crate::preprocess::preprocess_ir_frame(&frame)?;
        let embedding = self.encoder.encode(input.view())?;
        
        let store = EmbeddingStore::load(user, &self.config.embeddings_dir())?;
        verify_embedding(&embedding, &store, self.config.threshold())
    }
    
    pub fn enroll(&mut self, user: &str, frames: usize, interval_ms: u64) -> Result<()> {
        let mut store = EmbeddingStore::default();
        
        for i in 0..frames {
            println!("Capturing frame {}/{}...", i + 1, frames);
            let frame = crate::capture::capture_ir_frame(&self.config.device(), self.config.capture_timeout_ms())?;
            let mut frame = frame;
            crate::preprocess::histogram_equalize(&mut frame);
            let input = crate::preprocess::preprocess_ir_frame(&frame)?;
            let embedding = self.encoder.encode(input.view())?;
            store.add_embedding(embedding);
            
            if i < frames - 1 {
                std::thread::sleep(std::time::Duration::from_millis(interval_ms));
            }
        }
        
        store.save(user, &self.config.embeddings_dir())?;
        println!("Saved {} embeddings for user '{}'", frames, user);
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_config_defaults() {
        let config = FaceAuthConfig::default();
        assert_eq!(config.threshold(), 0.6);
        assert!(config.device().starts_with("/dev/video"));
    }
}