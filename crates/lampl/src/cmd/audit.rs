// SPDX-License-Identifier: Apache-2.0

//! Read-only audit over the server's export: duplicate, stale and untagged memories, superseded ones excluded.

use std::collections::{BTreeMap, HashSet};
use std::io::BufRead;

use chrono::{DateTime, Utc};
use clap::Args as ClapArgs;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::server;

#[derive(ClapArgs)]
pub(crate) struct AuditArgs {
    /// Machine-readable report
    #[arg(long)]
    json: bool,
    /// A memory not updated for this many days is stale
    #[arg(long, default_value_t = 365)]
    stale_days: i64,
    #[arg(long)]
    project: Option<String>,
}

fn normalise(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub(crate) fn audit(args: &AuditArgs) -> Result<(), String> {
    let client = server("audit")?;
    let mut path = "/v1/export?vectors=false".to_owned();
    if let Some(p) = &args.project {
        path.push_str("&project=");
        path.push_str(
            &p.replace('%', "%25")
                .replace('&', "%26")
                .replace(' ', "%20"),
        );
    }
    let mut memories = Vec::new();
    let mut superseded = HashSet::new();
    let mut ended = false;
    for line in std::io::BufReader::new(client.stream(&path)?).lines() {
        let line = line.map_err(|e| format!("export stream broke: {e}"))?;
        let v: Value = serde_json::from_str(&line).map_err(|e| format!("bad export line: {e}"))?;
        match v["kind"].as_str() {
            Some("memory") => memories.push(v),
            Some("link") if v["link_type"] == "SUPERSEDES" => {
                superseded.insert(v["to_id"].as_str().unwrap_or_default().to_owned());
            }
            Some("end") => ended = true,
            _ => {}
        }
    }
    if !ended {
        return Err("export ended without an end marker; the audit would be incomplete".to_owned());
    }
    memories.retain(|m| !superseded.contains(m["id"].as_str().unwrap_or_default()));
    let now = Utc::now();
    let mut by_title: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    let mut by_body: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    let (mut stale, mut untagged) = (Vec::new(), Vec::new());
    for m in &memories {
        let project = m["project"].as_str().unwrap_or_default();
        by_title
            .entry(format!(
                "{project}\u{0}{}",
                normalise(m["title"].as_str().unwrap_or_default())
            ))
            .or_default()
            .push(m);
        let digest: String =
            Sha256::digest(normalise(m["body"].as_str().unwrap_or_default()).as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
        by_body
            .entry(format!("{project}\u{0}{digest}"))
            .or_default()
            .push(m);
        let age = m["updated_at"]
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| (now - t.with_timezone(&Utc)).num_days());
        if let Some(age) = age.filter(|a| *a >= args.stale_days) {
            stale.push(json!({"id": m["id"], "title": m["title"], "age_days": age}));
        }
        if m["tags"].as_array().is_none_or(Vec::is_empty) {
            untagged.push(json!({"id": m["id"], "title": m["title"]}));
        }
    }
    let mut duplicates = Vec::new();
    for (reason, groups) in [("same title", by_title), ("same body", by_body)] {
        for group in groups.values().filter(|g| g.len() > 1) {
            duplicates.push(json!({"reason": reason, "ids": group.iter().map(|m| m["id"].clone()).collect::<Vec<_>>(), "title": group[0]["title"]}));
        }
    }
    let report = json!({
        "memories_audited": memories.len(),
        "superseded_excluded": superseded.len(),
        "duplicates": duplicates,
        "stale": stale,
        "untagged": untagged,
    });
    if args.json {
        println!("{report}");
    } else {
        println!(
            "audited {} memories ({} superseded excluded)",
            memories.len(),
            superseded.len()
        );
        println!("duplicates: {} group(s)", duplicates.len());
        for d in &duplicates {
            println!(
                "  {} [{}]: {}",
                d["reason"].as_str().unwrap_or(""),
                d["ids"],
                d["title"].as_str().unwrap_or("")
            );
        }
        println!("stale (>= {} days): {}", args.stale_days, stale.len());
        println!("untagged: {}", untagged.len());
    }
    Ok(())
}
