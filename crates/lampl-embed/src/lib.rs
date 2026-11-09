// SPDX-License-Identifier: Apache-2.0

//! Embedders for lamplighter: Qwen3-Embedding-0.6B in-process on CPU (candle), or an HTTP endpoint.

mod endpoint;
mod local;
mod truncate;

use std::path::PathBuf;

pub use endpoint::{EndpointEmbedder, EndpointKind};
pub use local::{LocalConfig, LocalEmbedder, default_cache_dir, model_cached, prepare};
pub use truncate::{FALLBACK_MAX_CHARS, Truncator, prepare_tokenizer};

pub const DEFAULT_REPO_ID: &str = "Qwen/Qwen3-Embedding-0.6B";
pub const DIM: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("model weights not found under {}; run `lampl doctor` (or the explicit prepare step) to download them", .cache_dir.display())]
    ModelMissing { cache_dir: PathBuf },
    #[error("model download failed: {0}")]
    Download(String),
    #[error("model load failed: {0}")]
    Load(String),
    #[error("embedding failed: {0}")]
    Embed(String),
    #[error("endpoint error: {0}")]
    Endpoint(String),
    #[error("dimension mismatch: expected {expected}, got {got}")]
    DimMismatch { expected: usize, got: usize },
}

/// A text embedder. Implementations return one L2-normalised vector per input, in input order, and never add a query-instruction prefix.
pub trait Embedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError>;
    fn dim(&self) -> usize;
    fn model_id(&self) -> &str;
    fn model_digest(&self) -> &str;
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_of_identical_vectors_is_one() {
        assert!((cosine(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]) - 1.0).abs() < 1e-6);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 0.0]), 0.0);
    }

    /// Runs only where the model is cached: text cut by the endpoint's truncator embeds like the in-process model's own
    /// 512-token truncation of the whole text.
    #[test]
    fn truncated_text_embeds_like_in_process_window_when_cached() {
        let cfg = LocalConfig::default();
        let (Some(Ok(t)), Ok(local)) = (Truncator::cached(&cfg), LocalEmbedder::load(&cfg)) else {
            eprintln!("skipped: model not cached");
            return;
        };
        let long = format!(
            "Deploy notes\n\n{}",
            "The café's `lampl serve` on /mnt/tank/apps restarted twice after the pgvector upgrade; ".repeat(120)
        );
        let cut = t.cut(&long).unwrap();
        assert!(cut.len() < long.len());
        let v = local.embed(&[long.as_str(), cut]).unwrap();
        let parity = cosine(&v[0], &v[1]);
        let whole = LocalEmbedder::load(&LocalConfig {
            max_length: 8192,
            ..cfg
        })
        .unwrap()
        .embed(&[long.as_str()])
        .unwrap();
        eprintln!(
            "cut vs in-process: {parity:.5}; whole text vs in-process: {:.5}",
            cosine(&v[0], &whole[0])
        );
        assert!(parity > 0.999, "cosine {parity}");
    }

    #[test]
    fn missing_weights_is_a_typed_error_and_never_downloads() {
        let dir = std::env::temp_dir().join(format!("lampl-embed-missing-{}", std::process::id()));
        let cfg = LocalConfig {
            cache_dir: dir.clone(),
            ..LocalConfig::default()
        };
        match LocalEmbedder::load(&cfg) {
            Err(EmbedError::ModelMissing { cache_dir }) => assert_eq!(cache_dir, dir),
            other => panic!("expected ModelMissing, got {:?}", other.map(|_| ())),
        }
        assert!(
            !dir.exists(),
            "the embed path must not create or fill the cache"
        );
    }
}
