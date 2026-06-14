pub mod capture;
pub mod config;
pub mod error;
pub mod inference;
pub mod preprocess;
pub mod storage;
pub mod verify;

pub use crate::capture::Camera;
pub use crate::config::FaceAuthConfig;
use crate::inference::FaceEncoder;
use crate::storage::EmbeddingStore;
use crate::verify::verify_embedding;
use anyhow::Result;
use std::time::Instant;

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
        // Fail fast if user isn't enrolled — don't touch the camera
        let t0 = Instant::now();
        let store = EmbeddingStore::load(user, &self.config.embeddings_dir())?;
        eprintln!("TIMING store_load: {:?}", t0.elapsed());

        let t1 = Instant::now();
        let frame = crate::capture::capture_ir_frame(&self.config.device(), self.config.capture_timeout_ms())?;
        eprintln!("TIMING capture: {:?}", t1.elapsed());

        let t2 = Instant::now();
        let mut frame = frame;
        crate::preprocess::histogram_equalize(&mut frame);
        let input = crate::preprocess::preprocess_ir_frame(&frame)?;
        eprintln!("TIMING preprocess: {:?}", t2.elapsed());

        let t3 = Instant::now();
        let embedding = self.encoder.encode(input.view())?;
        eprintln!("TIMING encode: {:?}", t3.elapsed());

        let result = verify_embedding(&embedding, &store, self.config.threshold());
        result
    }
    
    pub fn enroll(&mut self, user: &str, frames: usize, interval_ms: u64) -> Result<()> {
        let mut store = EmbeddingStore::default();
        let mut cam = Camera::open(&self.config.device(), 640, 400)?;
        
        for i in 0..frames {
            println!("Capturing frame {}/{}...", i + 1, frames);
            let frame = cam.capture_frame(self.config.capture_timeout_ms())?;
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