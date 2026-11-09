// SPDX-License-Identifier: Apache-2.0

//! HTTP client for `lampl serve`. Every failure is an error with a message; nothing here returns an empty result in place of one.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use ureq::http::Response;
use ureq::{Agent, Body};

use crate::local::config::{self, Config};
use lampl::serve::{API_VERSION, API_VERSION_HEADER};

const SHORT: Duration = Duration::from_secs(30);
const BULK: Duration = Duration::from_secs(600);

pub(crate) enum Target {
    Local,
    Server(ServerSettings),
}

pub(crate) struct ServerSettings {
    pub(crate) urls: Vec<String>,
    key_file: Option<PathBuf>,
}

/// `LAMPL_SERVER` (comma-separated URL list) wins over the config file; with neither, the client is not configured.
pub(crate) fn target() -> Result<Target, String> {
    if let Some(v) = std::env::var("LAMPL_SERVER")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        let urls = split_urls(&v);
        if urls.is_empty() {
            return Err("LAMPL_SERVER holds no URL".to_owned());
        }
        return Ok(Target::Server(ServerSettings {
            urls,
            key_file: config::load().ok().and_then(|c| match c {
                Config::Server { api_key_file, .. } => api_key_file,
                Config::Local { .. } => None,
            }),
        }));
    }
    match config::load()? {
        Config::Local { .. } => Ok(Target::Local),
        Config::Server {
            url,
            fallback_urls,
            api_key_file,
        } => {
            let mut urls = vec![url];
            urls.extend(fallback_urls);
            Ok(Target::Server(ServerSettings {
                urls,
                key_file: api_key_file,
            }))
        }
    }
}

fn split_urls(v: &str) -> Vec<String> {
    v.split(',')
        .map(|u| u.trim().trim_end_matches('/').to_owned())
        .filter(|u| !u.is_empty())
        .collect()
}

fn api_key(settings: &ServerSettings) -> Result<String, String> {
    if let Some(k) = std::env::var("LAMPL_API_KEY")
        .ok()
        .map(|k| k.trim().to_owned())
        .filter(|k| !k.is_empty())
    {
        return Ok(k);
    }
    if let Some(path) = &settings.key_file {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading api_key_file {}: {e}", path.display()))?;
        let key = text.trim().to_owned();
        if key.is_empty() {
            return Err(format!("api_key_file {} is empty", path.display()));
        }
        return Ok(key);
    }
    Err("no API key: set LAMPL_API_KEY or `api_key_file` in config.toml".to_owned())
}

fn agent(timeout: Duration) -> Agent {
    Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(5)))
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .build()
        .into()
}

pub(crate) struct Client {
    pub(crate) base: String,
    key: String,
    short: Agent,
    bulk: Agent,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

fn check_version(base: &str, resp: &Response<Body>) -> Result<(), String> {
    let seen = resp
        .headers()
        .get(API_VERSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or_else(|| {
            format!("{base} did not send {API_VERSION_HEADER}; it is not a lampl server")
        })?;
    if seen == API_VERSION {
        Ok(())
    } else {
        Err(format!(
            "API version mismatch: {base} speaks version {seen}, this client speaks {API_VERSION}; upgrade the older side"
        ))
    }
}

impl Client {
    /// Picks the first reachable URL, verifies it is a compatible lampl server, then requires an API key.
    pub(crate) fn connect(settings: &ServerSettings) -> Result<Client, String> {
        let short = agent(SHORT);
        let mut failures = Vec::new();
        for base in &settings.urls {
            match short.get(format!("{base}/livez")).call() {
                Ok(resp) => {
                    check_version(base, &resp)?;
                    if !failures.is_empty() {
                        eprintln!("lampl: {}; using {base}", failures.join("; "));
                    }
                    return Ok(Client {
                        base: base.clone(),
                        key: api_key(settings)?,
                        short,
                        bulk: agent(BULK),
                    });
                }
                Err(e) => failures.push(format!("server {base} is unreachable ({e})")),
            }
        }
        Err(format!(
            "{}; not falling back to local mode",
            failures.join("; ")
        ))
    }

    fn fail(&self, resp: &mut Response<Body>) -> String {
        let status = resp.status();
        if status.as_u16() == 401 {
            return format!("server {} rejected the API key (401)", self.base);
        }
        let msg = resp
            .body_mut()
            .read_json::<Value>()
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_owned))
            .unwrap_or_default();
        format!("server {} answered {status}: {msg}", self.base)
    }

    fn finish(&self, mut resp: Response<Body>) -> Result<Response<Body>, String> {
        check_version(&self.base, &resp)?;
        if resp.status().is_success() {
            Ok(resp)
        } else {
            Err(self.fail(&mut resp))
        }
    }

    fn auth(&self) -> String {
        format!("Bearer {}", self.key)
    }

    pub(crate) fn get(&self, path: &str) -> Result<Value, String> {
        let resp = self
            .short
            .get(format!("{}{path}", self.base))
            .header("Authorization", self.auth())
            .call()
            .map_err(|e| self.transport(&e))?;
        read_json(self.finish(resp)?)
    }

    pub(crate) fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        let resp = self
            .short
            .post(format!("{}{path}", self.base))
            .header("Authorization", self.auth())
            .send_json(body)
            .map_err(|e| self.transport(&e))?;
        read_json(self.finish(resp)?)
    }

    pub(crate) fn delete(&self, path: &str) -> Result<Value, String> {
        let resp = self
            .short
            .delete(format!("{}{path}", self.base))
            .header("Authorization", self.auth())
            .call()
            .map_err(|e| self.transport(&e))?;
        read_json(self.finish(resp)?)
    }

    /// Streams an NDJSON body from the server.
    pub(crate) fn stream(&self, path: &str) -> Result<Box<dyn std::io::Read + Send>, String> {
        let resp = self
            .bulk
            .get(format!("{}{path}", self.base))
            .header("Authorization", self.auth())
            .call()
            .map_err(|e| self.transport(&e))?;
        Ok(Box::new(self.finish(resp)?.into_body().into_reader()))
    }

    pub(crate) fn post_ndjson(&self, path: &str, body: &[u8]) -> Result<Value, String> {
        let resp = self
            .bulk
            .post(format!("{}{path}", self.base))
            .header("Authorization", self.auth())
            .content_type("application/x-ndjson")
            .send(body)
            .map_err(|e| self.transport(&e))?;
        read_json(self.finish(resp)?)
    }

    fn transport(&self, e: &ureq::Error) -> String {
        format!(
            "server {} failed mid-request ({e}); not falling back to local mode",
            self.base
        )
    }
}

fn read_json(mut resp: Response<Body>) -> Result<Value, String> {
    resp.body_mut()
        .with_config()
        .limit(256 * 1024 * 1024)
        .read_json::<Value>()
        .map_err(|e| format!("unreadable server response: {e}"))
}
