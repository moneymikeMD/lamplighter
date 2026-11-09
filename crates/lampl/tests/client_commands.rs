// SPDX-License-Identifier: Apache-2.0

//! The `lampl` client against a real `serve` over TCP. Needs `LAMPL_TEST_DATABASE_URL` (a pgvector Postgres superuser) and skips without it; each test gets its own database.

use std::io::{Read, Write};
use std::process::{Command, Output};
use std::sync::Arc;

use lampl::serve::{
    AppState, EmbedMode, KeyStore, SharedEmbedder, embed_pending, generate_key, hash_key, probe,
    router,
};
use lampl_embed::{EmbedError, Embedder};
use serde_json::Value;
use sqlx::Connection;
use tokio::runtime::Runtime;
use uuid::Uuid;

#[derive(Debug)]
struct Fake {
    digest: &'static str,
    down: bool,
}

impl Embedder for Fake {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if self.down {
            return Err(EmbedError::Endpoint("fake embedder is down".to_owned()));
        }
        if texts.iter().any(|t| t.contains("POISON")) {
            return Err(EmbedError::Embed("rejects this text".to_owned()));
        }
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0; lampl_embed::DIM];
                for w in t.to_lowercase().split_whitespace() {
                    let h = w.bytes().fold(7usize, |a, b| {
                        a.wrapping_mul(31).wrapping_add(usize::from(b))
                    });
                    v[h % lampl_embed::DIM] += 1.0;
                }
                v
            })
            .collect())
    }
    fn dim(&self) -> usize {
        lampl_embed::DIM
    }
    fn model_id(&self) -> &str {
        "fake"
    }
    fn model_digest(&self) -> &str {
        self.digest
    }
}

struct Live {
    rt: Runtime,
    state: Arc<AppState>,
    url: String,
    key: String,
    db: String,
    admin: String,
    cfg: tempfile::TempDir,
}

fn admin_url() -> Option<String> {
    let url = std::env::var("LAMPL_TEST_DATABASE_URL").ok();
    if url.is_none() {
        assert!(
            std::env::var("CI").as_deref() != Ok("true"),
            "LAMPL_TEST_DATABASE_URL must be set when CI=true"
        );
        eprintln!("LAMPL_TEST_DATABASE_URL unset; skipping");
    }
    url
}

fn start(embedder: Fake) -> Option<Live> {
    let admin = admin_url()?;
    let rt = Runtime::new().unwrap();
    let db = format!("lampl_c_{}", Uuid::now_v7().simple());
    let (base, query) = admin
        .split_once('?')
        .map_or((admin.clone(), String::new()), |(b, q)| {
            (b.to_owned(), format!("?{q}"))
        });
    let root = base.rsplit_once('/').unwrap().0.to_owned();
    let key = generate_key().unwrap();
    let keys = std::env::temp_dir().join(format!("lampl-client-keys-{}", Uuid::now_v7()));
    std::fs::write(&keys, format!("tester {}\n", hash_key(&key))).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&keys, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let (state, url) = rt.block_on(async {
        let mut conn = sqlx::PgConnection::connect(&admin).await.unwrap();
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {db}")))
            .execute(&mut conn)
            .await
            .unwrap();
        let store = lampl_store_pg::PgStore::connect(&format!("{root}/{db}{query}"))
            .await
            .unwrap();
        let embedder: SharedEmbedder = Arc::new(embedder);
        let state = AppState::new(
            store,
            embedder,
            EmbedMode::Endpoint,
            KeyStore::open(&keys).unwrap(),
            0.95,
        );
        probe(&state).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = router(Arc::clone(&state));
        tokio::spawn(async move { axum::serve(listener, app).await });
        (state, url)
    });
    Some(Live {
        rt,
        state,
        url,
        key,
        db,
        admin,
        cfg: tempfile::tempdir().unwrap(),
    })
}

impl Live {
    fn lampl(&self, args: &[&str]) -> Output {
        self.lampl_with(&self.key, args)
    }

    fn lampl_with(&self, key: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_lampl"))
            .args(args)
            .env("LAMPL_CONFIG_DIR", self.cfg.path())
            .env("LAMPL_DATA_DIR", self.cfg.path())
            .env("LAMPL_SERVER", &self.url)
            .env("LAMPL_API_KEY", key)
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.lampl(args);
        assert!(out.status.success(), "{args:?}: {}", err(&out));
        String::from_utf8(out.stdout).unwrap()
    }

    fn embed_all(&self) {
        self.rt
            .block_on(async { while embed_pending(&self.state).await.unwrap() > 0 {} });
    }

    fn recall_ids(&self, query: &str) -> Vec<String> {
        self.ok(&["recall", "--query", query])
            .lines()
            .map(|l| {
                serde_json::from_str::<Value>(l).unwrap()["memory"]["id"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        let (admin, db) = (self.admin.clone(), self.db.clone());
        self.rt.block_on(async move {
            if let Ok(mut conn) = sqlx::PgConnection::connect(&admin).await {
                let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
                    "DROP DATABASE IF EXISTS {db} WITH (FORCE)"
                )))
                .execute(&mut conn)
                .await;
            }
        });
    }
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn json(s: &str) -> Value {
    serde_json::from_str(s.lines().next().unwrap()).unwrap()
}

const DOCS: &[(&str, &str, &str)] = &[
    (
        "restic nightly backup",
        "restic backs up to the nas every night",
        "procedure",
    ),
    (
        "grafana alert gotcha",
        "grafana alert rules silently pause when the datasource renames",
        "gotcha",
    ),
    (
        "postgres vacuum fix",
        "autovacuum was starved; raise the cost limit",
        "fix",
    ),
    (
        "dns outage problem",
        "unbound crashed after the config reload",
        "problem",
    ),
];

fn populate(l: &Live) -> Vec<String> {
    let ids: Vec<String> = DOCS
        .iter()
        .map(|(t, b, ty)| {
            json(&l.ok(&[
                "store",
                "--title",
                t,
                "--body",
                b,
                "--type",
                ty,
                "--tags",
                "lab,test",
                "--project",
                "lab",
            ]))["id"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    l.ok(&["link", &ids[2], &ids[3], "SOLVES"]);
    l.ok(&["link", &ids[0], &ids[1], "CONTRADICTS", "--strength", "0.4"]);
    l.embed_all();
    ids
}

#[test]
fn unreachable_server_is_an_error_naming_it_never_an_empty_result() {
    let d = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_lampl"))
        .args(["recall", "--query", "hello"])
        .env("LAMPL_CONFIG_DIR", d.path())
        .env("LAMPL_DATA_DIR", d.path())
        .env("LAMPL_SERVER", "http://127.0.0.1:9")
        .env("LAMPL_API_KEY", "x")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(err(&out).contains("http://127.0.0.1:9"), "{}", err(&out));
    assert!(err(&out).contains("unreachable"));
}

#[test]
fn a_server_speaking_another_api_version_is_refused() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut s = stream;
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nx-lampl-api-version: 99\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}");
        }
    });
    let d = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_lampl"))
        .args(["recall", "--query", "x"])
        .env("LAMPL_CONFIG_DIR", d.path())
        .env("LAMPL_SERVER", &url)
        .env("LAMPL_API_KEY", "x")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(err(&out).contains("API version mismatch"), "{}", err(&out));
}

#[test]
fn pg_commands_work_and_a_wrong_key_fails_loudly() {
    let Some(l) = start(Fake {
        digest: "sha256:a",
        down: false,
    }) else {
        return;
    };
    let ids = populate(&l);
    let got = json(&l.ok(&["get", &ids[0]]));
    assert_eq!(got["title"], "restic nightly backup");
    let related = l.ok(&["related", &ids[2]]);
    assert!(related.contains("SOLVES"));
    assert!(!l.recall_ids("restic nightly").is_empty());

    let out = l.lampl(&["link", &ids[0], &ids[1], "NOPE"]);
    assert!(!out.status.success());
    assert!(
        err(&out).contains("SUPERSEDES, SOLVES, CAUSES, CONTRADICTS"),
        "{}",
        err(&out)
    );
    let out = l.lampl(&["store", "--title", "t", "--body", "b", "--type", "general"]);
    assert!(err(&out).contains("fact, decision, problem, fix, procedure, gotcha"));

    let out = l.lampl_with("lampl_wrong", &["recall", "--query", "restic"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(err(&out).contains("rejected the API key"), "{}", err(&out));
    assert!(!err(&out).contains("lampl_wrong"));
}

#[test]
fn pg_doctor_reports_the_store_and_fails_when_degraded() {
    let Some(l) = start(Fake {
        digest: "sha256:a",
        down: false,
    }) else {
        return;
    };
    populate(&l);
    let report: Value = serde_json::from_str(&l.ok(&["doctor", "--json"])).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["health"]["memory_count"], 4);
    assert!(report["health"]["store"]["host"].is_string());
    assert!(l.ok(&["doctor"]).contains("vector coverage"));

    let Some(down) = start(Fake {
        digest: "sha256:a",
        down: true,
    }) else {
        return;
    };
    let out = down.lampl(&["doctor"]);
    assert!(!out.status.success());
    assert!(err(&out).contains("embedder unavailable"), "{}", err(&out));
}

#[test]
fn pg_doctor_names_failed_rows_and_retry_failed_requeues_them() {
    let Some(l) = start(Fake {
        digest: "sha256:a",
        down: false,
    }) else {
        return;
    };
    populate(&l);
    let bad = json(&l.ok(&[
        "store",
        "--title",
        "bad",
        "--body",
        "POISON",
        "--type",
        "fact",
        "--project",
        "lab",
    ]))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    l.embed_all();

    let out = l.lampl(&["doctor"]);
    assert!(!out.status.success());
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        text.contains("rows failing to embed: 1 (next retry due "),
        "{text}"
    );
    assert!(
        text.contains(&format!("{bad}: embedding failed: rejects this text")),
        "{text}"
    );

    let out = l.lampl(&["doctor", "--json"]);
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["health"]["embed_failed"][0]["id"], bad.as_str());

    let out = l.lampl(&["doctor", "--retry-failed"]);
    assert!(
        err(&out).contains("queued 1 failed row(s)"),
        "{}",
        err(&out)
    );
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("rows failing to embed"),
        "the retry cleared the skip set before the report"
    );
    l.embed_all();
    let report: Value = serde_json::from_slice(&l.lampl(&["doctor", "--json"]).stdout).unwrap();
    assert_eq!(
        report["health"]["embed_failed_rows"], 1,
        "re-tried and rejected again"
    );
}

#[test]
fn pg_audit_finds_duplicates_untagged_and_skips_superseded() {
    let Some(l) = start(Fake {
        digest: "sha256:a",
        down: false,
    }) else {
        return;
    };
    let a = json(&l.ok(&[
        "store",
        "--title",
        "Same  Title",
        "--body",
        "one",
        "--project",
        "p",
    ]))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let b = json(&l.ok(&[
        "store",
        "--title",
        "same title",
        "--body",
        "two",
        "--project",
        "p",
        "--tags",
        "x",
    ]))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let c = json(&l.ok(&["store", "--title", "old", "--body", "one", "--project", "q"]))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let d = json(&l.ok(&["store", "--title", "new", "--body", "one", "--project", "q"]))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    l.ok(&["link", &d, &c, "SUPERSEDES"]);
    let r: Value = serde_json::from_str(&l.ok(&["audit", "--json"])).unwrap();
    assert_eq!(r["memories_audited"], 3);
    assert_eq!(r["superseded_excluded"], 1);
    let dups = r["duplicates"].as_array().unwrap();
    assert_eq!(dups.len(), 1, "{r}");
    assert!(dups[0]["ids"].to_string().contains(&a) && dups[0]["ids"].to_string().contains(&b));
    assert_eq!(r["untagged"].as_array().unwrap().len(), 2);
}

#[test]
fn pg_export_to_a_fresh_database_then_import_gives_identical_recall() {
    let Some(src) = start(Fake {
        digest: "sha256:a",
        down: false,
    }) else {
        return;
    };
    let ids = populate(&src);
    let newer = json(&src.ok(&[
        "store",
        "--title",
        "restic nightly backup v2",
        "--body",
        "restic now backs up to the nas twice a night",
        "--project",
        "lab",
        "--tags",
        "lab",
    ]))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    src.ok(&["link", &newer, &ids[0], "SUPERSEDES"]);
    src.embed_all();
    let queries = [
        "restic nightly",
        "grafana alert",
        "autovacuum",
        "unbound dns",
        "backup",
    ];
    let before: Vec<Vec<String>> = queries.iter().map(|q| src.recall_ids(q)).collect();
    assert!(before.iter().any(|v| !v.is_empty()));
    let file = src.cfg.path().join("export.ndjson");
    src.ok(&["export", "-o", file.to_str().unwrap()]);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("\"vector\""), "export carries vectors");
    assert!(text.lines().next().unwrap().contains("sha256:a"));

    let dst = start(Fake {
        digest: "sha256:a",
        down: false,
    })
    .unwrap();
    let out: Value = serde_json::from_str(&dst.ok(&["import", file.to_str().unwrap()])).unwrap();
    assert_eq!(
        (
            out["memories_imported"].as_u64(),
            out["links_imported"].as_u64()
        ),
        (Some(5), Some(3))
    );
    let health: Value = serde_json::from_str(&dst.ok(&["doctor", "--json"])).unwrap();
    assert_eq!(
        health["health"]["embedded_count"], 5,
        "vectors arrived, no re-embed needed"
    );
    let after: Vec<Vec<String>> = queries.iter().map(|q| dst.recall_ids(q)).collect();
    assert_eq!(before, after);
    for q in queries {
        assert!(
            !dst.recall_ids(q).contains(&ids[0]),
            "superseded memory surfaced for {q}"
        );
    }
    assert!(dst.recall_ids("restic nightly").contains(&newer));
    assert!(dst.ok(&["related", &ids[2]]).contains("SOLVES"));

    let again: Value = serde_json::from_str(&dst.ok(&["import", file.to_str().unwrap()])).unwrap();
    assert_eq!(
        (
            again["memories_imported"].as_u64(),
            again["links_imported"].as_u64()
        ),
        (Some(0), Some(0))
    );
}

#[test]
fn pg_import_refuses_a_model_mismatch_unless_reembed() {
    let Some(src) = start(Fake {
        digest: "sha256:a",
        down: false,
    }) else {
        return;
    };
    populate(&src);
    let file = src.cfg.path().join("export.ndjson");
    src.ok(&["export", "-o", file.to_str().unwrap()]);
    let dst = start(Fake {
        digest: "sha256:b",
        down: false,
    })
    .unwrap();
    let out = dst.lampl(&["import", file.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        err(&out).contains("embedding model mismatch") && err(&out).contains("--reembed"),
        "{}",
        err(&out)
    );
    let ok: Value =
        serde_json::from_str(&dst.ok(&["import", file.to_str().unwrap(), "--reembed"])).unwrap();
    assert_eq!(ok["memories_imported"], 4);
    dst.embed_all();
    assert!(!dst.recall_ids("restic nightly").is_empty());
}

#[test]
fn pg_memorygraph_export_imports_idempotently_keeping_contradicts() {
    let Some(l) = start(Fake {
        digest: "sha256:a",
        down: false,
    }) else {
        return;
    };
    let (a, b, c) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let doc = serde_json::json!({
        "format_version": "2.0",
        "memories": [
            {"id": a, "type": "solution", "title": "fix one", "content": "body one", "tags": ["x"], "importance": 0.9, "created_at": "2026-09-12T17:24:44.550Z", "updated_at": "2026-09-18T18:15:47.393Z"},
            {"id": b, "type": "problem", "title": "problem two", "content": "body two", "tags": [], "importance": 1, "created_at": "2026-09-12T17:24:44.550Z", "updated_at": "2026-09-18T18:15:47.393Z"},
            {"id": "not-a-uuid", "type": "code_pattern", "title": "pattern", "content": "body three", "tags": [], "created_at": "2026-09-12T17:24:44.550Z", "updated_at": "2026-09-12T17:24:44.550Z"}
        ],
        "relationships": [
            {"from_memory_id": a, "to_memory_id": b, "type": "CONTRADICTS", "properties": {"strength": 0.7, "created_at": "2026-09-13T00:00:00Z"}},
            {"from_memory_id": a, "to_memory_id": b, "type": "RELATED_TO", "properties": {}},
            {"from_memory_id": a, "to_memory_id": c, "type": "SOLVES", "properties": {}}
        ]
    });
    let file = l.cfg.path().join("mg.json");
    std::fs::write(&file, doc.to_string()).unwrap();
    let first: Value = serde_json::from_str(&l.ok(&["import", file.to_str().unwrap()])).unwrap();
    assert_eq!(
        (
            first["memories_imported"].as_u64(),
            first["links_imported"].as_u64()
        ),
        (Some(3), Some(1))
    );
    let second: Value = serde_json::from_str(&l.ok(&["import", file.to_str().unwrap()])).unwrap();
    assert_eq!(
        (
            second["memories_imported"].as_u64(),
            second["links_imported"].as_u64()
        ),
        (Some(0), Some(0))
    );
    let rel = l.ok(&["related", &a.to_string()]);
    assert!(rel.contains("CONTRADICTS") && !rel.contains("SUPERSEDES"));
    assert_eq!(json(&l.ok(&["get", &a.to_string()]))["memory_type"], "fix");
}

#[test]
fn pg_memorygraph_013_link_types_import_and_unmapped_types_are_reported() {
    let Some(l) = start(Fake {
        digest: "sha256:a",
        down: false,
    }) else {
        return;
    };
    let (newer, older) = (Uuid::now_v7(), Uuid::now_v7());
    let rel = |t: &str, from: Uuid, to: Uuid| serde_json::json!({"from_memory_id": from, "to_memory_id": to, "type": t, "properties": {}});
    let mem = |id: Uuid, title: &str| serde_json::json!({"id": id, "type": "solution", "title": title, "content": "body", "created_at": "2026-09-12T17:24:44.550Z"});
    let doc = serde_json::json!({
        "memories": [mem(newer, "new approach"), mem(older, "old approach")],
        "relationships": [
            rel("REPLACES", newer, older),
            rel("ADDRESSES", newer, older),
            rel("BUILDS_ON", newer, older),
            rel("DEPENDS_ON", newer, older),
            rel("FOLLOWS", newer, older),
            rel("INVENTED_LATER", newer, older),
            rel("INVENTED_LATER", older, newer)
        ]
    });
    let file = l.cfg.path().join("mg013.json");
    std::fs::write(&file, doc.to_string()).unwrap();
    let path = file.to_str().unwrap();
    let dry = l.lampl(&["import", path, "--dry-run"]);
    assert!(dry.status.success(), "{}", err(&dry));
    let report = err(&dry);
    assert!(report.contains("links dropped: BUILDS_ON: 1"), "{report}");
    assert!(
        report.contains("links dropped: unmapped type INVENTED_LATER: 2"),
        "{report}"
    );
    let out: Value = serde_json::from_str(&l.ok(&["import", path])).unwrap();
    assert_eq!(out["links_imported"], 2);
    let related = l.ok(&["related", &older.to_string()]);
    assert!(related.contains("SUPERSEDES") && related.contains("SOLVES"));
}

#[test]
fn pg_a_truncated_export_file_is_refused() {
    let Some(l) = start(Fake {
        digest: "sha256:a",
        down: false,
    }) else {
        return;
    };
    populate(&l);
    let file = l.cfg.path().join("e.ndjson");
    l.ok(&["export", "-o", file.to_str().unwrap()]);
    let text = std::fs::read_to_string(&file).unwrap();
    let cut: Vec<&str> = text.lines().collect();
    std::fs::write(&file, cut[..cut.len() - 1].join("\n")).unwrap();
    let fresh = start(Fake {
        digest: "sha256:a",
        down: false,
    })
    .unwrap();
    let out = fresh.lampl(&["import", file.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(err(&out).contains("truncated"), "{}", err(&out));
}

#[test]
fn pg_project_export_leaves_out_links_that_leave_the_project() {
    let Some(l) = start(Fake {
        digest: "sha256:a",
        down: false,
    }) else {
        return;
    };
    let a = json(&l.ok(&["store", "--title", "a", "--body", "alpha", "--project", "p"]))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let b = json(&l.ok(&["store", "--title", "b", "--body", "beta", "--project", "p"]))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let c = json(&l.ok(&["store", "--title", "c", "--body", "gamma", "--project", "q"]))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    l.ok(&["link", &a, &b, "CAUSES"]);
    l.ok(&["link", &a, &c, "CAUSES"]);
    let file = l.cfg.path().join("p.ndjson");
    l.ok(&["export", "--project", "p", "-o", file.to_str().unwrap()]);
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(text.matches("\"kind\":\"link\"").count(), 1);
    let fresh = start(Fake {
        digest: "sha256:a",
        down: false,
    })
    .unwrap();
    let out: Value = serde_json::from_str(&fresh.ok(&["import", file.to_str().unwrap()])).unwrap();
    assert_eq!(
        (
            out["memories_imported"].as_u64(),
            out["links_imported"].as_u64()
        ),
        (Some(2), Some(1))
    );
}

const MEMORYGRAPH_DDL: &str = "
CREATE EXTENSION IF NOT EXISTS vector;
CREATE TABLE memories (
  id text PRIMARY KEY, type text NOT NULL, title text NOT NULL, content text NOT NULL, summary text,
  tags text[] NOT NULL DEFAULT '{}', importance double precision NOT NULL DEFAULT 0.5,
  confidence double precision NOT NULL DEFAULT 0.8, effectiveness double precision,
  usage_count integer NOT NULL DEFAULT 0, created_at timestamptz NOT NULL, updated_at timestamptz NOT NULL,
  last_accessed timestamptz, version integer NOT NULL DEFAULT 1, updated_by text, context jsonb,
  embedding halfvec(1024), embedding_model text, embedding_error text
);
CREATE TABLE links (
  id text PRIMARY KEY, from_id text NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
  to_id text NOT NULL REFERENCES memories(id) ON DELETE CASCADE, rel_type text NOT NULL,
  strength double precision NOT NULL DEFAULT 0.5, confidence double precision NOT NULL DEFAULT 0.8,
  context text, evidence_count integer NOT NULL DEFAULT 1, success_rate double precision,
  created_at timestamptz NOT NULL, last_validated timestamptz NOT NULL, validation_count integer NOT NULL DEFAULT 0,
  counter_evidence_count integer NOT NULL DEFAULT 0, valid_from timestamptz NOT NULL, valid_until timestamptz,
  recorded_at timestamptz NOT NULL, invalidated_by text
);
INSERT INTO memories (id, type, title, content, tags, importance, created_at, updated_at) VALUES
  ('11111111-1111-4111-8111-111111111111', 'solution', 'fix one', 'body one', '{x}', 0.9, '2026-09-12T17:24:44Z', '2026-09-18T18:15:47Z'),
  ('22222222-2222-4222-8222-222222222222', 'problem', 'problem two', 'body two', '{}', 1, '2026-09-12T17:24:44Z', '2026-09-12T17:24:44Z'),
  ('legacy-3', 'general', 'legacy three', 'body three', '{y}', 0.5, '2026-09-12T17:24:44Z', '2026-09-12T17:24:44Z');
INSERT INTO links (id, from_id, to_id, rel_type, strength, created_at, last_validated, valid_from, recorded_at, valid_until, invalidated_by) VALUES
  ('l1', '11111111-1111-4111-8111-111111111111', '22222222-2222-4222-8222-222222222222', 'CONTRADICTS', 0.7, now(), now(), now(), now(), NULL, NULL),
  ('l2', '11111111-1111-4111-8111-111111111111', 'legacy-3', 'SOLVES', 0.5, now(), now(), now(), now(), now(), NULL),
  ('l3', '22222222-2222-4222-8222-222222222222', 'legacy-3', 'CAUSES', 0.5, now(), now(), now(), now(), NULL, 'l1'),
  ('l4', 'legacy-3', '22222222-2222-4222-8222-222222222222', 'RELATED_TO', 0.5, now(), now(), now(), now(), NULL, NULL);
";

#[test]
fn pg_from_postgres_reads_memorygraphs_flat_schema_and_skips_invalidated_links() {
    let Some(l) = start(Fake {
        digest: "sha256:a",
        down: false,
    }) else {
        return;
    };
    let src = format!("lampl_mg_{}", Uuid::now_v7().simple());
    let (base, query) = l
        .admin
        .split_once('?')
        .map_or((l.admin.clone(), String::new()), |(b, q)| {
            (b.to_owned(), format!("?{q}"))
        });
    let root = base.rsplit_once('/').unwrap().0.to_owned();
    l.rt.block_on(async {
        let mut admin = sqlx::PgConnection::connect(&l.admin).await.unwrap();
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {src}")))
            .execute(&mut admin)
            .await
            .unwrap();
        let mut conn = sqlx::PgConnection::connect(&format!("{root}/{src}{query}"))
            .await
            .unwrap();
        sqlx::raw_sql(MEMORYGRAPH_DDL)
            .execute(&mut conn)
            .await
            .unwrap();
    });
    let url = format!("{root}/{src}{query}");
    let run = || {
        let out = Command::new(env!("CARGO_BIN_EXE_lampl"))
            .args(["import", "--from-postgres", "--project", "mg"])
            .env("LAMPL_FROM_POSTGRES_URL", &url)
            .env("LAMPL_SERVER", &l.url)
            .env("LAMPL_API_KEY", &l.key)
            .env("LAMPL_CONFIG_DIR", l.cfg.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", err(&out));
        out
    };
    let first = run();
    let out: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(
        (
            out["memories_imported"].as_u64(),
            out["links_imported"].as_u64()
        ),
        (Some(3), Some(1))
    );
    assert!(err(&first).contains("RELATED_TO"), "{}", err(&first));
    let again: Value = serde_json::from_slice(&run().stdout).unwrap();
    assert_eq!(
        (
            again["memories_imported"].as_u64(),
            again["links_imported"].as_u64()
        ),
        (Some(0), Some(0))
    );
    let rel = l.ok(&["related", "11111111-1111-4111-8111-111111111111"]);
    assert!(rel.contains("CONTRADICTS") && !rel.contains("SOLVES"));
    assert_eq!(
        json(&l.ok(&["get", "11111111-1111-4111-8111-111111111111"]))["memory_type"],
        "fix"
    );
    l.rt.block_on(async {
        let mut admin = sqlx::PgConnection::connect(&l.admin).await.unwrap();
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {src} WITH (FORCE)"
        )))
        .execute(&mut admin)
        .await;
    });
}

#[test]
fn pg_delete_removes_a_memory_and_its_links_and_a_second_delete_fails() {
    let Some(l) = start(Fake {
        digest: "sha256:a",
        down: false,
    }) else {
        return;
    };
    let ids = populate(&l);
    let out = json(&l.ok(&["delete", &ids[3]]));
    assert_eq!(out["id"], ids[3].as_str());
    assert_eq!(out["links_removed"], 1);
    assert!(!l.ok(&["related", &ids[2]]).contains("SOLVES"));
    assert!(!l.recall_ids("unbound crashed").contains(&ids[3]));
    assert!(l.lampl(&["get", &ids[2]]).status.success());

    let again = l.lampl(&["delete", &ids[3]]);
    assert!(!again.status.success());
    assert!(again.stdout.is_empty());
    assert!(err(&again).contains("not found"), "{}", err(&again));
}
