//! `lampl serve` writes tracing to stderr, never stdout, and never emits ANSI when stderr is a pipe.

use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn serve_logs_to_stderr_without_ansi_when_piped() {
    let dir = std::env::temp_dir().join(format!("lampl-serve-logging-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let keys = dir.join("keys");
    std::fs::write(&keys, format!("test {}\n", "0".repeat(64))).unwrap();
    std::fs::set_permissions(&keys, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_lampl"))
        .args(["serve", "--listen", "127.0.0.1:0", "--keys-file"])
        .arg(&keys)
        .args(["--database-url", "postgres://u@127.0.0.1:1/none"])
        .env("RUST_LOG", "trace")
        .env_remove("NO_COLOR")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_secs(2));
    child.kill().ok();
    let out = child.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stdout.is_empty(), "stdout: {stdout}");
    assert!(!stderr.is_empty(), "no log reached stderr");
    assert!(!stderr.contains('\x1b'), "stderr: {stderr:?}");
}
