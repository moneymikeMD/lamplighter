// SPDX-License-Identifier: Apache-2.0

//! lamplighter arm: drives `lampl serve` over its HTTP API, one project per question.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use ureq::http::Response;
use ureq::{Agent, Body};

use crate::dataset::{Question, Turn};
use crate::retriever::{Ranked, Retriever};

const TAG_PREFIX: &str = "session:";
const RECALL_LIMIT: usize = 100;

#[derive(Debug, Clone)]
pub(crate) struct Options {
    pub(crate) base: String,
    pub(crate) allow_nonempty: bool,
    pub(crate) allow_full_text: bool,
    pub(crate) wait: Duration,
}

pub(crate) struct Lamplighter {
    opts: Options,
    key: String,
    agent: Agent,
    run: String,
    pub(crate) info: Value,
}

impl std::fmt::Debug for Lamplighter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lamplighter")
            .field("run", &self.run)
            .finish_non_exhaustive()
    }
}

fn read(mut resp: Response<Body>) -> Result<Value, String> {
    let status = resp.status();
    let body = resp
        .body_mut()
        .read_json::<Value>()
        .map_err(|e| format!("unreadable server response ({status}): {e}"))?;
    if status.is_success() {
        Ok(body)
    } else {
        Err(format!(
            "server answered {status}: {}",
            body["error"].as_str().unwrap_or("no message")
        ))
    }
}

/// One round is a user turn and the assistant turns that follow it; this is the retrieval unit, mapped back to its session.
fn rounds(turns: &[Turn]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in turns {
        let line = format!("{}: {}", t.role, t.content);
        match out.last_mut() {
            Some(last) if t.role != "user" => {
                last.push('\n');
                last.push_str(&line);
            }
            _ => out.push(line),
        }
    }
    out
}

impl Lamplighter {
    pub(crate) fn connect(opts: Options, key: String) -> Result<Lamplighter, String> {
        let agent: Agent = Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(120)))
            .http_status_as_error(false)
            .build()
            .into();
        let mut me = Lamplighter {
            opts,
            key,
            agent,
            run: format!("{:x}", std::process::id() as u64 ^ epoch_millis()),
            info: Value::Null,
        };
        let health = me.health()?;
        if health["embedder_available"] != json!(true) && !me.opts.allow_full_text {
            return Err(
                "the server's embedder is unavailable, so only full-text would be measured; fix it or pass --allow-full-text"
                    .to_owned(),
            );
        }
        let count = health["memory_count"].as_u64().unwrap_or(0);
        if count > 0 && !me.opts.allow_nonempty {
            return Err(format!(
                "the store already holds {count} memories; the benchmark writes thousands and cannot delete them, so point it at a scratch store (or pass --allow-nonempty)"
            ));
        }
        me.info = json!({
            "embed_model": health["embed_model"],
            "embed_digest": health["embed_digest"],
            "embed_mode": health["embed_mode"],
            "embedder_available": health["embedder_available"],
            "api_version": health["api_version"],
            "schema_version": health["schema_version"],
        });
        Ok(me)
    }

    fn health(&self) -> Result<Value, String> {
        let resp = self
            .agent
            .get(format!("{}/health", self.opts.base))
            .header("Authorization", format!("Bearer {}", self.key))
            .call()
            .map_err(|e| format!("server is unreachable: {e}"))?;
        if resp.headers().get("x-lampl-api-version").is_none() {
            return Err(
                "the server did not send X-Lampl-Api-Version; it is not a lampl server".to_owned(),
            );
        }
        if resp.status().as_u16() == 401 {
            return Err("the server rejected the API key (401)".to_owned());
        }
        read(resp)
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        let resp = self
            .agent
            .post(format!("{}{path}", self.opts.base))
            .header("Authorization", format!("Bearer {}", self.key))
            .send_json(body)
            .map_err(|e| format!("POST {path} failed: {e}"))?;
        read(resp)
    }

    fn project(&self, q: &Question) -> String {
        format!("lme-{}-{}", self.run, q.id)
    }
}

fn epoch_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl Retriever for Lamplighter {
    fn name(&self) -> &str {
        "lamplighter"
    }

    fn ingest(&mut self, q: &Question) -> Result<usize, String> {
        let project = self.project(q);
        let mut stored = 0;
        for (sid, turns) in q.session_ids.iter().zip(&q.sessions) {
            for (n, body) in rounds(turns).into_iter().enumerate() {
                self.post(
                    "/v1/memories",
                    &json!({
                        "memory_type": "fact",
                        "title": format!("{sid} round {n}"),
                        "body": body,
                        "project": project,
                        "tags": [format!("{TAG_PREFIX}{sid}")],
                    }),
                )?;
                stored += 1;
            }
        }
        Ok(stored)
    }

    fn settle(&mut self) -> Result<(), String> {
        if self.info["embedder_available"] != json!(true) {
            return Ok(());
        }
        let start = Instant::now();
        let mut last_report = Instant::now();
        loop {
            let h = self.health()?;
            let (total, done) = (
                h["memory_count"].as_u64().unwrap_or(0),
                h["embedded_count"].as_u64().unwrap_or(0),
            );
            if h["embedder_available"] != json!(true) {
                return Err("the embedder went away while waiting for vectors".to_owned());
            }
            if done >= total {
                return Ok(());
            }
            if start.elapsed() > self.opts.wait {
                return Err(format!(
                    "timed out after {:?} with {done} of {total} memories embedded",
                    self.opts.wait
                ));
            }
            if last_report.elapsed() > Duration::from_secs(30) {
                eprintln!("embedding: {done}/{total}");
                last_report = Instant::now();
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    fn rank(&mut self, q: &Question) -> Result<Ranked, String> {
        let body = self.post(
            "/v1/recall",
            &json!({"query": q.question, "project": self.project(q), "limit": RECALL_LIMIT}),
        )?;
        let mut sessions: Vec<String> = Vec::new();
        for hit in body["hits"].as_array().into_iter().flatten() {
            let tags = hit["memory"]["tags"].as_array().into_iter().flatten();
            for sid in tags
                .filter_map(Value::as_str)
                .filter_map(|t| t.strip_prefix(TAG_PREFIX))
            {
                if !sessions.iter().any(|s| s == sid) {
                    sessions.push(sid.to_owned());
                }
            }
        }
        Ok(Ranked {
            sessions,
            mode: body["mode"].as_str().unwrap_or("unknown").to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: &str, content: &str) -> Turn {
        Turn {
            role: role.to_owned(),
            content: content.to_owned(),
        }
    }

    #[test]
    fn rounds_group_assistant_turns_under_their_user_turn() {
        let r = rounds(&[
            turn("user", "q1"),
            turn("assistant", "a1"),
            turn("user", "q2"),
            turn("assistant", "a2"),
            turn("assistant", "a2b"),
        ]);
        assert_eq!(
            r,
            [
                "user: q1\nassistant: a1",
                "user: q2\nassistant: a2\nassistant: a2b"
            ]
        );
    }

    #[test]
    fn leading_assistant_turn_still_forms_a_round() {
        assert_eq!(rounds(&[turn("assistant", "hi")]), ["assistant: hi"]);
    }
}
