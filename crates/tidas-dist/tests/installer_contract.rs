//! Contract tests for the POSIX installer in `scripts/install.sh`.
//!
//! These tests stub `uname` and `curl` on `PATH`, so platform selection is
//! proven without network access and without a macOS Intel runner. The retired
//! `Darwin x86_64` tuple must be rejected before any download attempt.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const INSTALLER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/install.sh");

struct StubEnv {
    stub_dir: PathBuf,
    curl_log: PathBuf,
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn stub_environment(system: &str, machine: &str, temporary: &Path) -> StubEnv {
    let stub_dir = temporary.join("stubs");
    fs::create_dir_all(&stub_dir).unwrap();
    write_executable(
        &stub_dir.join("uname"),
        &format!(
            "#!/bin/sh\ncase \"$1\" in\n  -m) printf '%s\\n' '{machine}' ;;\n  *) printf '%s\\n' '{system}' ;;\nesac\n"
        ),
    );
    let curl_log = temporary.join("curl.log");
    write_executable(
        &stub_dir.join("curl"),
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$TIDAS_INSTALLER_TEST_CURL_LOG\"\nexit 1\n",
    );
    StubEnv { stub_dir, curl_log }
}

fn run_installer(stub_dir: &Path, curl_log: &Path) -> (std::process::ExitStatus, String, String) {
    let existing_path = std::env::var("PATH").unwrap_or_default();
    let output = Command::new("sh")
        .arg(INSTALLER)
        .arg("--version")
        .arg("0.1.0")
        .env("PATH", format!("{}:{}", stub_dir.display(), existing_path))
        .env("TIDAS_INSTALLER_TEST_CURL_LOG", curl_log)
        .output()
        .expect("installer test runs sh");
    (
        output.status,
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn posix_installer_rejects_macos_intel_before_any_download() {
    let temporary = tempfile::tempdir().unwrap();
    let stubs = stub_environment("Darwin", "x86_64", temporary.path());
    let (status, _stdout, stderr) = run_installer(&stubs.stub_dir, &stubs.curl_log);
    assert_eq!(status.code(), Some(1));
    assert!(stderr.contains("macOS Intel"), "stderr was: {stderr}");
    assert!(
        !stubs.curl_log.exists(),
        "the installer must reject macOS Intel before any download attempt"
    );
}

#[test]
fn posix_installer_still_selects_linux_x64() {
    let temporary = tempfile::tempdir().unwrap();
    let stubs = stub_environment("Linux", "x86_64", temporary.path());
    let (status, _stdout, _stderr) = run_installer(&stubs.stub_dir, &stubs.curl_log);
    assert_eq!(status.code(), Some(1));
    let log = fs::read_to_string(&stubs.curl_log).unwrap();
    assert!(
        log.contains("tidas-v0.1.0-x86_64-unknown-linux-gnu.tar.gz"),
        "installer selected: {log}"
    );
}

#[test]
fn posix_installer_still_selects_macos_arm64() {
    let temporary = tempfile::tempdir().unwrap();
    let stubs = stub_environment("Darwin", "arm64", temporary.path());
    let (status, _stdout, _stderr) = run_installer(&stubs.stub_dir, &stubs.curl_log);
    assert_eq!(status.code(), Some(1));
    let log = fs::read_to_string(&stubs.curl_log).unwrap();
    assert!(
        log.contains("tidas-v0.1.0-aarch64-apple-darwin.tar.gz"),
        "installer selected: {log}"
    );
}
