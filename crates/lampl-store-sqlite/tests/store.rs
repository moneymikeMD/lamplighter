// SPDX-License-Identifier: Apache-2.0

//! Store tests mirroring the Postgres suite on hand-built vectors; they run in memory and need no services.

use lampl_core::{EMBEDDING_DIM, Embedding, LinkType, MemoryType, NewMemory};
use lampl_store_sqlite::{Direction, RecallQuery, SqliteStore};
use uuid::Uuid;

fn store() -> SqliteStore {
    SqliteStore::open_in_memory().expect("open")
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

#[test]
fn schema_version_is_reported() {
    let s = store();
    assert!(s.schema_version().unwrap() >= 1);
}

#[test]
fn store_get_link_related_round_trip() {
    let mut s = store();
    let p = project();
    let problem = s
        .store(new(&p, "disk full", "the volume filled up", Some(axis(0))))
        .unwrap();
    let fix = s
        .store(new(&p, "expand volume", "grow the lvm volume", None))
        .unwrap();

    let got = s.get(problem.id).unwrap().unwrap();
    assert_eq!(got.title, "disk full");
    assert_eq!(got.project, p);
    assert_eq!(got.embedding_model.as_deref(), Some("test-model"));
    assert!(s.get(Uuid::now_v7()).unwrap().is_none());

    s.link(fix.id, problem.id, LinkType::Solves, 0.9).unwrap();
    let from_problem = s.related(problem.id).unwrap();
    assert_eq!(from_problem.len(), 1);
    assert_eq!(from_problem[0].direction, Direction::Incoming);
    assert_eq!(from_problem[0].link.link_type, LinkType::Solves);
    assert_eq!(from_problem[0].memory.id, fix.id);
    let from_fix = s.related(fix.id).unwrap();
    assert_eq!(from_fix[0].direction, Direction::Outgoing);

    assert!(s.link(fix.id, fix.id, LinkType::Causes, 1.0).is_err());
    assert!(
        s.link(fix.id, Uuid::now_v7(), LinkType::Causes, 1.0)
            .is_err()
    );
}

#[test]
fn superseded_memory_is_excluded_from_recall_but_still_reachable() {
    let mut s = store();
    let p = project();
    let old = s
        .store(new(
            &p,
            "deploy script path",
            "the deploy script lives in bin",
            Some(axis(1)),
        ))
        .unwrap();
    let newer = s
        .store(new(
            &p,
            "deploy script path",
            "the deploy script lives in scripts",
            Some(mix(1, 2, 0.1)),
        ))
        .unwrap();

    let mut q = RecallQuery::new("deploy script path");
    q.project = Some(p.clone());
    q.embedding = Some(axis(1));
    let before: Vec<_> = s
        .recall(&q)
        .unwrap()
        .into_iter()
        .map(|h| h.memory.id)
        .collect();
    assert!(before.contains(&old.id) && before.contains(&newer.id));

    s.link(newer.id, old.id, LinkType::Supersedes, 1.0).unwrap();
    let after: Vec<_> = s
        .recall(&q)
        .unwrap()
        .into_iter()
        .map(|h| h.memory.id)
        .collect();
    assert_eq!(after, vec![newer.id]);

    assert!(s.get(old.id).unwrap().is_some());
    let rel = s.related(old.id).unwrap();
    assert_eq!(rel[0].link.link_type, LinkType::Supersedes);
    assert_eq!(rel[0].memory.id, newer.id);
}

#[test]
fn unrelated_query_returns_nothing_through_the_floor() {
    let mut s = store();
    let p = project();
    s.store(new(
        &p,
        "postgres vacuum tuning",
        "autovacuum scale factor for large tables",
        Some(axis(3)),
    ))
    .unwrap();
    s.store(new(
        &p,
        "nginx upstream timeout",
        "proxy read timeout raised to ninety seconds",
        Some(axis(4)),
    ))
    .unwrap();

    let mut q = RecallQuery::new("quarterly tax filing deadline");
    q.project = Some(p.clone());
    q.embedding = Some(axis(5));
    assert!(s.recall(&q).unwrap().is_empty());

    let mut relevant = RecallQuery::new("autovacuum scale factor");
    relevant.project = Some(p);
    relevant.embedding = Some(axis(3));
    let hits = s.recall(&relevant).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory.title, "postgres vacuum tuning");
}

#[test]
fn survivors_order_by_cosine_plus_coverage_on_a_hand_built_fixture() {
    let mut s = store();
    let p = project();
    let both = s
        .store(new(
            &p,
            "alpha beta gamma",
            "alpha beta gamma delta",
            Some(mix(6, 7, 0.3)),
        ))
        .unwrap();
    let text_only = s
        .store(new(&p, "alpha beta", "alpha beta notes", Some(axis(9))))
        .unwrap();
    let vec_only = s
        .store(new(
            &p,
            "unrelated words",
            "nothing shared here",
            Some(axis(6)),
        ))
        .unwrap();

    let mut q = RecallQuery::new("alpha beta gamma");
    q.project = Some(p);
    q.embedding = Some(axis(6));
    q.floor.cosine = 0.0;
    q.floor.coverage = 0.0;
    let ids: Vec<_> = s
        .recall(&q)
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

#[test]
fn wrong_dimension_is_rejected() {
    let mut s = store();
    let r = s.store(new(&project(), "t", "b", Some(vec![0.0; 3])));
    assert!(r.is_err());
}

#[test]
fn file_database_persists_across_opens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.db");
    let id = {
        let mut s = SqliteStore::open(&path).unwrap();
        s.store(new("p", "persisted note", "survives reopen", Some(axis(2))))
            .unwrap()
            .id
    };
    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.get(id).unwrap().unwrap().title, "persisted note");
    let mut q = RecallQuery::new("persisted");
    q.embedding = Some(axis(2));
    assert_eq!(s.recall(&q).unwrap().len(), 1);
    assert_eq!(s.schema_version().unwrap(), 1);
}

#[test]
fn sqlite_vec_is_the_pinned_version() {
    assert_eq!(store().vec_version().unwrap(), "v0.1.9");
}

#[test]
fn full_text_only_recall_works_without_an_embedding() {
    let mut s = store();
    s.store(new(
        "p",
        "rotating logs",
        "logrotate keeps seven days",
        None,
    ))
    .unwrap();
    s.store(new("p", "tls renewal", "certbot renews monthly", None))
        .unwrap();
    let hits = s.recall(&RecallQuery::new("logrotate days")).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory.title, "rotating logs");
    assert!(hits[0].cosine.is_none());
}

#[test]
fn tags_and_quotes_survive_and_odd_queries_do_not_error() {
    let mut s = store();
    let m = s
        .store(new("p", "quote \"test\"", "a body with 'quotes'", None))
        .unwrap();
    assert_eq!(s.get(m.id).unwrap().unwrap().tags, vec!["test".to_owned()]);
    assert!(
        s.recall(&RecallQuery::new("\"quote\" OR NEAR( ) *"))
            .is_ok()
    );
}
