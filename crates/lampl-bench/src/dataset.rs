// SPDX-License-Identifier: Apache-2.0

//! LongMemEval download (never bundled), hash check and streaming load.

use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::de::{DeserializeOwned, Deserializer, SeqAccess, Visitor};
use sha2::{Digest, Sha256};

pub(crate) const REVISION: &str = "98d7416c24c778c2fee6e6f3006e7a073259d48f";
pub(crate) const FILE: &str = "longmemeval_s_cleaned.json";
pub(crate) const SHA256: &str = "d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442";

#[derive(Debug, Deserialize)]
pub(crate) struct Head {
    #[serde(rename = "question_id")]
    pub(crate) id: String,
    #[serde(rename = "question_type")]
    pub(crate) kind: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Turn {
    pub(crate) role: String,
    pub(crate) content: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Question {
    #[serde(rename = "question_id")]
    pub(crate) id: String,
    #[serde(rename = "question_type")]
    pub(crate) kind: String,
    pub(crate) question: String,
    pub(crate) answer_session_ids: Vec<String>,
    #[serde(rename = "haystack_session_ids")]
    pub(crate) session_ids: Vec<String>,
    #[serde(rename = "haystack_sessions")]
    pub(crate) sessions: Vec<Vec<Turn>>,
}

pub(crate) fn default_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("lamplighter")
        .join("bench")
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f = File::open(path).map_err(|e| format!("opening {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Returns the cached dataset file, downloading the pinned revision first when it is absent or its hash differs.
pub(crate) fn ensure(cache_dir: &Path) -> Result<PathBuf, String> {
    let path = cache_dir.join(format!("{REVISION}-{FILE}"));
    if path.exists() && sha256_file(&path)? == SHA256 {
        return Ok(path);
    }
    std::fs::create_dir_all(cache_dir)
        .map_err(|e| format!("creating {}: {e}", cache_dir.display()))?;
    let url = format!(
        "https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/{REVISION}/{FILE}"
    );
    eprintln!("downloading {FILE} at {} (277 MB)", &REVISION[..8]);
    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("downloading dataset: {e}"))?;
    let tmp = path.with_extension("part");
    let mut out = File::create(&tmp).map_err(|e| format!("creating {}: {e}", tmp.display()))?;
    let mut body = resp.into_body();
    let mut reader = body.as_reader();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("downloading dataset: {e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    }
    drop(out);
    let got = sha256_file(&tmp)?;
    if got != SHA256 {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "dataset hash mismatch: expected {SHA256}, got {got}"
        ));
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("moving dataset into place: {e}"))?;
    Ok(path)
}

struct Filtered<T, F>(F, std::marker::PhantomData<T>);

impl<'de, T: DeserializeOwned, F: FnMut(T)> Visitor<'de> for Filtered<T, F> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array of questions")
    }

    fn visit_seq<A: SeqAccess<'de>>(mut self, mut seq: A) -> Result<(), A::Error> {
        while let Some(item) = seq.next_element::<T>()? {
            (self.0)(item);
        }
        Ok(())
    }
}

fn stream<T: DeserializeOwned>(path: &Path, sink: impl FnMut(T)) -> Result<(), String> {
    let f = File::open(path).map_err(|e| format!("opening {}: {e}", path.display()))?;
    let mut de = serde_json::Deserializer::from_reader(BufReader::new(f));
    de.deserialize_seq(Filtered(sink, std::marker::PhantomData))
        .map_err(|e| format!("parsing {}: {e}", path.display()))
}

pub(crate) fn heads(path: &Path) -> Result<Vec<Head>, String> {
    let mut out = Vec::new();
    stream::<Head>(path, |h| out.push(h))?;
    Ok(out)
}

/// Loads only the named questions, in file order, so memory stays proportional to the subset.
pub(crate) fn load(path: &Path, ids: &HashSet<String>) -> Result<Vec<Question>, String> {
    let mut out = Vec::new();
    stream::<Question>(path, |q| {
        if ids.contains(&q.id) {
            out.push(q);
        }
    })?;
    Ok(out)
}
