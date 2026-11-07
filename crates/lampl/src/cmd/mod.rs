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
    let Some(settings) = server_settings()? else {
        return local::store(args);
    };
    let client = Client::connect(&settings)?;
    if args.no_embed {
        return Err(
            "--no-embed applies to local mode; the server embeds in the background".to_owned(),
        );
    }
    let memory_type: MemoryType = args.memory_type.parse().map_err(|e| format!("{e}"))?;
    print(&client.post(
        "/v1/memories",
        &json!({
            "memory_type": memory_type.as_str(),
            "title": args.title,
            "body": args.body,
            "tags": args.tags,
            "project": args.project,
            "importance": args.importance,
        }),
    )?);
    Ok(())
}

pub(crate) fn recall(args: &local::RecallArgs) -> Result<(), String> {
    let Some(settings) = server_settings()? else {
        return local::recall(args);
    };
    let client = Client::connect(&settings)?;
    let mut body = json!({"query": args.text(), "limit": args.limit});
    if let Some(p) = &args.project {
        body["project"] = json!(p);
    }
    let out = client.post("/v1/recall", &body)?;
    if out["mode"] == "full-text" {
        eprintln!(
            "lampl: warning: the server's embedder is unavailable; this recall is full-text only"
        );
    }
    for hit in out["hits"].as_array().into_iter().flatten() {
        print(hit);
    }
    Ok(())
}

#[derive(ClapArgs)]
pub(crate) struct GetArgs {
    id: Uuid,
}

pub(crate) fn get(args: &GetArgs) -> Result<(), String> {
    print(&server("get")?.get(&format!("/v1/memories/{}", args.id))?);
    Ok(())
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
    let link_type: LinkType = args.link_type.parse().map_err(|e| format!("{e}"))?;
    print(&server("link")?.post(
        "/v1/links",
        &json!({
            "from_id": args.from,
            "to_id": args.to,
            "link_type": link_type.as_str(),
            "strength": args.strength,
        }),
    )?);
    Ok(())
}

#[derive(ClapArgs)]
pub(crate) struct RelatedArgs {
    id: Uuid,
}

pub(crate) fn related(args: &RelatedArgs) -> Result<(), String> {
    let out = server("related")?.get(&format!("/v1/memories/{}/related", args.id))?;
    for item in out.as_array().into_iter().flatten() {
        print(item);
    }
    Ok(())
}
