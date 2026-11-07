// SPDX-License-Identifier: Apache-2.0

//! `lampl serve` integration tests. They need `LAMPL_TEST_DATABASE_URL` pointing at a pgvector Postgres superuser and skip when it is unset; each test runs against its own freshly created database.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use clap::Parser;
use http_body_util::BodyExt;
use lampl::serve::{
    API_VERSION, API_VERSION_HEADER, AppState, EmbedMode, KeyStore, SharedEmbedder, embed_pending,
    generate_key, hash_key, probe, router,
};
use lampl_embed::{EmbedError, Embedder};
use serde_json::{Value, json};
use sqlx::Connection;
use tower::ServiceExt;
use uuid::Uuid;

#[derive(Debug)]
struct FakeEmbedder {
    fail: AtomicBool,
}

impl FakeEmbedder {
    fn new(fail: bool) -> Arc<Self> {
        Arc::new(FakeEmbedder {
            fail: AtomicBool::new(fail),
        })
    }
}

impl Embedder for FakeEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if self.fail.load(Ordering::Relaxed) {
            return Err(EmbedError::Endpoint("fake embedder is down".to_owned()));
        }
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0; lampl_embed::DIM];
                v[t.len() % lampl_embed::DIM] = 1.0;
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
        "sha256:fake"
    }
}

struct TestDb {
    admin_url: String,
    url: String,
    name: String,
}

impl TestDb {
    async fn create() -> Option<TestDb> {
        let Ok(admin_url) = std::env::var("LAMPL_TEST_DATABASE_URL") else {
            assert!(
                std::env::var("CI").as_deref() != Ok("true"),
                "LAMPL_TEST_DATABASE_URL must be set when CI=true"
            );
            eprintln!("LAMPL_TEST_DATABASE_URL unset; skipping");
            return None;
        };
        let name = format!("lampl_t_{}", Uuid::now_v7().simple());
        let (base, query) = match admin_url.split_once('?') {
            Some((b, q)) => (b.to_owned(), format!("?{q}")),
            None => (admin_url.clone(), String::new()),
        };
        let root = base.rsplit_once('/').expect("url has a database").0;
        let mut conn = sqlx::PgConnection::connect(&admin_url)
            .await
            .expect("admin connect");
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&mut conn)
            .await
            .expect("create database");
        Some(TestDb {
            url: format!("{root}/{name}{query}"),
            admin_url,
            name,
        })
    }

    async fn drop_db(self) {
        let mut conn = sqlx::PgConnection::connect(&self.admin_url)
            .await
            .expect("admin connect");
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            self.name
        )))
        .execute(&mut conn)
        .await;
    }
}

fn keys_file(key: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("lampl-keys-{}", Uuid::now_v7()));
    std::fs::write(&path, format!("# test\ntester {}\n", hash_key(key))).unwrap();
    path
}

struct Harness {
    state: Arc<AppState>,
    app: Router,
    key: String,
}

async fn harness(db: &TestDb, embedder: SharedEmbedder) -> Harness {
    let store = lampl_store_pg::PgStore::connect(&db.url)
        .await
        .expect("connect");
    let key = generate_key().unwrap();
    let keys = KeyStore::open(&keys_file(&key)).unwrap();
    let state = AppState::new(store, embedder, EmbedMode::Endpoint, keys, 0.95);
    Harness {
        app: router(Arc::clone(&state)),
        state,
        key,
    }
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    bytes: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).expect("json body")
    }
}

async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    key: Option<&str>,
    body: Option<Value>,
) -> Reply {
    send(
        app,
        method,
        uri,
        key,
        body.map_or_else(Vec::new, |b| b.to_string().into_bytes()),
    )
    .await
}

async fn send(app: &Router, method: Method, uri: &str, key: Option<&str>, body: Vec<u8>) -> Reply {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(k) = key {
        req = req.header("authorization", format!("Bearer {k}"));
    }
    if !body.is_empty() {
        req = req.header("content-type", "application/json");
    }
    let resp = app
        .clone()
        .oneshot(req.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let (parts, body) = resp.into_parts();
    Reply {
        status: parts.status,
        headers: parts.headers,
        bytes: body.collect().await.unwrap().to_bytes().to_vec(),
    }
}

fn memory(title: &str, body: &str) -> Value {
    json!({"memory_type": "fact", "title": title, "body": body, "tags": ["t"], "project": "p"})
}

#[tokio::test]
async fn pg_missing_or_wrong_key_is_401_and_every_response_carries_the_api_version() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let h = harness(&db, FakeEmbedder::new(false)).await;
    let expected = API_VERSION.to_string();

    let none = call(&h.app, Method::GET, "/v1/memories", None, None).await;
    assert_eq!(none.status, StatusCode::UNAUTHORIZED);
    let wrong = call(
        &h.app,
        Method::GET,
        "/v1/memories",
        Some("lampl_nope"),
        None,
    )
    .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    let post = call(
        &h.app,
        Method::POST,
        "/v1/memories",
        None,
        Some(memory("a", "b")),
    )
    .await;
    assert_eq!(post.status, StatusCode::UNAUTHORIZED);

    let ok = call(
        &h.app,
        Method::POST,
        "/v1/memories",
        Some(&h.key),
        Some(memory("a", "b")),
    )
    .await;
    assert_eq!(ok.status, StatusCode::CREATED);
    let missing = call(&h.app, Method::GET, "/nope", None, None).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    let health = call(&h.app, Method::GET, "/health", None, None).await;
    assert_eq!(health.status, StatusCode::OK);

    for r in [&none, &wrong, &post, &ok, &missing, &health] {
        assert_eq!(
            r.headers.get(API_VERSION_HEADER).unwrap(),
            expected.as_str()
        );
    }
    drop(h);
    db.drop_db().await;
}

#[tokio::test]
async fn pg_health_reports_degraded_when_the_embedder_fails_and_recovers() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let embedder = FakeEmbedder::new(true);
    let h = harness(&db, embedder.clone()).await;

    probe(&h.state).await;
    let stored = call(
        &h.app,
        Method::POST,
        "/v1/memories",
        Some(&h.key),
        Some(memory("zebra crossing", "stripes on the road")),
    )
    .await;
    assert_eq!(
        stored.status,
        StatusCode::CREATED,
        "store must not need the embedder"
    );
    assert!(stored.json()["embedding_model"].is_null());
    assert!(embed_pending(&h.state).await.is_err());

    let health = call(&h.app, Method::GET, "/health", None, None)
        .await
        .json();
    assert_eq!(health["degraded"], true);
    assert_eq!(health["status"], "degraded");
    assert_eq!(health["embedder_available"], false);
    assert_eq!(health["memory_count"], 1);
    assert_eq!(health["vector_coverage"], 0.0);
    assert_eq!(health["embed_mode"], "endpoint");
    assert!(health["schema_version"].as_i64().unwrap() >= 1);
    assert_eq!(health["api_version"], API_VERSION);

    let recall = call(
        &h.app,
        Method::POST,
        "/v1/recall",
        Some(&h.key),
        Some(json!({"query": "zebra stripes"})),
    )
    .await
    .json();
    assert_eq!(recall["mode"], "full-text");
    assert_eq!(
        recall["hits"].as_array().unwrap().len(),
        1,
        "full-text still finds rows not yet embedded"
    );

    embedder.fail.store(false, Ordering::Relaxed);
    probe(&h.state).await;
    assert_eq!(embed_pending(&h.state).await.unwrap(), 1);
    let healed = call(&h.app, Method::GET, "/health", None, None)
        .await
        .json();
    assert_eq!(healed["degraded"], false);
    assert_eq!(healed["status"], "ok");
    assert_eq!(healed["vector_coverage"], 1.0);
    assert_eq!(healed["embedded_count"], 1);
    let recall = call(
        &h.app,
        Method::POST,
        "/v1/recall",
        Some(&h.key),
        Some(json!({"query": "zebra stripes"})),
    )
    .await
    .json();
    assert_eq!(recall["mode"], "hybrid");
    db.drop_db().await;
}

#[tokio::test]
async fn pg_empty_store_reports_an_explicit_zero_count() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let h = harness(&db, FakeEmbedder::new(false)).await;
    probe(&h.state).await;
    let health = call(&h.app, Method::GET, "/health", None, None)
        .await
        .json();
    assert_eq!(health["memory_count"], 0);
    assert_eq!(health["embedded_count"], 0);
    assert_eq!(health["status"], "ok");
    db.drop_db().await;
}

#[tokio::test]
async fn pg_store_get_link_related_and_list_round_trip() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let h = harness(&db, FakeEmbedder::new(false)).await;
    let k = Some(h.key.as_str());
    let a = call(
        &h.app,
        Method::POST,
        "/v1/memories",
        k,
        Some(memory("alpha", "first")),
    )
    .await
    .json();
    let b = call(
        &h.app,
        Method::POST,
        "/v1/memories",
        k,
        Some(memory("beta", "second")),
    )
    .await
    .json();
    let (ida, idb) = (a["id"].as_str().unwrap(), b["id"].as_str().unwrap());

    let got = call(&h.app, Method::GET, &format!("/v1/memories/{ida}"), k, None).await;
    assert_eq!(got.json()["title"], "alpha");
    let unknown = call(
        &h.app,
        Method::GET,
        &format!("/v1/memories/{}", Uuid::now_v7()),
        k,
        None,
    )
    .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);

    let link = call(
        &h.app,
        Method::POST,
        "/v1/links",
        k,
        Some(json!({"from_id": idb, "to_id": ida, "link_type": "SOLVES"})),
    )
    .await;
    assert_eq!(link.status, StatusCode::CREATED);
    let bad = call(
        &h.app,
        Method::POST,
        "/v1/links",
        k,
        Some(json!({"from_id": idb, "to_id": ida, "link_type": "RELATES_TO"})),
    )
    .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let selfie = call(
        &h.app,
        Method::POST,
        "/v1/links",
        k,
        Some(json!({"from_id": ida, "to_id": ida, "link_type": "SOLVES"})),
    )
    .await;
    assert_eq!(selfie.status, StatusCode::BAD_REQUEST);

    let rel = call(
        &h.app,
        Method::GET,
        &format!("/v1/memories/{ida}/related"),
        k,
        None,
    )
    .await
    .json();
    assert_eq!(rel[0]["direction"], "incoming");
    assert_eq!(rel[0]["memory"]["id"], idb);

    let page1 = call(
        &h.app,
        Method::GET,
        "/v1/memories?project=p&limit=1",
        k,
        None,
    )
    .await
    .json();
    assert_eq!(page1["memories"][0]["id"], ida);
    let after = page1["next_after"].as_str().unwrap();
    let page2 = call(
        &h.app,
        Method::GET,
        &format!("/v1/memories?project=p&limit=1&after={after}"),
        k,
        None,
    )
    .await
    .json();
    assert_eq!(page2["memories"][0]["id"], idb);

    let invalid = call(
        &h.app,
        Method::POST,
        "/v1/memories",
        k,
        Some(json!({"memory_type": "general", "title": "x", "body": "y", "project": "p"})),
    )
    .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    assert!(
        invalid.json()["error"]
            .as_str()
            .unwrap()
            .contains("fact, decision")
    );
    db.drop_db().await;
}

#[tokio::test]
async fn pg_export_then_import_reproduces_memories_and_links() {
    let (Some(src), Some(dst)) = (TestDb::create().await, TestDb::create().await) else {
        return;
    };
    let a = harness(&src, FakeEmbedder::new(false)).await;
    let b = harness(&dst, FakeEmbedder::new(false)).await;
    let (ka, kb) = (Some(a.key.as_str()), Some(b.key.as_str()));
    let m1 = call(
        &a.app,
        Method::POST,
        "/v1/memories",
        ka,
        Some(memory("one", "uno")),
    )
    .await
    .json();
    let m2 = call(
        &a.app,
        Method::POST,
        "/v1/memories",
        ka,
        Some(memory("two", "dos")),
    )
    .await
    .json();
    call(
        &a.app,
        Method::POST,
        "/v1/links",
        ka,
        Some(json!({"from_id": m2["id"], "to_id": m1["id"], "link_type": "SUPERSEDES"})),
    )
    .await;

    let export = call(&a.app, Method::GET, "/v1/export", ka, None).await;
    assert_eq!(export.status, StatusCode::OK);
    assert_eq!(export.bytes.iter().filter(|c| **c == b'\n').count(), 3);

    let first = send(&b.app, Method::POST, "/v1/import", kb, export.bytes.clone())
        .await
        .json();
    assert_eq!(first["memories_imported"], 2);
    assert_eq!(first["links_imported"], 1);
    let again = send(&b.app, Method::POST, "/v1/import", kb, export.bytes.clone())
        .await
        .json();
    assert_eq!(again["memories_imported"], 0);
    assert_eq!(again["memories_skipped"], 2);
    assert_eq!(again["links_skipped"], 1);

    let id1 = m1["id"].as_str().unwrap();
    let got = call(
        &b.app,
        Method::GET,
        &format!("/v1/memories/{id1}"),
        kb,
        None,
    )
    .await
    .json();
    assert_eq!(got["title"], "one");
    assert_eq!(got["created_at"], m1["created_at"]);
    let health = call(&b.app, Method::GET, "/health", None, None)
        .await
        .json();
    assert_eq!(health["memory_count"], 2);
    assert_eq!(
        health["embedded_count"], 0,
        "imported rows wait for the background embedder"
    );
    assert_eq!(embed_pending(&b.state).await.unwrap(), 2);

    let junk = send(
        &b.app,
        Method::POST,
        "/v1/import",
        kb,
        b"not json\n".to_vec(),
    )
    .await;
    assert_eq!(junk.status, StatusCode::BAD_REQUEST);
    drop((a, b));
    src.drop_db().await;
    dst.drop_db().await;
}

#[derive(clap::Parser)]
struct Wrapper {
    #[command(flatten)]
    args: lampl::serve::Args,
}

#[test]
fn pg_serve_refuses_to_start_against_a_newer_schema() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let Some(db) = rt.block_on(TestDb::create()) else {
        return;
    };
    rt.block_on(async {
        let store = lampl_store_pg::PgStore::connect(&db.url).await.unwrap();
        sqlx::query("INSERT INTO schema_version (version) VALUES (9999)")
            .execute(store.pool())
            .await
            .unwrap();
    });
    let keys = keys_file("lampl_unused");
    let w = Wrapper::parse_from([
        "t",
        "--database-url",
        &db.url,
        "--keys-file",
        keys.to_str().unwrap(),
        "--listen",
        "127.0.0.1:0",
    ]);
    let err = lampl::serve::run(w.args).expect_err("must refuse");
    assert!(err.contains("newer than this build understands"), "{err}");
    rt.block_on(db.drop_db());
}
