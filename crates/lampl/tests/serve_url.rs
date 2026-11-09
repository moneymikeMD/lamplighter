// SPDX-License-Identifier: Apache-2.0

//! `lampl serve` refuses an empty or anonymous database user before connecting, and never prints the password.

use std::process::Command;

fn serve(url: &str) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_lampl"))
        .args(["serve"])
        .env("LAMPL_DATABASE_URL", url)
        .env_remove("PGUSER")
        .env_remove("LAMPL_KEYS_FILE")
        .output()
        .expect("run lampl");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn serve_refuses_a_url_without_a_user_and_names_the_socket_form() {
    let (ok, err) = serve("postgresql:///lampl?host=/tmp");
    assert!(!ok);
    assert!(err.contains("postgresql://<user>@%2Ftmp/lampl"), "{err}");
}

#[test]
fn serve_refuses_a_user_less_url_with_a_password_without_printing_it() {
    let (ok, err) = serve("postgres://:s3cret@%2Ftmp/lampl");
    assert!(!ok);
    assert!(
        err.contains("no usable user") && !err.contains("s3cret"),
        "{err}"
    );
}

#[test]
fn serve_refuses_pguser_anonymous_too() {
    let out = Command::new(env!("CARGO_BIN_EXE_lampl"))
        .args(["serve"])
        .env("LAMPL_DATABASE_URL", "postgresql:///lampl?host=/tmp")
        .env("PGUSER", "anonymous")
        .env_remove("LAMPL_KEYS_FILE")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no usable user"));
}
