// SPDX-License-Identifier: Apache-2.0

//! Per-client API keys: a keys file of `name sha256-hex` lines, reloaded when its mtime changes so a revocation needs no restart.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};

use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

use super::AppState;
use super::error::ApiError;

const KEY_PREFIX: &str = "lampl_";
const RECHECK: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("cannot read keys file {}: {source}", .path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("keys file line {line}: expected `<name> <64 hex chars of sha256>`")]
    Malformed { line: usize },
    #[error("keys file {} holds no keys; refusing to serve without authentication", .path.display())]
    Empty { path: PathBuf },
    #[error("client name must be non-empty, contain no whitespace and not start with `#`")]
    BadName,
    #[error("a key for `{0}` already exists; revoke it first")]
    Exists(String),
    #[error("no key for `{0}`")]
    Unknown(String),
    #[error("system randomness unavailable: {0}")]
    Random(String),
}

struct Loaded {
    mtime: Option<SystemTime>,
    checked: Instant,
    by_hash: HashMap<String, String>,
}

pub struct KeyStore {
    path: PathBuf,
    inner: RwLock<Loaded>,
}

impl std::fmt::Debug for KeyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// The hex SHA-256 stored in the keys file for a presented key.
pub fn hash_key(key: &str) -> String {
    hex(&Sha256::digest(key.as_bytes()))
}

/// Appends a new client key to the keys file (created owner-only if absent) and returns the key, shown once.
pub fn add_key(path: &Path, name: &str) -> Result<String, KeyError> {
    if name.is_empty() || name.chars().any(char::is_whitespace) || name.starts_with('#') {
        return Err(KeyError::BadName);
    }
    let mut lines = read_lines(path)?;
    if parse_names(&lines)?.iter().any(|n| n == name) {
        return Err(KeyError::Exists(name.to_owned()));
    }
    let key = generate_key()?;
    lines.push(format!("{name} {}", hash_key(&key)));
    write_lines(path, &lines)?;
    Ok(key)
}

/// Client names in the keys file.
pub fn list_keys(path: &Path) -> Result<Vec<String>, KeyError> {
    parse_names(&read_lines(path)?)
}

/// Removes a client's key; the running server stops accepting it within a second.
pub fn revoke_key(path: &Path, name: &str) -> Result<(), KeyError> {
    let lines = read_lines(path)?;
    let kept: Vec<String> = lines
        .iter()
        .filter(|l| l.split_whitespace().next() != Some(name) || l.trim_start().starts_with('#'))
        .cloned()
        .collect();
    if kept.len() == lines.len() {
        return Err(KeyError::Unknown(name.to_owned()));
    }
    write_lines(path, &kept)
}

fn read_lines(path: &Path) -> Result<Vec<String>, KeyError> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(t.lines().map(str::to_owned).collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(source) => Err(KeyError::Read {
            path: path.to_owned(),
            source,
        }),
    }
}

fn parse_names(lines: &[String]) -> Result<Vec<String>, KeyError> {
    let by_hash = parse(&lines.join("\n"))?;
    let mut names: Vec<String> = by_hash.into_values().collect();
    names.sort();
    Ok(names)
}

fn write_lines(path: &Path, lines: &[String]) -> Result<(), KeyError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let io = |source| KeyError::Read {
        path: path.to_owned(),
        source,
    };
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(io)?;
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    f.write_all(text.as_bytes()).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

/// A fresh 256-bit key; the caller shows it once and stores only `hash_key` of it.
pub fn generate_key() -> Result<String, KeyError> {
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| KeyError::Random(e.to_string()))?;
    Ok(format!("{KEY_PREFIX}{}", hex(&raw)))
}

fn parse(text: &str) -> Result<HashMap<String, String>, KeyError> {
    let mut by_hash = HashMap::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(name), Some(hash), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err(KeyError::Malformed { line: i + 1 });
        };
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(KeyError::Malformed { line: i + 1 });
        }
        by_hash.insert(hash.to_ascii_lowercase(), name.to_owned());
    }
    Ok(by_hash)
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl KeyStore {
    pub fn open(path: &Path) -> Result<Self, KeyError> {
        let text = std::fs::read_to_string(path).map_err(|source| KeyError::Read {
            path: path.to_owned(),
            source,
        })?;
        let by_hash = parse(&text)?;
        if by_hash.is_empty() {
            return Err(KeyError::Empty {
                path: path.to_owned(),
            });
        }
        Ok(KeyStore {
            path: path.to_owned(),
            inner: RwLock::new(Loaded {
                mtime: mtime(path),
                checked: Instant::now(),
                by_hash,
            }),
        })
    }

    fn refresh(&self) {
        let due = self
            .inner
            .read()
            .is_ok_and(|l| l.checked.elapsed() >= RECHECK);
        if !due {
            return;
        }
        let Ok(mut loaded) = self.inner.write() else {
            return;
        };
        loaded.checked = Instant::now();
        let current = mtime(&self.path);
        if current == loaded.mtime {
            return;
        }
        match std::fs::read_to_string(&self.path)
            .map_err(|e| e.to_string())
            .and_then(|t| parse(&t).map_err(|e| e.to_string()))
        {
            Ok(by_hash) => {
                loaded.by_hash = by_hash;
                loaded.mtime = current;
                tracing::info!(keys = loaded.by_hash.len(), "keys file reloaded");
            }
            Err(e) => tracing::error!(error = %e, "keys file reload failed; keeping previous keys"),
        }
    }

    /// Returns the client name for a valid key.
    pub fn authenticate(&self, presented: &str) -> Option<String> {
        self.refresh();
        let hash = hash_key(presented);
        self.inner.read().ok()?.by_hash.get(&hash).cloned()
    }
}

pub(crate) async fn require_key(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let presented = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    match presented.and_then(|k| state.keys.authenticate(k)) {
        Some(name) => {
            tracing::debug!(client = %name, path = %req.uri().path(), "authenticated");
            next.run(req).await
        }
        None => {
            let mut resp = ApiError::unauthorized().into_response();
            resp.headers_mut()
                .insert(WWW_AUTHENTICATE, "Bearer".parse().expect("static header"));
            resp
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_comments_and_rejects_bad_lines() {
        let h = hash_key("lampl_x");
        let ok = parse(&format!("# c\n\nmac {h}\n")).unwrap();
        assert_eq!(ok.get(&h).map(String::as_str), Some("mac"));
        assert!(matches!(
            parse("mac nothex"),
            Err(KeyError::Malformed { line: 1 })
        ));
        assert!(matches!(
            parse(&format!("a {h} extra")),
            Err(KeyError::Malformed { line: 1 })
        ));
    }

    #[test]
    fn generated_keys_are_distinct_and_prefixed() {
        let (a, b) = (generate_key().unwrap(), generate_key().unwrap());
        assert_ne!(a, b);
        assert!(a.starts_with("lampl_") && a.len() == 6 + 64);
    }
}
