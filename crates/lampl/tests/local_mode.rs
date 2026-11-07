// SPDX-License-Identifier: Apache-2.0

//! Local mode exists only when chosen: init creates it, nothing else does, and a server client fails closed.

use std::path::Path;
use std::process::{Command, Output};

fn lampl(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lampl"))
        .args(args)
        .env("LAMPL_CONFIG_DIR", dir)
        .env("LAMPL_DATA_DIR", dir)
        .output()
        .expect("run lampl")
}

fn entries(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn init_local_creates_database_and_records_mode() {
    let d = tempfile::tempdir().unwrap();
    let out = lampl(d.path(), &["init", "--local"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let names = entries(d.path());
    assert!(names.iter().any(|n| n == "lampl.db"), "{names:?}");
    let cfg = std::fs::read_to_string(d.path().join("config.toml")).unwrap();
    assert!(cfg.contains("mode = \"local\""), "{cfg}");
    assert!(cfg.contains("lampl.db"), "{cfg}");
}

#[test]
fn unconfigured_client_errors_and_creates_nothing() {
    let d = tempfile::tempdir().unwrap();
    for args in [
        &["recall", "anything"][..],
        &["store", "--title", "t", "--body", "b"],
    ] {
        let out = lampl(d.path(), args);
        assert!(!out.status.success());
        assert!(
            stderr(&out).contains("run `lampl init`"),
            "{}",
            stderr(&out)
        );
    }
    assert!(entries(d.path()).is_empty());
}

#[test]
fn server_configured_client_with_server_down_fails_closed_and_creates_no_database() {
    let d = tempfile::tempdir().unwrap();
    let out = lampl(d.path(), &["init", "--server", "http://127.0.0.1:1"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let before = entries(d.path());
    assert_eq!(before, ["config.toml"]);

    for args in [
        &["recall", "anything"][..],
        &["store", "--title", "t", "--body", "b", "--no-embed"],
    ] {
        let out = lampl(d.path(), args);
        assert!(!out.status.success());
        assert!(
            stderr(&out).contains("not falling back"),
            "{}",
            stderr(&out)
        );
    }
    assert_eq!(entries(d.path()), before, "data dir must be unchanged");
}

#[test]
fn init_never_switches_modes_silently() {
    let d = tempfile::tempdir().unwrap();
    assert!(
        lampl(d.path(), &["init", "--server", "http://127.0.0.1:1"])
            .status
            .success()
    );
    let out = lampl(d.path(), &["init", "--local"]);
    assert!(!out.status.success());
    assert!(!entries(d.path()).iter().any(|n| n.ends_with(".db")));
}

#[test]
fn deleted_database_is_an_error_not_a_fresh_empty_store() {
    let d = tempfile::tempdir().unwrap();
    assert!(lampl(d.path(), &["init", "--local"]).status.success());
    std::fs::remove_file(d.path().join("lampl.db")).unwrap();
    let out = lampl(d.path(), &["recall", "x", "--no-embed"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("missing"), "{}", stderr(&out));
    assert!(!d.path().join("lampl.db").exists());
}

#[test]
fn store_then_recall_round_trips_by_full_text() {
    let d = tempfile::tempdir().unwrap();
    assert!(lampl(d.path(), &["init", "--local"]).status.success());
    let out = lampl(
        d.path(),
        &[
            "store",
            "--title",
            "backup window",
            "--body",
            "nightly restic at two",
            "--no-embed",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let out = lampl(d.path(), &["recall", "restic nightly", "--no-embed"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("backup window"), "{stdout}");
}

#[test]
fn recall_json_prints_one_object_and_an_empty_recall_is_an_empty_hits_array() {
    let d = tempfile::tempdir().unwrap();
    assert!(lampl(d.path(), &["init", "--local"]).status.success());
    let out = lampl(d.path(), &["recall", "--no-embed", "--json", "nothing"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v, serde_json::json!({"hits": [], "warning": null}));
}
