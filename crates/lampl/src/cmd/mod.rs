// SPDX-License-Identifier: Apache-2.0

//! Client commands. `store` and `recall` follow the configured mode; the rest need a server and fail loudly without one.

mod audit;
mod doctor;
mod transfer;

use clap::Args as ClapArgs;
use lampl_core::{LinkType, MemoryType};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::client::{Client, ServerSettings, Target, target};
use crate::local;

pub(crate) use audit::{AuditArgs, audit};
pub(crate) use doctor::{DoctorArgs, doctor};
pub(crate) use transfer::{ExportArgs, ImportArgs, export, import};

fn server(what: &str) -> Result<Client, String> {
    match target()? {
        Target::Server(s) => Client::connect(&s),
        Target::Local => Err(format!(
            "`lampl {what}` needs a server; local mode supports only store and recall"
        )),
    }
}

pub(super) fn server_settings() -> Result<Option<ServerSettings>, String> {
    Ok(match target()? {
        Target::Server(s) => Some(s),
        Target::Local => None,
    })
}

fn print(v: &Value) {
    println!("{v}");
}

pub(crate) fn store(args: local::StoreArgs) -> Result<(), String> {
    print(&store_value(args)?);
    Ok(())
}

pub(crate) fn store_value(args: local::StoreArgs) -> Result<Value, String> {
    let memory_type: MemoryType = args.memory_type.parse().map_err(|e| format!("{e}"))?;
    let Some(settings) = server_settings()? else {
        return local::store_value(args);
    };
    let client = Client::connect(&settings)?;
    if args.no_embed {
        return Err(
            "--no-embed applies to local mode; the server embeds in the background".to_owned(),
        );
    }
    client.post(
        "/v1/memories",
        &json!({
            "memory_type": memory_type.as_str(),
            "title": args.title,
            "body": args.body,
            "tags": args.tags,
            "project": args.project,
            "importance": args.importance,
        }),
    )
}

pub(crate) fn recall(args: &local::RecallArgs) -> Result<(), String> {
    let out = recall_hits(args)?;
    if let Some(w) = out.warning {
        eprintln!("lampl: warning: {w}");
    }
    for hit in &out.hits {
        print(hit);
    }
    Ok(())
}

pub(crate) struct RecallOutput {
    pub(crate) hits: Vec<Value>,
    pub(crate) warning: Option<&'static str>,
}

pub(crate) fn recall_hits(args: &local::RecallArgs) -> Result<RecallOutput, String> {
    let Some(settings) = server_settings()? else {
        return Ok(RecallOutput {
            hits: local::recall_hits(args)?,
            warning: None,
        });
    };
    let client = Client::connect(&settings)?;
    let mut body = json!({"query": args.text(), "limit": args.limit});
    if let Some(p) = &args.project {
        body["project"] = json!(p);
    }
    let out = client.post("/v1/recall", &body)?;
    Ok(RecallOutput {
        hits: out["hits"].as_array().cloned().unwrap_or_default(),
        warning: (out["mode"] == "full-text")
            .then_some("the server's embedder is unavailable; this recall is full-text only"),
    })
}

#[derive(ClapArgs)]
pub(crate) struct GetArgs {
    id: Uuid,
}

pub(crate) fn get(args: &GetArgs) -> Result<(), String> {
    print(&get_value(args.id)?);
    Ok(())
}

pub(crate) fn get_value(id: Uuid) -> Result<Value, String> {
    server("get")?.get(&format!("/v1/memories/{id}"))
}

#[derive(ClapArgs)]
pub(crate) struct LinkArgs {
    from: Uuid,
    to: Uuid,
    /// SUPERSEDES, SOLVES, CAUSES or CONTRADICTS
    link_type: String,
    #[arg(long, default_value_t = 1.0)]
    strength: f32,
}

pub(crate) fn link(args: &LinkArgs) -> Result<(), String> {
    print(&link_value(
        args.from,
        args.to,
        &args.link_type,
        args.strength,
    )?);
    Ok(())
}

pub(crate) fn link_value(
    from: Uuid,
    to: Uuid,
    link_type: &str,
    strength: f32,
) -> Result<Value, String> {
    let link_type: LinkType = link_type.parse().map_err(|e| format!("{e}"))?;
    server("link")?.post(
        "/v1/links",
        &json!({
            "from_id": from,
            "to_id": to,
            "link_type": link_type.as_str(),
            "strength": strength,
        }),
    )
}

#[derive(ClapArgs)]
pub(crate) struct RelatedArgs {
    id: Uuid,
}

pub(crate) fn related(args: &RelatedArgs) -> Result<(), String> {
    for item in &related_items(args.id)? {
        print(item);
    }
    Ok(())
}

pub(crate) fn related_items(id: Uuid) -> Result<Vec<Value>, String> {
    let out = server("related")?.get(&format!("/v1/memories/{id}/related"))?;
    Ok(out.as_array().cloned().unwrap_or_default())
}
