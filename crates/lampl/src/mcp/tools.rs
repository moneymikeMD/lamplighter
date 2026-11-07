// SPDX-License-Identifier: Apache-2.0

//! Tool definitions and dispatch. Descriptions stay short: each is paid for in every session's context.

use lampl_core::{LinkType, MemoryType};
use rmcp::model::Tool;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::schema_arc;
use crate::{cmd, local};

fn memory_types() -> Vec<&'static str> {
    MemoryType::ALL.iter().map(|t| t.as_str()).collect()
}

fn link_types() -> Vec<&'static str> {
    LinkType::ALL.iter().map(|t| t.as_str()).collect()
}

pub(super) fn definitions() -> Vec<Tool> {
    let id = json!({"type": "string", "format": "uuid"});
    vec![
        Tool::new(
            "store",
            "Store a memory.",
            schema_arc(json!({
                "type": "object",
                "properties": {
                    "type": {"type": "string", "enum": memory_types(), "default": "fact"},
                    "title": {"type": "string"},
                    "body": {"type": "string"},
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "project": {"type": "string", "default": "default"},
                    "importance": {"type": "number", "minimum": 0, "maximum": 1, "default": 0.5}
                },
                "required": ["title", "body"],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "recall",
            "Recall memories matching a query.",
            schema_arc(json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "project": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "default": 10}
                },
                "required": ["query"],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "get",
            "Fetch one memory by id (server mode).",
            schema_arc(json!({
                "type": "object",
                "properties": {"id": id},
                "required": ["id"],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "link",
            "Link memory `from` to memory `to` (server mode).",
            schema_arc(json!({
                "type": "object",
                "properties": {
                    "from": id,
                    "to": id,
                    "type": {"type": "string", "enum": link_types()},
                    "strength": {"type": "number", "minimum": 0, "maximum": 1, "default": 1}
                },
                "required": ["from", "to", "type"],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "related",
            "List memories linked to one (server mode).",
            schema_arc(json!({
                "type": "object",
                "properties": {"id": id},
                "required": ["id"],
                "additionalProperties": false
            })),
        ),
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreIn {
    #[serde(rename = "type", default = "default_type")]
    memory_type: String,
    title: String,
    body: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default = "default_project")]
    project: String,
    #[serde(default = "default_importance")]
    importance: f32,
}

fn default_type() -> String {
    "fact".to_owned()
}

fn default_project() -> String {
    "default".to_owned()
}

fn default_importance() -> f32 {
    0.5
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecallIn {
    query: String,
    project: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    10
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdIn {
    id: Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkIn {
    from: Uuid,
    to: Uuid,
    #[serde(rename = "type")]
    link_type: String,
    #[serde(default = "default_strength")]
    strength: f32,
}

fn default_strength() -> f32 {
    1.0
}

fn parse<T: for<'de> Deserialize<'de>>(tool: &str, args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|e| format!("invalid arguments for `{tool}`: {e}"))
}

pub(super) fn call(name: &str, args: Value) -> Result<Value, String> {
    match name {
        "store" => {
            let a: StoreIn = parse(name, args)?;
            cmd::store_value(local::StoreArgs {
                title: a.title,
                body: a.body,
                memory_type: a.memory_type,
                tags: a.tags,
                project: a.project,
                importance: a.importance,
                no_embed: false,
            })
        }
        "recall" => {
            let a: RecallIn = parse(name, args)?;
            let out = cmd::recall_hits(&local::RecallArgs::new(a.query, a.project, a.limit))?;
            Ok(json!({"hits": out.hits, "warning": out.warning}))
        }
        "get" => cmd::get_value(parse::<IdIn>(name, args)?.id),
        "link" => {
            let a: LinkIn = parse(name, args)?;
            cmd::link_value(a.from, a.to, &a.link_type, a.strength)
        }
        "related" => Ok(Value::Array(cmd::related_items(
            parse::<IdIn>(name, args)?.id,
        )?)),
        other => Err(format!("unknown tool `{other}`")),
    }
}
