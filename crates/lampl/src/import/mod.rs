// SPDX-License-Identifier: Apache-2.0

//! Turns lamplighter exports and memorygraph data (JSON export or a direct memory-pg read) into the NDJSON lines `POST /v1/import` takes.

mod postgres;

use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};
use lampl_core::{LinkType, MemoryType};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub(crate) use postgres::read_memory_pg;

const MAPPING: &str = include_str!("mapping.toml");
const ID_NAMESPACE: Uuid = Uuid::from_u128(0x6c61_6d70_6c69_6768_7465_7220_6d67_0001);

#[derive(Debug, Deserialize)]
struct Mapping {
    types: BTreeMap<String, String>,
    links: BTreeMap<String, String>,
}

fn mapping() -> Result<Mapping, String> {
    let m: Mapping = toml::from_str(MAPPING).map_err(|e| format!("mapping.toml: {e}"))?;
    for (from, to) in &m.types {
        to.parse::<MemoryType>()
            .map_err(|e| format!("mapping.toml: type `{from}`: {e}"))?;
    }
    for (from, to) in &m.links {
        if to != "dropped" {
            to.parse::<LinkType>()
                .map_err(|e| format!("mapping.toml: link `{from}`: {e}"))?;
        }
    }
    Ok(m)
}

/// Lines ready to upload plus what the conversion left out.
#[derive(Debug, Default)]
pub(crate) struct Bundle {
    pub(crate) memories: Vec<Value>,
    pub(crate) links: Vec<Value>,
    /// Model id and digest of a lamplighter export's vectors; `None` when the source carries none.
    pub(crate) model: Option<(String, String)>,
    pub(crate) report: BTreeMap<String, u64>,
}

impl Bundle {
    pub(crate) fn strip_vectors(&mut self) {
        for m in &mut self.memories {
            if let Some(o) = m.as_object_mut() {
                o.remove("vector");
                o.remove("embedding_model");
                o.remove("embedding_digest");
            }
        }
    }

    pub(crate) fn has_vectors(&self) -> bool {
        self.memories.iter().any(|m| m.get("vector").is_some())
    }

    pub(crate) fn ndjson(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        for (kind, lines) in [("memory", &self.memories), ("link", &self.links)] {
            for line in lines {
                let mut line = line.clone();
                line["kind"] = json!(kind);
                serde_json::to_writer(&mut out, &line).map_err(|e| e.to_string())?;
                out.push(b'\n');
            }
        }
        Ok(out)
    }
}

pub(crate) enum Source {
    Lamplighter(Bundle),
    Memorygraph {
        memories: Vec<Value>,
        rels: Vec<Value>,
    },
}

/// Detects the format: a single JSON object with a `memories` array is a memorygraph export, anything else must be a complete lamplighter NDJSON export.
pub(crate) fn parse_file(text: &str) -> Result<Source, String> {
    if let Ok(v) = serde_json::from_str::<Value>(text)
        && let Some(memories) = v.get("memories").and_then(Value::as_array)
    {
        let rels = v
            .get("relationships")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        return Ok(Source::Memorygraph {
            memories: memories.clone(),
            rels,
        });
    }
    parse_lamplighter(text).map(Source::Lamplighter)
}

fn parse_lamplighter(text: &str) -> Result<Bundle, String> {
    let mut bundle = Bundle::default();
    let mut end = None;
    let mut seen_header = false;
    for (i, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(raw).map_err(|e| {
            format!(
                "line {}: not JSON ({e}); is this a lamplighter export?",
                i + 1
            )
        })?;
        match v["kind"].as_str() {
            Some("header") => {
                seen_header = true;
                if v["format"] != "lampl-export" {
                    return Err(format!("line {}: not a lampl-export file", i + 1));
                }
                bundle.model = Some((
                    v["embed_model"].as_str().unwrap_or_default().to_owned(),
                    v["embed_digest"].as_str().unwrap_or_default().to_owned(),
                ));
            }
            Some("memory") => bundle.memories.push(v),
            Some("link") => bundle.links.push(v),
            Some("end") => end = Some((v["memories"].as_u64(), v["links"].as_u64())),
            other => {
                return Err(format!(
                    "line {}: unknown kind {other:?}; is this a lamplighter export?",
                    i + 1
                ));
            }
        }
    }
    if !seen_header {
        return Err("no header line: this is not a lamplighter export".to_owned());
    }
    match end {
        Some((Some(m), Some(l)))
            if m == bundle.memories.len() as u64 && l == bundle.links.len() as u64 =>
        {
            Ok(bundle)
        }
        Some(_) => Err(
            "export end marker disagrees with the lines present; the file is damaged".to_owned(),
        ),
        None => Err("export has no end marker; the file is truncated".to_owned()),
    }
}

fn remap(id: &str) -> Uuid {
    Uuid::parse_str(id).unwrap_or_else(|_| Uuid::new_v5(&ID_NAMESPACE, id.as_bytes()))
}

fn timestamp(v: &Value, fallback: DateTime<Utc>) -> String {
    v.as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map_or(fallback, |t| t.with_timezone(&Utc))
        .to_rfc3339()
}

/// Maps memorygraph memories and relationships onto the lean model. Ids that are not UUIDs get a stable v5 id so a second run produces the same rows; an unknown memory type fails the whole import, while an unmapped link type is dropped and counted by name in the report.
pub(crate) fn from_memorygraph(
    memories: &[Value],
    rels: &[Value],
    project: &str,
) -> Result<Bundle, String> {
    let map = mapping()?;
    let mut bundle = Bundle::default();
    let mut ids = HashSet::new();
    let now = Utc::now();
    for m in memories {
        let src_id = m["id"].as_str().ok_or("a memory has no id")?;
        let src_type = m["type"].as_str().unwrap_or_default();
        let target = map.types.get(src_type).ok_or_else(|| {
            format!(
                "memory {src_id}: memorygraph type `{src_type}` is not in the mapping table; known: {}",
                map.types.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        let content = m["content"].as_str().unwrap_or_default();
        let mut body = content.to_owned();
        if let Some(summary) = m["summary"].as_str().filter(|s| !s.is_empty()) {
            body.push_str("\n\nSummary: ");
            body.push_str(summary);
        }
        let title = m["title"]
            .as_str()
            .filter(|t| !t.trim().is_empty())
            .map_or_else(
                || content.chars().take(80).collect::<String>(),
                str::to_owned,
            );
        if title.trim().is_empty() {
            return Err(format!("memory {src_id} has neither title nor content"));
        }
        let mut tags: Vec<String> = m["tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        tags.push(format!("memorygraph:{src_type}"));
        let created = DateTime::parse_from_rfc3339(m["created_at"].as_str().unwrap_or_default())
            .map_or(now, |t| t.with_timezone(&Utc));
        let id = remap(src_id);
        ids.insert(id);
        bundle.memories.push(json!({
            "id": id,
            "memory_type": target,
            "title": title,
            "body": body,
            "tags": tags,
            "project": project,
            "importance": m["importance"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0),
            "created_at": created.to_rfc3339(),
            "updated_at": timestamp(&m["updated_at"], created),
        }));
    }
    for r in rels {
        let src_type = r["type"].as_str().unwrap_or_default();
        let Some(target) = map.links.get(src_type) else {
            *bundle
                .report
                .entry(format!("links dropped: unmapped type {src_type}"))
                .or_default() += 1;
            continue;
        };
        if target == "dropped" {
            *bundle
                .report
                .entry(format!("links dropped: {src_type}"))
                .or_default() += 1;
            continue;
        }
        let (Some(from), Some(to)) = (r["from_memory_id"].as_str(), r["to_memory_id"].as_str())
        else {
            return Err("a relationship has no endpoints".to_owned());
        };
        let (from, to) = (remap(from), remap(to));
        if from == to {
            *bundle
                .report
                .entry("links dropped: self link".to_owned())
                .or_default() += 1;
            continue;
        }
        if !ids.contains(&from) || !ids.contains(&to) {
            *bundle
                .report
                .entry("links dropped: endpoint not in export".to_owned())
                .or_default() += 1;
            continue;
        }
        let props = &r["properties"];
        bundle.links.push(json!({
            "from_id": from,
            "to_id": to,
            "link_type": target,
            "strength": props["strength"].as_f64().unwrap_or(1.0).clamp(0.0, 1.0),
            "created_at": timestamp(&props["created_at"], now),
        }));
    }
    Ok(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_table_is_valid_and_covers_the_thirteen_types() {
        let m = mapping().unwrap();
        assert_eq!(m.types.len(), 13);
        assert_eq!(m.links["CONTRADICTS"], "CONTRADICTS");
        assert_eq!(m.links["REPLACES"], "SUPERSEDES");
        assert_ne!(m.links["CONTRADICTS"], "SUPERSEDES");
    }

    fn rel(kind: &str, from: &str, to: &str) -> Value {
        json!({"type": kind, "from_memory_id": from, "to_memory_id": to, "properties": {"strength": 0.7}})
    }

    #[test]
    fn memorygraph_013_link_types_map_and_unknown_types_are_counted() {
        let mems: Vec<Value> = ["a", "b"]
            .iter()
            .map(|i| json!({"id": i, "type": "general", "title": *i, "content": "c"}))
            .collect();
        let rels = vec![
            rel("ADDRESSES", "a", "b"),
            rel("REPLACES", "a", "b"),
            rel("BUILDS_ON", "a", "b"),
            rel("DEPENDS_ON", "a", "b"),
            rel("FOLLOWS", "a", "b"),
            rel("FUTURE_TYPE", "a", "b"),
            rel("FUTURE_TYPE", "b", "a"),
        ];
        let b = from_memorygraph(&mems, &rels, "p").unwrap();
        let types: Vec<&str> = b
            .links
            .iter()
            .map(|l| l["link_type"].as_str().unwrap())
            .collect();
        assert_eq!(types, ["SOLVES", "SUPERSEDES"]);
        assert_eq!(b.links[1]["from_id"], json!(remap("a")));
        assert_eq!(b.links[1]["to_id"], json!(remap("b")));
        assert_eq!(b.report["links dropped: BUILDS_ON"], 1);
        assert_eq!(b.report["links dropped: unmapped type FUTURE_TYPE"], 2);
    }

    #[test]
    fn non_uuid_ids_remap_stably() {
        assert_eq!(remap("abc"), remap("abc"));
        assert_ne!(remap("abc"), remap("xyz"));
    }

    #[test]
    fn unknown_type_fails_closed() {
        let m = vec![json!({"id": "a", "type": "decision", "title": "t", "content": "c"})];
        assert!(
            from_memorygraph(&m, &[], "p")
                .unwrap_err()
                .contains("not in the mapping table")
        );
    }

    #[test]
    fn truncated_lamplighter_export_is_refused() {
        let text = r#"{"kind":"header","format":"lampl-export","format_version":1,"api_version":1,"embed_model":"m","embed_digest":"d"}"#;
        assert!(parse_lamplighter(text).unwrap_err().contains("truncated"));
    }
}
