// SPDX-License-Identifier: Apache-2.0

//! Direct read of memory-pg's memorygraph tables, the one place the client talks to a database.
//! The layout (`nodes(id, properties jsonb)`, `relationships(from_id, to_id, rel_type, properties jsonb)`) is UNVERIFIED against the live store; an unrecognised schema fails closed.

use serde_json::{Value, json};
use sqlx::{Connection, PgConnection, Row};

use super::Source;

async fn read(url: &str) -> Result<Source, String> {
    let mut conn = PgConnection::connect(url)
        .await
        .map_err(|e| format!("connecting to the memorygraph database: {e}"))?;
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables WHERE table_schema = 'public' ORDER BY 1",
    )
    .fetch_all(&mut conn)
    .await
    .map_err(|e| e.to_string())?;
    let unrecognised = |e: &dyn std::fmt::Display| {
        format!(
            "memorygraph schema not recognised ({e}); tables present: {}. Import its JSON export instead",
            tables.join(", ")
        )
    };
    let nodes = sqlx::query("SELECT properties::text AS p FROM nodes WHERE properties ? 'title'")
        .fetch_all(&mut conn)
        .await
        .map_err(|e| unrecognised(&e))?;
    let rels = sqlx::query("SELECT from_id::text AS f, to_id::text AS t, rel_type AS r, properties::text AS p FROM relationships")
        .fetch_all(&mut conn)
        .await
        .map_err(|e| unrecognised(&e))?;
    let parse = |s: &str| serde_json::from_str::<Value>(s).map_err(|e| unrecognised(&e));
    let memories = nodes
        .iter()
        .map(|r| parse(&r.get::<String, _>("p")))
        .collect::<Result<Vec<_>, _>>()?;
    let rels = rels
        .iter()
        .map(|r| {
            Ok(json!({
                "from_memory_id": r.get::<String, _>("f"),
                "to_memory_id": r.get::<String, _>("t"),
                "type": r.get::<String, _>("r"),
                "properties": parse(&r.get::<String, _>("p"))?,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Source::Memorygraph { memories, rels })
}

pub(crate) fn read_memory_pg(url: &str) -> Result<Source, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?
        .block_on(read(url))
}
