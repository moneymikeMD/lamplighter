// SPDX-License-Identifier: Apache-2.0

//! Direct read of memory-pg's memorygraph tables (`memories` and `links`, flat columns), the one place the client talks to a database.
//! Soft-invalidated links (`valid_until` or `invalidated_by` set) are skipped.

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
    let ts =
        |c: &str| format!("to_char({c} AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')");
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT id, type, title, content, summary, tags, importance, {} AS created_at, {} AS updated_at FROM memories ORDER BY id",
        ts("created_at"),
        ts("updated_at")
    )))
    .fetch_all(&mut conn)
    .await
    .map_err(|e| unrecognised(&e))?;
    let memories = rows
        .iter()
        .map(|r| {
            Ok(json!({
                "id": r.try_get::<String, _>("id")?,
                "type": r.try_get::<String, _>("type")?,
                "title": r.try_get::<String, _>("title")?,
                "content": r.try_get::<String, _>("content")?,
                "summary": r.try_get::<Option<String>, _>("summary")?,
                "tags": r.try_get::<Vec<String>, _>("tags")?,
                "importance": r.try_get::<f64, _>("importance")?,
                "created_at": r.try_get::<String, _>("created_at")?,
                "updated_at": r.try_get::<String, _>("updated_at")?,
            }))
        })
        .collect::<Result<Vec<Value>, sqlx::Error>>()
        .map_err(|e| unrecognised(&e))?;
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT from_id, to_id, rel_type, strength, {} AS created_at FROM links \
         WHERE valid_until IS NULL AND invalidated_by IS NULL ORDER BY id",
        ts("created_at")
    )))
    .fetch_all(&mut conn)
    .await
    .map_err(|e| unrecognised(&e))?;
    let rels = rows
        .iter()
        .map(|r| {
            Ok(json!({
                "from_memory_id": r.try_get::<String, _>("from_id")?,
                "to_memory_id": r.try_get::<String, _>("to_id")?,
                "type": r.try_get::<String, _>("rel_type")?,
                "properties": {
                    "strength": r.try_get::<f64, _>("strength")?,
                    "created_at": r.try_get::<String, _>("created_at")?,
                },
            }))
        })
        .collect::<Result<Vec<Value>, sqlx::Error>>()
        .map_err(|e| unrecognised(&e))?;
    Ok(Source::Memorygraph { memories, rels })
}

pub(crate) fn read_memory_pg(url: &str) -> Result<Source, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?
        .block_on(read(url))
}
