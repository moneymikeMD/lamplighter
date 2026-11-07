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
fn commands_work_and_a_wrong_key_fails_loudly() {
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
fn doctor_reports_the_store_and_fails_when_degraded() {
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
fn audit_finds_duplicates_untagged_and_skips_superseded() {
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
fn export_to_a_fresh_database_then_import_gives_identical_recall() {
    let Some(src) = start(Fake {
        digest: "sha256:a",
        down: false,
    }) else {
        return;
    };
    let ids = populate(&src);
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
        (Some(4), Some(2))
    );
    let health: Value = serde_json::from_str(&dst.ok(&["doctor", "--json"])).unwrap();
    assert_eq!(
        health["health"]["embedded_count"], 4,
        "vectors arrived, no re-embed needed"
    );
    let after: Vec<Vec<String>> = queries.iter().map(|q| dst.recall_ids(q)).collect();
    assert_eq!(before, after);
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
fn import_refuses_a_model_mismatch_unless_reembed() {
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
fn memorygraph_export_imports_idempotently_keeping_contradicts() {
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
fn a_truncated_export_file_is_refused() {
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
