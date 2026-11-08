// SPDX-License-Identifier: Apache-2.0

//! Integration tests; need `LAMPL_TEST_DATABASE_URL` pointing at a pgvector Postgres, and skip when it is unset.

use lampl_core::{EMBEDDING_DIM, Embedding, LinkType, MemoryType, NewMemory};
use lampl_store_pg::{Direction, PgStore, RecallQuery};
use uuid::Uuid;

async fn store() -> Option<PgStore> {
    let Ok(url) = std::env::var("LAMPL_TEST_DATABASE_URL") else {
        assert!(
            std::env::var("CI").as_deref() != Ok("true"),
            "LAMPL_TEST_DATABASE_URL must be set when CI=true"
        );
        eprintln!("LAMPL_TEST_DATABASE_URL unset; skipping");
        return None;
    };
    Some(PgStore::connect(&url).await.expect("connect"))
}

fn axis(i: usize) -> Vec<f32> {
    let mut v = vec![0.0; EMBEDDING_DIM];
    v[i] = 1.0;
    v
}

fn mix(i: usize, j: usize, wj: f32) -> Vec<f32> {
    let mut v = axis(i);
    v[j] = wj;
    v
}

fn new(project: &str, title: &str, body: &str, vector: Option<Vec<f32>>) -> NewMemory {
    NewMemory {
        memory_type: MemoryType::Fact,
        title: title.to_owned(),
        body: body.to_owned(),
        tags: vec!["test".to_owned()],
        project: project.to_owned(),
        importance: 0.5,
        embedding: vector.map(|vector| Embedding {
            vector,
            model_id: "test-model".to_owned(),
            model_digest: "sha256:test".to_owned(),
        }),
    }
}

fn project() -> String {
    format!("t-{}", Uuid::now_v7())
}

#[tokio::test]
async fn schema_version_is_reported() {
    let Some(s) = store().await else { return };
    assert!(s.schema_version().await.unwrap() >= 1);
}

#[tokio::test]
async fn store_get_link_related_round_trip() {
    let Some(s) = store().await else { return };
    let p = project();
    let problem = s
        .store(new(&p, "disk full", "the volume filled up", Some(axis(0))))
        .await
        .unwrap();
    let fix = s
        .store(new(&p, "expand volume", "grow the lvm volume", None))
        .await
        .unwrap();

    let got = s.get(problem.id).await.unwrap().unwrap();
    assert_eq!(got.title, "disk full");
    assert_eq!(got.project, p);
    assert_eq!(got.embedding_model.as_deref(), Some("test-model"));
    assert!(s.get(Uuid::now_v7()).await.unwrap().is_none());

    s.link(fix.id, problem.id, LinkType::Solves, 0.9)
        .await
        .unwrap();
    let from_problem = s.related(problem.id).await.unwrap();
    assert_eq!(from_problem.len(), 1);
    assert_eq!(from_problem[0].direction, Direction::Incoming);
    assert_eq!(from_problem[0].link.link_type, LinkType::Solves);
    assert_eq!(from_problem[0].memory.id, fix.id);
    let from_fix = s.related(fix.id).await.unwrap();
    assert_eq!(from_fix[0].direction, Direction::Outgoing);

    assert!(s.link(fix.id, fix.id, LinkType::Causes, 1.0).await.is_err());
    assert!(
        s.link(fix.id, Uuid::now_v7(), LinkType::Causes, 1.0)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn superseded_memory_is_excluded_from_recall_but_still_reachable() {
    let Some(s) = store().await else { return };
    let p = project();
    let old = s
        .store(new(
            &p,
            "deploy script path",
            "the deploy script lives in bin",
            Some(axis(1)),
        ))
        .await
        .unwrap();
    let newer = s
        .store(new(
            &p,
            "deploy script path",
            "the deploy script lives in scripts",
            Some(mix(1, 2, 0.1)),
        ))
        .await
        .unwrap();

    let mut q = RecallQuery::new("deploy script path");
    q.project = Some(p.clone());
    q.embedding = Some(axis(1));
    q.embedding_digest = Some("sha256:test".to_owned());
    let before: Vec<_> = s
        .recall(&q)
        .await
        .unwrap()
        .into_iter()
        .map(|h| h.memory.id)
        .collect();
    assert!(before.contains(&old.id) && before.contains(&newer.id));

    s.link(newer.id, old.id, LinkType::Supersedes, 1.0)
        .await
        .unwrap();
    let after: Vec<_> = s
        .recall(&q)
        .await
        .unwrap()
        .into_iter()
        .map(|h| h.memory.id)
        .collect();
    assert_eq!(after, vec![newer.id]);

    assert!(s.get(old.id).await.unwrap().is_some());
    let rel = s.related(old.id).await.unwrap();
    assert_eq!(rel[0].link.link_type, LinkType::Supersedes);
    assert_eq!(rel[0].memory.id, newer.id);
}

#[tokio::test]
async fn unrelated_query_returns_nothing_through_the_floor() {
    let Some(s) = store().await else { return };
    let p = project();
    s.store(new(
        &p,
        "postgres vacuum tuning",
        "autovacuum scale factor for large tables",
        Some(axis(3)),
    ))
    .await
    .unwrap();
    s.store(new(
        &p,
        "nginx upstream timeout",
        "proxy read timeout raised to ninety seconds",
        Some(axis(4)),
    ))
    .await
    .unwrap();

    let mut q = RecallQuery::new("quarterly tax filing deadline");
    q.project = Some(p.clone());
    q.embedding = Some(axis(5));
    q.embedding_digest = Some("sha256:test".to_owned());
    assert!(s.recall(&q).await.unwrap().is_empty());

    let mut relevant = RecallQuery::new("autovacuum scale factor");
    relevant.project = Some(p);
    relevant.embedding = Some(axis(3));
    relevant.embedding_digest = Some("sha256:test".to_owned());
    let hits = s.recall(&relevant).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory.title, "postgres vacuum tuning");
}

#[tokio::test]
async fn survivors_order_by_cosine_plus_coverage_on_a_hand_built_fixture() {
    let Some(s) = store().await else { return };
    let p = project();
    let both = s
        .store(new(
            &p,
            "alpha beta gamma",
            "alpha beta gamma delta",
            Some(mix(6, 7, 0.3)),
        ))
        .await
        .unwrap();
    let text_only = s
        .store(new(&p, "alpha beta", "alpha beta notes", Some(axis(9))))
        .await
        .unwrap();
    let vec_only = s
        .store(new(
            &p,
            "unrelated words",
            "nothing shared here",
            Some(axis(6)),
        ))
        .await
        .unwrap();

    let mut q = RecallQuery::new("alpha beta gamma");
    q.project = Some(p);
    q.embedding = Some(axis(6));
    q.embedding_digest = Some("sha256:test".to_owned());
    q.floor.cosine = 0.0;
    q.floor.coverage = 0.0;
    let ids: Vec<_> = s
        .recall(&q)
        .await
        .unwrap()
        .into_iter()
        .map(|h| h.memory.id)
        .collect();
    assert_eq!(
        ids,
        vec![both.id, vec_only.id, text_only.id],
        "cosine plus coverage: both arms first, then the exact vector match, then the text-only match"
    );
}

#[tokio::test]
async fn wrong_dimension_is_rejected() {
    let Some(s) = store().await else { return };
    let r = s.store(new(&project(), "t", "b", Some(vec![0.0; 3]))).await;
    assert!(r.is_err());
}

#[tokio::test]
async fn connect_tolerates_a_schema_newer_than_this_build() {
    let Ok(url) = std::env::var("LAMPL_TEST_DATABASE_URL") else {
        assert!(
            std::env::var("CI").as_deref() != Ok("true"),
            "LAMPL_TEST_DATABASE_URL must be set when CI=true"
        );
        return;
    };
    PgStore::connect(&url).await.unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES (999, 'from a newer build', true, '\\x00', 0) ON CONFLICT DO NOTHING",
    )
    .execute(&pool)
    .await
    .unwrap();
    let reconnect = PgStore::connect(&url).await;
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 999")
        .execute(&pool)
        .await
        .unwrap();
    assert!(reconnect.is_ok(), "{:?}", reconnect.err());
}

#[tokio::test]
async fn query_terms_match_postgres_lexemes() {
    let Some(s) = store().await else { return };
    for text in lampl_core::PARITY_QUERIES {
        let mut want = s.lexemes(text).await.unwrap();
        want.sort();
        let mut got: Vec<String> = lampl_core::query_terms(text)
            .iter()
            .map(|t| lampl_core::term_stem(t))
            .collect();
        got.sort();
        assert_eq!(got, want, "lexemes differ for {text:?}");
    }
}

#[tokio::test]
async fn stop_words_do_not_count_against_coverage() {
    let Some(s) = store().await else { return };
    let p = project();
    let m = s
        .store(new(
            &p,
            "docker daemon",
            "the docker daemon restarted",
            None,
        ))
        .await
        .unwrap();
    let mut q = RecallQuery::new("the docker daemon to restart on a");
    q.project = Some(p);
    q.floor.cosine = 0.0;
    q.floor.coverage = 0.0;
    let hits = s.recall(&q).await.unwrap();
    assert_eq!(hits[0].memory.id, m.id);
    assert!(
        (hits[0].coverage - 1.0).abs() < 1e-9 || hits[0].coverage > 0.6,
        "coverage {}",
        hits[0].coverage
    );
}
