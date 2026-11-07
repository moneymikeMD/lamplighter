// SPDX-License-Identifier: Apache-2.0

//! In-process Qwen3-Embedding-0.6B on candle, CPU only. `prepare` is the only code that touches the network.

use std::path::{Path, PathBuf};

use candle_core::{DType, Device};
use candle_nn::VarBuilder;
use fastembed::{Qwen3Config, Qwen3Model, Qwen3TextEmbedding};
use hf_hub::{Cache, Repo, api::sync::ApiBuilder};
use tokenizers::{PaddingDirection, PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

use crate::{DEFAULT_REPO_ID, DIM, EmbedError, Embedder};

const FILES: [&str; 3] = ["config.json", "tokenizer.json", "model.safetensors"];

#[derive(Debug, Clone)]
pub struct LocalConfig {
    pub cache_dir: PathBuf,
    pub repo_id: String,
    pub max_length: usize,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            cache_dir: default_cache_dir(),
            repo_id: DEFAULT_REPO_ID.to_string(),
            max_length: 512,
        }
    }
}

pub fn default_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("lamplighter")
        .join("models")
}

/// Downloads the model files from Hugging Face into `cfg.cache_dir`. This is the only network call in the crate's local path; hooks must never call it.
pub fn prepare(cfg: &LocalConfig) -> Result<(), EmbedError> {
    let api = ApiBuilder::new()
        .with_cache_dir(cfg.cache_dir.clone())
        .with_progress(true)
        .build()
        .map_err(|e| EmbedError::Download(e.to_string()))?;
    let repo = api.model(cfg.repo_id.clone());
    for file in FILES {
        repo.get(file)
            .map_err(|e| EmbedError::Download(format!("{file}: {e}")))?;
    }
    Ok(())
}

pub struct LocalEmbedder {
    inner: Qwen3TextEmbedding,
    repo_id: String,
    digest: String,
}

impl std::fmt::Debug for LocalEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalEmbedder")
            .field("repo_id", &self.repo_id)
            .field("digest", &self.digest)
            .finish_non_exhaustive()
    }
}

impl LocalEmbedder {
    /// Loads from the local cache only; returns `ModelMissing` if any file is absent and never opens a socket.
    pub fn load(cfg: &LocalConfig) -> Result<Self, EmbedError> {
        let missing = || EmbedError::ModelMissing {
            cache_dir: cfg.cache_dir.clone(),
        };
        let cache = Cache::new(cfg.cache_dir.clone()).repo(Repo::model(cfg.repo_id.clone()));
        let config_path = cache.get(FILES[0]).ok_or_else(missing)?;
        let tokenizer_path = cache.get(FILES[1]).ok_or_else(missing)?;
        let weights_path = cache.get(FILES[2]).ok_or_else(missing)?;

        let model_cfg: Qwen3Config =
            serde_json::from_slice(&std::fs::read(&config_path).map_err(load_err)?)
                .map_err(load_err)?;
        let device = Device::Cpu;
        // candle's safetensors loader mmaps the weights; the cache file must not be modified while loaded.
        #[allow(unsafe_code)]
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(
                std::slice::from_ref(&weights_path),
                DType::F32,
                &device,
            )
        }
        .map_err(load_err)?;
        let model = Qwen3Model::new(model_cfg, vb).map_err(load_err)?;
        let tokenizer = tokenizer(&tokenizer_path, cfg.max_length)?;
        let inner = Qwen3TextEmbedding::new(model, tokenizer);
        if inner.output_dim() != DIM {
            return Err(EmbedError::DimMismatch {
                expected: DIM,
                got: inner.output_dim(),
            });
        }
        Ok(Self {
            inner,
            repo_id: cfg.repo_id.clone(),
            digest: snapshot_digest(&weights_path),
        })
    }
}

impl Embedder for LocalEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.inner
            .embed(texts)
            .map_err(|e| EmbedError::Embed(e.to_string()))
    }

    fn dim(&self) -> usize {
        self.inner.output_dim()
    }

    fn model_id(&self) -> &str {
        &self.repo_id
    }

    fn model_digest(&self) -> &str {
        &self.digest
    }
}

fn load_err<E: std::fmt::Display>(e: E) -> EmbedError {
    EmbedError::Load(e.to_string())
}

fn snapshot_digest(weights: &Path) -> String {
    weights
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn tokenizer(path: &Path, max_length: usize) -> Result<Tokenizer, EmbedError> {
    let mut tok = Tokenizer::from_file(path).map_err(load_err)?;
    tok.with_padding(Some(PaddingParams {
        strategy: PaddingStrategy::BatchLongest,
        direction: PaddingDirection::Left,
        ..Default::default()
    }));
    tok.with_truncation(Some(TruncationParams {
        max_length,
        ..Default::default()
    }))
    .map_err(load_err)?;
    Ok(tok)
}
