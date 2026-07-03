//! End-to-end daemon tests (T10): the built binary starts from a config, runs
//! checks, and shuts down gracefully on SIGTERM (V4); a missing config is fatal.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Read;
use std::net::TcpListener;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_synthetics");

fn wait_timeout(child: &mut Child, dur: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        if start.elapsed() > dur {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn help_succeeds_and_names_the_tool() {
    let out = Command::new(BIN).arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("synthetics"),
        "help should name the tool: {text}"
    );
    assert!(text.contains("--config"));
}

#[test]
fn missing_config_is_fatal() {
    let status = Command::new(BIN)
        .args(["--config", "/no/such/synthetics-daemon-test.toml"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success(), "missing config should exit non-zero");
}

#[test]
fn starts_runs_and_shuts_down_on_sigterm() {
    // Healthy TCP target for the daemon to check.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let dir = std::env::temp_dir().join("synthetics-daemon-test");
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("checks.toml");
    std::fs::write(
        &config,
        format!(
            "[[endpoint]]\nid = \"smoke\"\ninterval = \"80ms\"\ntimeout = \"1s\"\n  [endpoint.tcp]\n  host = \"127.0.0.1\"\n  port = {port}\n"
        ),
    )
    .unwrap();

    let mut child = Command::new(BIN)
        .args(["--config", config.to_str().unwrap(), "--log-format", "json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Let it run a few check cycles.
    std::thread::sleep(Duration::from_millis(400));
    assert!(
        child.try_wait().unwrap().is_none(),
        "daemon should still be running"
    );

    // SIGTERM -> graceful shutdown.
    let pid = child.id() as libc::pid_t;
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }

    let status = wait_timeout(&mut child, Duration::from_secs(5)).unwrap_or_else(|| {
        let _ = child.kill();
        panic!("daemon did not exit after SIGTERM");
    });
    assert!(
        status.success(),
        "graceful shutdown should exit 0, got {status:?}"
    );

    // The daemon logged that it ran checks (structured JSON on stdout).
    let mut stdout = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }
    assert!(
        stdout.contains("synthetics daemon started"),
        "expected startup log, got: {stdout}"
    );
}

#[test]
fn control_plane_mode_is_fatal_when_cp_unreachable() {
    // Initial config load from an unreachable control plane is fatal (T31).
    let status = Command::new(BIN)
        .args([
            "--control-plane",
            "http://127.0.0.1:1",
            "--worker-id",
            "w-test",
            "--region",
            "eu",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "unreachable CP at startup should exit non-zero"
    );
}
