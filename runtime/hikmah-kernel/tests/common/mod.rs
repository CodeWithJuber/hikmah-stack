#![allow(dead_code)]
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh, unique store path inside the system temp dir.
pub fn temp_store(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("hikmah-test-{name}-{nonce}-{n}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("memory.jsonl")
}

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

pub fn copy_fixture(name: &str) -> PathBuf {
    let target = temp_store(name);
    std::fs::copy(fixture(name), &target).unwrap();
    target
}

pub fn line_count(path: &PathBuf) -> usize {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count()
}

/// Remove every AI-agent marker from a child process's environment, so CLI tests behave the same
/// inside and outside an agent session (this suite often runs under one).
pub fn without_agent_session(command: &mut std::process::Command) -> &mut std::process::Command {
    for (name, _) in std::env::vars_os() {
        if let Some(name) = name.to_str() {
            if hikmah_kernel::principal::is_agent_marker(name) {
                command.env_remove(name);
            }
        }
    }
    command
}
