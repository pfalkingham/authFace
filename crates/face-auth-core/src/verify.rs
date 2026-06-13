use crate::storage::EmbeddingStore;
use crate::error::FaceAuthError;

pub fn verify_embedding(
    probe: &[f32],
    store: &EmbeddingStore,
    threshold: f32,
) -> anyhow::Result<bool> {
    if store.embeddings.is_empty() {
        return Err(FaceAuthError::NoEmbeddings.into());
    }
    
    let mut max_similarity = 0.0f32;
    
    for stored in &store.embeddings {
        let similarity = cosine_similarity(probe, stored);
        if similarity > max_similarity {
            max_similarity = similarity;
        }
    }
    
    tracing::debug!("Max similarity: {:.4}, threshold: {:.4}", max_similarity, threshold);
    
    Ok(max_similarity >= threshold)
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
        
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 0.0).abs() < 1e-6);
        
        let a = vec![1.0, 1.0, 0.0];
        let b = vec![1.0, 1.0, 0.0];
        let norm = 2.0f32.sqrt();
        let expected = 2.0 / (norm * norm);
        assert!((cosine_similarity(&a, &b) - expected).abs() < 1e-6);
    }
}