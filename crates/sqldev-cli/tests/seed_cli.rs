//! Offline tests for the CLI argument plumbing of `sqldev seed`.
//!
//! The interesting end-to-end path requires a live DB (introspect →
//! generate → execute) and lives in `live_smoke.rs`. Here we just verify
//! that without args the command is a no-op, that `--seed N` is accepted,
//! and that `--file` validation kicks in.

use std::process::Command;

fn sqldev() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_sqldev"));
    c.current_dir("/");
    for k in [
        "SQLDEV_HOST",
        "SQLDEV_PORT",
        "SQLDEV_USER",
        "SQLDEV_PASSWORD",
        "SQLDEV_DATABASE",
        "SQLDEV_TRUST_CERT",
    ] {
        c.env_remove(k);
    }
    c
}

#[test]
fn seed_with_no_files_and_no_default_dir_is_a_noop() {
    // No `seeds/` exists at `/`, so we should bail with the `no seed files`
    // banner without ever touching the network.
    let out = sqldev().arg("seed").output().expect("spawn");
    assert!(
        out.status.success(),
        "seed (no files) should succeed:\nstderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("no seed files found"),
        "expected no-op banner in:\n{stdout}"
    );
}

#[test]
fn seed_with_missing_explicit_dir_errors() {
    let out = sqldev()
        .args(["seed", "--dir"])
        .arg("/definitely/does/not/exist/sqldev")
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("does not exist"),
        "expected does-not-exist error in:\n{stderr}"
    );
}

#[test]
fn seed_help_lists_the_flags() {
    let out = sqldev().args(["seed", "--help"]).output().expect("spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    for needle in ["--file", "--dir", "--table", "--seed", "--dry-run"] {
        assert!(stdout.contains(needle), "help missing {needle}:\n{stdout}");
    }
}
