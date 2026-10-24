//! The `gentboxd` binary: start, serve, a second hub refused, clean shutdown.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

struct Daemon {
    child: Child,
    state: PathBuf,
}

impl Daemon {
    fn start(state: &Path) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_gentboxd"))
            .args(["--state-dir"])
            .arg(state)
            .args(["--config-dir"])
            .arg(state)
            .env("GENTBOX_LOG", "warn")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let daemon = Self {
            child,
            state: state.to_path_buf(),
        };
        daemon.wait_for(|| daemon.state.join("API/port").exists());
        daemon
    }

    fn wait_for(&self, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "gentboxd did not get there in time"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn port(&self) -> u16 {
        std::fs::read_to_string(self.state.join("API/port"))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    fn token(&self) -> String {
        std::fs::read_to_string(self.state.join("API/token")).unwrap()
    }

    fn terminate(mut self) -> std::process::ExitStatus {
        let killed = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .unwrap();
        assert!(killed.success());
        self.child.wait().unwrap()
    }
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[tokio::test(flavor = "multi_thread")]
async fn gentboxd_serves_refuses_a_second_hub_and_cleans_up_on_sigterm() {
    let directory = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(directory.path());

    // Discovery: folder 0700, token and port 0600.
    let api = directory.path().join("API");
    assert_eq!(mode(&api), 0o700);
    assert_eq!(mode(&api.join("token")), 0o600);
    assert_eq!(mode(&api.join("port")), 0o600);
    assert_eq!(mode(&api.join("hub.sock")), 0o600);

    // It serves, and without a UI nothing can answer.
    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{}", daemon.port());
    let posted: Value = client
        .post(format!("{base}/v1/messages"))
        .bearer_auth(daemon.token())
        .json(&json!({
            "version": 1, "source": { "agent": "claude-code" },
            "conversation": { "id": "c1" }, "type": "decision_required", "blocking": "blocking",
            "title": "Allow Bash to run the tests?", "awaitsResponse": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(posted["type"], "decision_required");
    assert_eq!(posted["shouldWait"], false);

    // A second hub on the same state folder exits non-zero naming the holder.
    let second = Command::new(env!("CARGO_BIN_EXE_gentboxd"))
        .args(["--state-dir"])
        .arg(directory.path())
        .args(["--config-dir"])
        .arg(directory.path())
        .env("GENTBOX_LOG", "off")
        .output()
        .unwrap();
    assert!(!second.status.success());
    let stderr = String::from_utf8_lossy(&second.stderr);
    let holder = format!("another hub is running (pid {})", daemon.child.id());
    assert!(stderr.contains(&holder), "{stderr}");
    assert!(api.join("port").exists(), "the loser touched nothing");

    let status = daemon.terminate();
    assert!(status.success(), "{status:?}");
    assert!(
        !api.join("port").exists(),
        "port is removed on a clean shutdown"
    );
    assert!(!api.join("hub.sock").exists());
    assert!(api.join("token").exists());

    // The state survives a restart, and the lock is free again.
    let restarted = Daemon::start(directory.path());
    let base = format!("http://127.0.0.1:{}", restarted.port());
    let id = posted["id"].as_str().unwrap();
    let response: Value = client
        .get(format!("{base}/v1/messages/{id}/response?wait=0"))
        .bearer_auth(restarted.token())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(response["status"], "pending");
    assert!(restarted.terminate().success());
}

#[test]
fn an_invalid_config_file_stops_the_start_with_a_message() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("config.toml"),
        "[sources.a]\ntitleGuard = \"strict\"\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_gentboxd"))
        .args(["--state-dir"])
        .arg(directory.path())
        .args(["--config-dir"])
        .arg(directory.path())
        .env("GENTBOX_LOG", "off")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("config.toml"));
}
