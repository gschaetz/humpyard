//! Shared by the `up` process tests: a real `humpyard up` child, its output, and small helpers.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)] // test scaffolding

use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::common;

pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

pub fn state(name: &str) -> PathBuf {
    let dir = common::scratch().join(format!("up-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A configuration for a gateway that only has to listen.
pub fn write_config(state: &Path, port: u16, extra: &str) {
    std::fs::create_dir_all(state).unwrap();
    std::fs::set_permissions(state, std::fs::Permissions::from_mode(0o700)).unwrap();
    let text = format!(
        r#"listen = "127.0.0.1:{port}"
shutdown_grace_secs = 2
[providers.mock]
base_url = "http://127.0.0.1:9"
api_key_env = "HUMPYARD_TEST_KEY"
max_retries = 0
[targets.fast]
endpoints = [{{ provider = "mock", model = "fast-m" }}]
{extra}"#
    );
    std::fs::write(state.join("config.toml"), text).unwrap();
}

pub struct Up {
    pub child: Child,
    lines: Arc<Mutex<Vec<String>>>,
    errors: Arc<Mutex<Vec<String>>>,
}

impl Up {
    pub fn spawn(state: &Path, args: &[String], envs: &[(&str, &str)]) -> Self {
        let mut all = vec![("HUMPYARD_TEST_KEY", "test-key")];
        all.extend_from_slice(envs);
        Self::spawn_with(state, args, &all)
    }

    /// Like [`Up::spawn`] but sets only the variables given: the process has no provider key of its own.
    pub fn spawn_with(state: &Path, args: &[String], envs: &[(&str, &str)]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_humpyard"));
        command
            .arg("up")
            .arg("--no-open")
            .arg("--state-dir")
            .arg(state)
            .args(args)
            .env_remove("HUMPYARD_TEST_KEY")
            .env_remove("MOCK_KEY")
            .env_remove("GROQ_API_KEY")
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("NVIDIA_API_KEY")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (name, value) in envs {
            command.env(name, value);
        }
        let mut child = command.spawn().unwrap();
        let lines = Arc::new(Mutex::new(Vec::new()));
        let errors = Arc::new(Mutex::new(Vec::new()));
        for (reader, sink) in [
            (
                Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
                Arc::clone(&lines),
            ),
            (
                Box::new(child.stderr.take().unwrap()) as Box<dyn std::io::Read + Send>,
                Arc::clone(&errors),
            ),
        ] {
            std::thread::spawn(move || {
                for line in BufReader::new(reader).lines().map_while(Result::ok) {
                    sink.lock().unwrap().push(line);
                }
            });
        }
        Self {
            child,
            lines,
            errors,
        }
    }

    pub fn stdout(&self) -> String {
        self.lines.lock().unwrap().join("\n")
    }

    pub fn stderr(&self) -> String {
        self.errors.lock().unwrap().join("\n")
    }

    /// Waits until stdout has a line containing `needle`; fails if the process ends or the time runs out.
    pub fn wait_for(&mut self, needle: &str) -> String {
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(line) = self
                .lines
                .lock()
                .unwrap()
                .iter()
                .find(|l| l.contains(needle))
            {
                return line.clone();
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                panic!(
                    "up ended ({status}) before printing `{needle}`\nstdout: {}\nstderr: {}",
                    self.stdout(),
                    self.stderr()
                );
            }
            assert!(Instant::now() < end, "no `{needle}` in {}", self.stdout());
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The GUI's address and the access token, from the lines `up` prints.
    pub fn gui(&mut self) -> (String, String) {
        let link = self.wait_for("open this link to sign in: ");
        let url = link.split("sign in: ").nth(1).unwrap().to_string();
        let (base, token) = url.split_once("/#token=").unwrap();
        (base.to_string(), token.to_string())
    }

    pub fn terminate(&mut self) -> std::process::ExitStatus {
        let _ = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status();
        self.wait_exit()
    }

    pub fn wait_exit(&mut self) -> std::process::ExitStatus {
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < end, "up did not exit");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Up {
    fn drop(&mut self) {
        // A graceful stop, so `up` stops its gateway; a kill would leave that child running.
        let _ = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status();
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn connects(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(300),
    )
    .is_ok()
}

pub fn run_configs(state: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(state.join("run"))
        .map(|dirs| {
            dirs.filter_map(Result::ok)
                .map(|d| d.path().join("config.toml"))
                .filter(|p| p.exists())
                .collect()
        })
        .unwrap_or_default()
}
