// SPDX-License-Identifier: Apache-2.0

//! `lampl serve` integration tests. They need `LAMPL_TEST_DATABASE_URL` pointing at a pgvector Postgres superuser and skip when it is unset; each test runs against its own freshly created database.

use std::os::unix::fs::PermissionsExt;
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
        if texts.iter().any(|t| t.contains("POISON")) {
            return Err(EmbedError::Embed("rejects this text".to_owned()));
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
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
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
    let health = call(&h.app, Method::GET, "/health", Some(&h.key), None).await;
    assert_eq!(health.status, StatusCode::OK);
    let health_none = call(&h.app, Method::GET, "/health", None, None).await;
    assert_eq!(health_none.status, StatusCode::UNAUTHORIZED);
    assert!(
        health_none.json().get("memory_count").is_none(),
        "no details without a key"
    );
    let health_wrong = call(&h.app, Method::GET, "/health", Some("lampl_nope"), None).await;
    assert_eq!(health_wrong.status, StatusCode::UNAUTHORIZED);
    let livez = call(&h.app, Method::GET, "/livez", None, None).await;
    assert_eq!(livez.status, StatusCode::OK);
    assert_eq!(livez.json(), json!({"status": "alive"}));

    for r in [
        &none,
        &wrong,
        &post,
        &ok,
        &missing,
        &health,
        &health_none,
        &health_wrong,
        &livez,
    ] {
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

    let health = call(&h.app, Method::GET, "/health", Some(&h.key), None)
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
    let healed = call(&h.app, Method::GET, "/health", Some(&h.key), None)
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
async fn pg_health_notices_an_embedder_that_goes_down_after_a_good_probe() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let embedder = FakeEmbedder::new(false);
    let h = harness(&db, embedder.clone()).await;
    probe(&h.state).await;
    let up = call(&h.app, Method::GET, "/health", Some(&h.key), None)
        .await
        .json();
    assert_eq!(up["degraded"], false);

    embedder.fail.store(true, Ordering::Relaxed);
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    let down = call(&h.app, Method::GET, "/health", Some(&h.key), None)
        .await
        .json();
    assert_eq!(
        down["degraded"], true,
        "no stale ok after the embedder fails"
    );
    assert_eq!(down["embedder_available"], false);
    db.drop_db().await;
}

#[tokio::test]
async fn pg_a_row_the_embedder_rejects_does_not_block_later_rows() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let h = harness(&db, FakeEmbedder::new(false)).await;
    probe(&h.state).await;
    let k = Some(h.key.as_str());
    for (t, b) in [
        ("poison", "POISON text"),
        ("two", "dos"),
        ("three", "tres"),
        ("four", "cuatro"),
    ] {
        let r = call(&h.app, Method::POST, "/v1/memories", k, Some(memory(t, b))).await;
        assert_eq!(r.status, StatusCode::CREATED);
    }
    assert_eq!(embed_pending(&h.state).await.unwrap(), 3);
    assert_eq!(
        embed_pending(&h.state).await.unwrap(),
        0,
        "the poison row is not retried every sweep"
    );

    let health = call(&h.app, Method::GET, "/health", k, None).await.json();
    assert_eq!(health["memory_count"], 4);
    assert_eq!(health["embedded_count"], 3);
    assert_eq!(health["embed_failed_rows"], 1);
    assert_eq!(health["degraded"], true);
    assert_eq!(health["embedder_available"], true);
    db.drop_db().await;
}

#[tokio::test]
async fn pg_empty_store_reports_an_explicit_zero_count() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let h = harness(&db, FakeEmbedder::new(false)).await;
    probe(&h.state).await;
    let health = call(&h.app, Method::GET, "/health", Some(&h.key), None)
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
    // header, two memories, one link, end marker
    assert_eq!(export.bytes.iter().filter(|c| **c == b'\n').count(), 5);

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
    let health = call(&b.app, Method::GET, "/health", Some(&b.key), None)
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

#[test]
fn keys_add_list_revoke_is_hashed_at_rest_and_revocable() {
    let path = std::env::temp_dir().join(format!("lampl-keys-cli-{}", Uuid::now_v7()));
    let key = lampl::serve::add_key(&path, "mac").unwrap();
    let other = lampl::serve::add_key(&path, "desktop").unwrap();
    assert!(
        lampl::serve::add_key(&path, "mac").is_err(),
        "duplicate name"
    );
    assert_eq!(lampl::serve::list_keys(&path).unwrap(), ["desktop", "mac"]);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        !text.contains(&key) && text.contains(&hash_key(&key)),
        "only hashes at rest"
    );
    let store = KeyStore::open(&path).unwrap();
    assert_eq!(store.authenticate(&key).as_deref(), Some("mac"));
    lampl::serve::revoke_key(&path, "mac").unwrap();
    assert!(lampl::serve::revoke_key(&path, "mac").is_err());
    let store = KeyStore::open(&path).unwrap();
    assert!(store.authenticate(&key).is_none());
    assert_eq!(store.authenticate(&other).as_deref(), Some("desktop"));
}

#[derive(clap::Parser)]
struct Wrapper {
    #[command(flatten)]
    cmd: lampl::serve::Cmd,
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
    let err = lampl::serve::run(w.cmd).expect_err("must refuse");
    assert!(err.contains("newer than this build understands"), "{err}");
    rt.block_on(db.drop_db());
}

#[test]
fn keys_file_is_written_owner_only_and_a_looser_one_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(format!("lampl-keys-mode-{}", Uuid::now_v7()));
    let key = lampl::serve::add_key(&path, "mac").unwrap();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    let key2 = lampl::serve::add_key(&path, "desktop").unwrap();
    assert_eq!(mode(&path), 0o600, "a rewrite keeps the file owner-only");
    assert!(KeyStore::open(&path).is_ok());
    for loose in [0o640, 0o604, 0o644, 0o666] {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(loose)).unwrap();
        let err = KeyStore::open(&path).expect_err("must refuse a loose keys file");
        let msg = err.to_string();
        assert!(
            msg.contains("chmod 600") && !msg.contains(&key) && !msg.contains(&key2),
            "{msg}"
        );
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
    assert!(
        KeyStore::open(&path).is_ok(),
        "read-only for the owner is fine"
    );
}

#[test]
fn a_corrupt_keys_file_after_a_revoke_locks_everyone_out_until_it_is_fixed() {
    let path = std::env::temp_dir().join(format!("lampl-keys-corrupt-{}", Uuid::now_v7()));
    let mac = lampl::serve::add_key(&path, "mac").unwrap();
    let desk = lampl::serve::add_key(&path, "desktop").unwrap();
    let store = KeyStore::open(&path).unwrap();
    assert!(store.authenticate(&mac).is_some() && store.authenticate(&desk).is_some());

    lampl::serve::revoke_key(&path, "mac").unwrap();
    std::fs::write(&path, "garbage that is not a keys file\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert!(
        store.authenticate(&mac).is_none(),
        "revoked key must not survive a corrupt reload"
    );
    assert!(
        store.authenticate(&desk).is_none(),
        "fail closed for every key"
    );
    assert!(store.is_failed());

    std::fs::write(&path, format!("desktop {}\n", hash_key(&desk))).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert!(
        store.authenticate(&desk).is_some(),
        "fixing the file restores service"
    );
    assert!(store.authenticate(&mac).is_none());
    assert!(!store.is_failed());
}

#[tokio::test]
async fn pg_failed_keys_reload_is_401_for_a_revoked_key_and_reported_on_health() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let store = lampl_store_pg::PgStore::connect(&db.url).await.unwrap();
    let path = std::env::temp_dir().join(format!("lampl-keys-http-{}", Uuid::now_v7()));
    let mac = lampl::serve::add_key(&path, "mac").unwrap();
    lampl::serve::add_key(&path, "desktop").unwrap();
    let state = AppState::new(
        store,
        FakeEmbedder::new(false),
        EmbedMode::Endpoint,
        KeyStore::open(&path).unwrap(),
        0.95,
    );
    let app = router(Arc::clone(&state));
    let ok = call(&app, Method::GET, "/v1/memories", Some(&mac), None).await;
    assert_eq!(ok.status, StatusCode::OK);

    lampl::serve::revoke_key(&path, "mac").unwrap();
    std::fs::write(&path, "not a keys file\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let r = call(&app, Method::GET, "/v1/memories", Some(&mac), None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let health = call(&app, Method::GET, "/health", Some(&mac), None).await;
    assert_eq!(health.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(health.json()["degraded"], true);
    assert!(
        health.json()["error"]
            .as_str()
            .unwrap()
            .contains("keys file"),
        "{}",
        health.json()
    );
    db.drop_db().await;
}

struct GatedEmbedder {
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl Embedder for GatedEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.iter().any(|t| t.contains("gate")) {
            let _ = self.entered.send(());
            let _ = self.release.lock().unwrap().recv();
        }
        Ok(texts
            .iter()
            .map(|_| {
                let mut v = vec![0.0; lampl_embed::DIM];
                v[0] = 1.0;
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

#[tokio::test]
async fn pg_embedding_holds_no_transaction_or_lock_while_the_embedder_runs_and_a_crash_is_reclaimable()
 {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let h = harness(
        &db,
        Arc::new(GatedEmbedder {
            entered: entered_tx,
            release: std::sync::Mutex::new(release_rx),
        }),
    )
    .await;
    let k = Some(h.key.as_str());
    for (t, b) in [("one", "gate one"), ("two", "gate two")] {
        let r = call(&h.app, Method::POST, "/v1/memories", k, Some(memory(t, b))).await;
        assert_eq!(r.status, StatusCode::CREATED);
    }
    let state = Arc::clone(&h.state);
    let worker = tokio::spawn(async move { embed_pending(&state).await });
    tokio::task::spawn_blocking(move || {
        entered_rx.recv_timeout(std::time::Duration::from_secs(10))
    })
    .await
    .unwrap()
    .expect("embedder entered");

    let mut other = sqlx::PgConnection::connect(&db.url).await.unwrap();
    let idle: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND state LIKE 'idle in transaction%'",
    )
    .fetch_one(&mut other)
    .await
    .unwrap();
    assert_eq!(
        idle, 0,
        "no transaction stays open across the embedder call"
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        sqlx::query("UPDATE memories SET importance = 0.9").execute(&mut other),
    )
    .await
    .expect("rows are not locked while embedding")
    .unwrap();

    worker.abort();
    let _ = worker.await;
    drop(release_tx);
    assert_eq!(
        embed_pending(&h.state).await.unwrap(),
        0,
        "rows claimed by a live lease are not taken twice"
    );
    sqlx::query("UPDATE memories SET embed_claimed_until = now() - interval '1 second'")
        .execute(&mut other)
        .await
        .unwrap();
    assert_eq!(
        embed_pending(&h.state).await.unwrap(),
        2,
        "once the lease lapses a crashed batch is reclaimed"
    );
    let health = call(&h.app, Method::GET, "/health", k, None).await.json();
    assert_eq!(health["embedded_count"], 2);
    db.drop_db().await;
}

#[tokio::test]
async fn pg_an_embedder_outage_releases_its_claims_for_the_next_sweep() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let embedder = FakeEmbedder::new(true);
    let h = harness(&db, embedder.clone()).await;
    let k = Some(h.key.as_str());
    let r = call(
        &h.app,
        Method::POST,
        "/v1/memories",
        k,
        Some(memory("a", "b")),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED);
    assert!(embed_pending(&h.state).await.is_err());
    embedder.fail.store(false, Ordering::Relaxed);
    assert_eq!(embed_pending(&h.state).await.unwrap(), 1);
    db.drop_db().await;
}

#[tokio::test]
async fn pg_vector_recall_ignores_rows_embedded_by_another_model_digest() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let h = harness(&db, FakeEmbedder::new(false)).await;
    let k = Some(h.key.as_str());
    let r = call(
        &h.app,
        Method::POST,
        "/v1/memories",
        k,
        Some(memory("zebra crossing", "stripes on a road")),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED);
    assert_eq!(embed_pending(&h.state).await.unwrap(), 1);
    let query = "qqqq wwww eeee rrrr tttt yyyy uuu";
    assert_eq!(query.len(), "zebra crossing\n\nstripes on a road".len());
    let ask = || json!({"query": query, "limit": 5});
    let before = call(&h.app, Method::POST, "/v1/recall", k, Some(ask())).await;
    assert_eq!(
        before.json()["hits"].as_array().map_or(0, Vec::len),
        1,
        "same-model vector matches"
    );

    let mut other = sqlx::PgConnection::connect(&db.url).await.unwrap();
    sqlx::query("UPDATE memories SET embedding_digest = 'sha256:old-model'")
        .execute(&mut other)
        .await
        .unwrap();
    let after = call(&h.app, Method::POST, "/v1/recall", k, Some(ask())).await;
    assert_eq!(after.status, StatusCode::OK);
    assert_eq!(
        after.json()["hits"].as_array().map_or(0, Vec::len),
        0,
        "a stale-digest vector must not surface a hit"
    );
    db.drop_db().await;
}
