// SPDX-License-Identifier: Apache-2.0

//! Optional HTTP embedder: OpenAI-compatible `/v1/embeddings` or Ollama `/api/embed`.

use serde_json::{Value, json};

use crate::truncate::{FALLBACK_MAX_CHARS, Truncator};
use crate::{DIM, EmbedError, Embedder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    OpenAi,
    Ollama,
}

pub struct EndpointEmbedder {
    kind: EndpointKind,
    base_url: String,
    model: String,
    api_key: Option<String>,
    truncator: Truncator,
    agent: ureq::Agent,
}

impl std::fmt::Debug for EndpointEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EndpointEmbedder")
            .field("kind", &self.kind)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("truncator", &self.truncator)
            .finish_non_exhaustive()
    }
}

impl EndpointEmbedder {
    /// Truncates input to `FALLBACK_MAX_CHARS` characters until `with_truncator` gives it the model's tokenizer.
    pub fn new(kind: EndpointKind, base_url: &str, model: &str, api_key: Option<String>) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(60)))
            .build()
            .into();
        Self {
            kind,
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            api_key,
            truncator: Truncator::chars(FALLBACK_MAX_CHARS),
            agent,
        }
    }

    #[must_use]
    pub fn with_truncator(mut self, truncator: Truncator) -> Self {
        self.truncator = truncator;
        self
    }

    pub fn truncator(&self) -> &Truncator {
        &self.truncator
    }

    fn url(&self) -> String {
        match self.kind {
            EndpointKind::OpenAi => format!("{}/v1/embeddings", self.base_url),
            EndpointKind::Ollama => format!("{}/api/embed", self.base_url),
        }
    }
}

impl Embedder for EndpointEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let mut req = self.agent.post(&self.url());
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", &format!("Bearer {key}"));
        }
        let input = texts
            .iter()
            .map(|t| self.truncator.cut(t))
            .collect::<Result<Vec<&str>, _>>()?;
        let mut resp = req
            .send_json(json!({ "model": self.model, "input": input }))
            .map_err(|e| EmbedError::Endpoint(e.to_string()))?;
        let body: Value = resp
            .body_mut()
            .read_json()
            .map_err(|e| EmbedError::Endpoint(e.to_string()))?;
        parse_vectors(self.kind, &body, texts.len())
    }

    fn dim(&self) -> usize {
        DIM
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn model_digest(&self) -> &str {
        "endpoint"
    }
}

fn parse_vectors(
    kind: EndpointKind,
    body: &Value,
    expected: usize,
) -> Result<Vec<Vec<f32>>, EmbedError> {
    let rows: Vec<&Value> = match kind {
        EndpointKind::OpenAi => body["data"]
            .as_array()
            .map(|a| a.iter().map(|d| &d["embedding"]).collect()),
        EndpointKind::Ollama => body["embeddings"].as_array().map(|a| a.iter().collect()),
    }
    .ok_or_else(|| EmbedError::Endpoint("response has no embeddings array".into()))?;
    if rows.len() != expected {
        return Err(EmbedError::Endpoint(format!(
            "expected {expected} vectors, got {}",
            rows.len()
        )));
    }
    rows.into_iter()
        .map(|row| {
            let v: Vec<f32> = row
                .as_array()
                .ok_or_else(|| EmbedError::Endpoint("embedding is not an array".into()))?
                .iter()
                .map(|x| {
                    x.as_f64()
                        .map(|f| f as f32)
                        .ok_or_else(|| EmbedError::Endpoint("non-numeric embedding value".into()))
                })
                .collect::<Result<_, _>>()?;
            if v.len() != DIM {
                return Err(EmbedError::DimMismatch {
                    expected: DIM,
                    got: v.len(),
                });
            }
            Ok(v)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    fn serve_once(body: String) -> String {
        serve_capture(body).0
    }

    fn serve_capture(body: String) -> (String, std::sync::mpsc::Receiver<String>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = s.read(&mut chunk).unwrap();
                req.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&req).to_lowercase();
                if let Some(head_end) = text.find("\r\n\r\n") {
                    let len = text
                        .split("content-length:")
                        .nth(1)
                        .and_then(|r| r.split("\r\n").next())
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if req.len() >= head_end + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let raw = String::from_utf8_lossy(&req).into_owned();
            let _ = tx.send(raw.split("\r\n\r\n").nth(1).unwrap_or_default().to_owned());
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
        });
        (format!("http://{addr}"), rx)
    }

    fn vec_of(n: usize) -> Value {
        json!(vec![0.5f64; n])
    }

    #[test]
    fn ollama_returns_1024_dim_vectors() {
        let url = serve_once(json!({ "embeddings": [vec_of(DIM), vec_of(DIM)] }).to_string());
        let e = EndpointEmbedder::new(EndpointKind::Ollama, &url, "qwen3-embedding:0.6b", None);
        let out = e.embed(&["a", "b"]).unwrap();
        assert_eq!((out.len(), out[0].len()), (2, DIM));
    }

    #[test]
    fn openai_returns_1024_dim_vectors() {
        let url = serve_once(json!({ "data": [{ "embedding": vec_of(DIM) }] }).to_string());
        let e = EndpointEmbedder::new(EndpointKind::OpenAi, &url, "m", Some("k".into()));
        assert_eq!(e.embed(&["a"]).unwrap()[0].len(), DIM);
    }

    #[test]
    fn wrong_dimension_is_a_typed_error() {
        let url = serve_once(json!({ "embeddings": [vec_of(768)] }).to_string());
        let e = EndpointEmbedder::new(EndpointKind::Ollama, &url, "m", None);
        assert!(matches!(
            e.embed(&["a"]),
            Err(EmbedError::DimMismatch {
                expected: 1024,
                got: 768
            })
        ));
    }

    #[test]
    fn long_input_is_truncated_before_the_request() {
        let (url, sent) =
            serve_capture(json!({ "embeddings": [vec_of(DIM), vec_of(DIM)] }).to_string());
        let e = EndpointEmbedder::new(EndpointKind::Ollama, &url, "m", None);
        assert_eq!(
            e.truncator().describe(),
            format!("chars:{FALLBACK_MAX_CHARS}")
        );
        let long = "x".repeat(FALLBACK_MAX_CHARS * 3);
        e.embed(&[long.as_str(), "short"]).unwrap();
        let body: Value = serde_json::from_str(&sent.recv().unwrap()).unwrap();
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[0].as_str().unwrap().len(), FALLBACK_MAX_CHARS);
        assert_eq!(input[1], "short");
    }
}
