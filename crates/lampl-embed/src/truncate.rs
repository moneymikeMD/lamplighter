// SPDX-License-Identifier: Apache-2.0

//! Input truncation for the endpoint embedder, so it embeds the same text the in-process model does: the first 512 tokens.

use std::path::{Path, PathBuf};

use hf_hub::{Cache, Repo, api::sync::ApiBuilder};
use tokenizers::{Tokenizer, TruncationParams};

use crate::{EmbedError, LocalConfig};

const TOKENIZER_FILE: &str = "tokenizer.json";

/// Character cap used when no tokenizer is available: about 400 Qwen3 tokens of prose or code, at most about 660 in the
/// measurement in docs/embeddings.md, so it fits an endpoint context of 1,024 tokens but is not exact.
pub const FALLBACK_MAX_CHARS: usize = 1500;

/// Cuts embedder input to the in-process model's window.
pub struct Truncator {
    inner: Inner,
}

enum Inner {
    Tokens(Box<Tokenizer>),
    Chars(usize),
}

impl std::fmt::Debug for Truncator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe())
    }
}

impl Truncator {
    /// Exact truncation with the tokenizer at `path`, keeping what the in-process model keeps at `max_tokens` (special tokens included).
    pub fn from_tokenizer_file(path: &Path, max_tokens: usize) -> Result<Self, EmbedError> {
        let tok = Tokenizer::from_file(path)
            .map_err(|e| EmbedError::Load(format!("{}: {e}", path.display())))?;
        Self::from_tokenizer(tok, max_tokens)
    }

    fn from_tokenizer(mut tok: Tokenizer, max_tokens: usize) -> Result<Self, EmbedError> {
        tok.with_padding(None);
        tok.with_truncation(Some(TruncationParams {
            max_length: max_tokens,
            ..Default::default()
        }))
        .map_err(|e| EmbedError::Load(e.to_string()))?;
        Ok(Self {
            inner: Inner::Tokens(Box::new(tok)),
        })
    }

    /// Approximate truncation to at most `max_chars` characters.
    pub fn chars(max_chars: usize) -> Self {
        Self {
            inner: Inner::Chars(max_chars),
        }
    }

    /// The tokenizer from `cfg`'s model cache if it is there, else `None`; never touches the network.
    pub fn cached(cfg: &LocalConfig) -> Option<Result<Self, EmbedError>> {
        tokenizer_path(cfg).map(|p| Self::from_tokenizer_file(&p, cfg.max_length))
    }

    pub fn is_exact(&self) -> bool {
        matches!(self.inner, Inner::Tokens(_))
    }

    /// `tokens:<n>` or `chars:<n>`, for logs and health output.
    pub fn describe(&self) -> String {
        match &self.inner {
            Inner::Tokens(t) => {
                format!("tokens:{}", t.get_truncation().map_or(0, |p| p.max_length))
            }
            Inner::Chars(n) => format!("chars:{n}"),
        }
    }

    /// The longest prefix of `text` the in-process model would embed; `text` itself when it already fits.
    pub fn cut<'a>(&self, text: &'a str) -> Result<&'a str, EmbedError> {
        let end = match &self.inner {
            Inner::Chars(n) => text.char_indices().nth(*n).map(|(i, _)| i),
            Inner::Tokens(tok) => {
                let enc = tok
                    .encode(text, true)
                    .map_err(|e| EmbedError::Embed(format!("tokenize: {e}")))?;
                if enc.get_overflowing().is_empty() {
                    None
                } else {
                    enc.get_offsets()
                        .iter()
                        .zip(enc.get_special_tokens_mask())
                        .rev()
                        .find(|(_, special)| **special == 0)
                        .map(|((_, end), _)| *end)
                }
            }
        };
        Ok(match end {
            Some(mut e) if e < text.len() => {
                while !text.is_char_boundary(e) {
                    e -= 1;
                }
                &text[..e]
            }
            _ => text,
        })
    }
}

fn tokenizer_path(cfg: &LocalConfig) -> Option<PathBuf> {
    Cache::new(cfg.cache_dir.clone())
        .repo(Repo::model(cfg.repo_id.clone()))
        .get(TOKENIZER_FILE)
}

/// Downloads only `tokenizer.json` (about 11 MB) into `cfg.cache_dir`, or returns the cached copy. An explicit step like `prepare`.
pub fn prepare_tokenizer(cfg: &LocalConfig) -> Result<PathBuf, EmbedError> {
    if let Some(p) = tokenizer_path(cfg) {
        return Ok(p);
    }
    ApiBuilder::new()
        .with_cache_dir(cfg.cache_dir.clone())
        .with_progress(false)
        .build()
        .map_err(|e| EmbedError::Download(e.to_string()))?
        .model(cfg.repo_id.clone())
        .get(TOKENIZER_FILE)
        .map_err(|e| EmbedError::Download(format!("{TOKENIZER_FILE}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word_tokenizer() -> Tokenizer {
        let json = r#"{
            "version": "1.0", "truncation": null, "padding": null, "added_tokens": [
                {"id": 3, "content": "<eos>", "single_word": false, "lstrip": false, "rstrip": false, "normalized": false, "special": true}
            ],
            "normalizer": null,
            "pre_tokenizer": {"type": "Whitespace"},
            "post_processor": {"type": "TemplateProcessing",
                "single": [{"Sequence": {"id": "A", "type_id": 0}}, {"SpecialToken": {"id": "<eos>", "type_id": 0}}],
                "pair": [{"Sequence": {"id": "A", "type_id": 0}}, {"Sequence": {"id": "B", "type_id": 0}}],
                "special_tokens": {"<eos>": {"id": "<eos>", "ids": [3], "tokens": ["<eos>"]}}},
            "decoder": null,
            "model": {"type": "WordLevel", "vocab": {"a": 0, "b": 1, "[UNK]": 2, "<eos>": 3}, "unk_token": "[UNK]"}
        }"#;
        json.parse().unwrap()
    }

    #[test]
    fn tokenizer_cut_keeps_max_minus_special_tokens() {
        let t = Truncator::from_tokenizer(word_tokenizer(), 4).unwrap();
        assert!(t.is_exact());
        assert_eq!(t.describe(), "tokens:4");
        assert_eq!(t.cut("a b a b a").unwrap(), "a b a");
        assert_eq!(t.cut("a b a").unwrap(), "a b a");
        assert_eq!(t.cut("").unwrap(), "");
    }

    #[test]
    fn char_cut_counts_characters_not_bytes() {
        let t = Truncator::chars(3);
        assert!(!t.is_exact());
        assert_eq!(t.cut("ééééé").unwrap(), "ééé");
        assert_eq!(t.cut("ab").unwrap(), "ab");
    }

    #[test]
    fn missing_tokenizer_is_none_and_never_downloads() {
        let dir = std::env::temp_dir().join(format!("lampl-tok-missing-{}", std::process::id()));
        let cfg = LocalConfig {
            cache_dir: dir.clone(),
            ..LocalConfig::default()
        };
        assert!(Truncator::cached(&cfg).is_none());
        assert!(!dir.exists());
    }

    /// Needs the cached tokenizer (`lampl doctor --fetch-model`, run with `--ignored`): the cut text re-tokenizes to the 511
    /// content tokens the in-process model keeps before its end-of-text token.
    #[test]
    #[ignore = "needs the cached Qwen3 model: lampl doctor --fetch-model, then cargo test -p lampl-embed -- --ignored"]
    fn qwen_cut_matches_in_process_window_when_cached() {
        let cfg = LocalConfig::default();
        let t = Truncator::cached(&cfg)
            .expect("Qwen3 tokenizer not cached")
            .unwrap();
        let raw = Tokenizer::from_file(tokenizer_path(&cfg).unwrap()).unwrap();
        let long = "Deploy notes: the café's `lampl serve` on /mnt/tank/apps 🚀 restarted twice; "
            .repeat(200);
        for text in [long.as_str(), &format!("Title\n\n{long}")] {
            let cut = t.cut(text).unwrap();
            let kept = raw.encode(cut, false).unwrap().len();
            assert!((510..=512).contains(&kept), "kept {kept} tokens");
            let full = raw.encode(text, false).unwrap();
            let in_process: Vec<u32> = full.get_ids()[..511].to_vec();
            assert_eq!(
                raw.encode(cut, false).unwrap().get_ids()[..510],
                in_process[..510]
            );
        }
        assert_eq!(t.cut("short").unwrap(), "short");
    }
}
