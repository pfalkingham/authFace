use crate::error::FaceAuthError;
use crate::storage::EmbeddingStore;

pub fn verify_embedding(
    probe: &[f32],
    store: &EmbeddingStore,
    threshold: f32,
) -> anyhow::Result<bool> {
    if store.embeddings.is_empty() {
        return Err(FaceAuthError::NoEmbeddings.into());
    }

    // A threshold of zero or below matches anything. Config validation should
    // already have caught it; refuse rather than authenticate if it has not.
    if !threshold.is_finite() || threshold <= 0.0 {
        anyhow::bail!("refusing to verify against non-positive threshold {threshold}");
    }

    // A non-finite probe makes every comparison NaN, which compares false and
    // so fails closed — but silently. Say why instead.
    if !probe.iter().all(|v| v.is_finite()) {
        anyhow::bail!("probe embedding contains non-finite values");
    }

    let mut max_similarity = f32::NEG_INFINITY;

    for stored in &store.embeddings {
        anyhow::ensure!(
            stored.len() == probe.len(),
            "embedding length mismatch: stored {} vs probe {}",
            stored.len(),
            probe.len()
        );
        let similarity = cosine_similarity(probe, stored);
        if similarity > max_similarity {
            max_similarity = similarity;
        }
    }

    tracing::debug!(
        similarity = max_similarity,
        threshold,
        "verification complete"
    );

    Ok(max_similarity >= threshold)
}

/// Cosine similarity of two equal-length vectors.
///
/// Callers must check the lengths first: `zip` would otherwise stop at the
/// shorter of the two and score a prefix as though it were the whole vector.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
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

    fn store_of(vecs: Vec<Vec<f32>>) -> EmbeddingStore {
        EmbeddingStore { embeddings: vecs }
    }

    #[test]
    fn cosine_similarity_basics() {
        let a = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);

        let b = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);

        let c = vec![-1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &c) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn matches_identical_embedding() {
        let probe = vec![1.0, 0.0, 0.0];
        let store = store_of(vec![vec![1.0, 0.0, 0.0]]);
        assert!(verify_embedding(&probe, &store, 0.6).unwrap());
    }

    #[test]
    fn rejects_orthogonal_embedding() {
        let probe = vec![1.0, 0.0, 0.0];
        let store = store_of(vec![vec![0.0, 1.0, 0.0]]);
        assert!(!verify_embedding(&probe, &store, 0.6).unwrap());
    }

    #[test]
    fn opposed_embedding_scores_below_zero_not_clamped() {
        // The old implementation seeded the running maximum at 0.0, so an
        // anti-correlated match was indistinguishable from an orthogonal one.
        let probe = vec![1.0, 0.0, 0.0];
        let store = store_of(vec![vec![-1.0, 0.0, 0.0]]);
        assert!(!verify_embedding(&probe, &store, 0.6).unwrap());
    }

    #[test]
    fn rejects_length_mismatch_instead_of_comparing_a_prefix() {
        let probe = vec![1.0, 0.0, 0.0, 0.0];
        let store = store_of(vec![vec![1.0, 0.0]]);
        assert!(verify_embedding(&probe, &store, 0.6).is_err());
    }

    #[test]
    fn rejects_non_positive_threshold() {
        let probe = vec![1.0, 0.0, 0.0];
        let store = store_of(vec![vec![0.0, 1.0, 0.0]]);
        assert!(verify_embedding(&probe, &store, 0.0).is_err());
        assert!(verify_embedding(&probe, &store, -1.0).is_err());
        assert!(verify_embedding(&probe, &store, f32::NAN).is_err());
    }

    #[test]
    fn rejects_non_finite_probe() {
        let probe = vec![f32::NAN, 0.0, 0.0];
        let store = store_of(vec![vec![1.0, 0.0, 0.0]]);
        assert!(verify_embedding(&probe, &store, 0.6).is_err());
    }

    #[test]
    fn empty_store_is_an_error_not_a_pass() {
        let probe = vec![1.0, 0.0, 0.0];
        assert!(verify_embedding(&probe, &store_of(vec![]), 0.6).is_err());
    }
}
