//! The binary, run the way a user runs it.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tp_scan(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_tp-scan")).current_dir(root()).args(args).output().unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn check_lists_every_filter() {
    let (ok, out, err) = tp_scan(&["check", "filters"]);
    assert!(ok, "{err}");
    assert!(out.contains("ok: 42 filters (32 triggers, 10 lists)"), "{out}");
    assert!(out.contains("M_Spike_Hold_Up"));
}

#[test]
fn check_reports_a_broken_file_with_its_location() {
    let dir = std::env::temp_dir().join(format!("tp-scan-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("x.scan"), "trigger t {\n  when last >\n}\n").unwrap();
    let (ok, _, err) = tp_scan(&["check", dir.to_str().unwrap()]);
    assert!(!ok);
    assert!(err.contains("x.scan:3:1: expected a value"), "{err}");
}

#[test]
fn replay_writes_signals_lists_and_explanations() {
    let jsonl = std::env::temp_dir().join(format!("tp-scan-{}.jsonl", std::process::id()));
    let (ok, out, err) = tp_scan(&[
        "replay",
        "--filters",
        "filters",
        "--bars",
        "crates/tp-marketdata/tests/fixtures/parity_two_sessions.csv",
        "--daily",
        "crates/tp-scan-cli/tests/fixtures/xyz_daily.csv",
        "--float",
        "crates/tp-scan-cli/tests/fixtures/float.csv",
        "--jsonl",
        jsonl.to_str().unwrap(),
        "--explain",
        "M_Spike_Hold_Up:XYZ",
        "--quiet",
    ]);
    assert!(ok, "{err}");
    let summary = out.lines().find(|l| l.contains(" events, ")).unwrap_or_else(|| panic!("{out}"));
    assert!(summary.contains("0 errors"), "{summary}");
    let n_signals: usize = summary.split(", ").find_map(|p| p.strip_suffix(" signals")).unwrap().parse().unwrap();
    assert!(n_signals > 0, "{summary}");
    assert!(out.contains("M_Spike_Hold_Up => "), "{out}");

    let text = std::fs::read_to_string(&jsonl).unwrap();
    let signals = text.lines().filter(|l| l.contains("\"type\":\"signal\"")).count();
    assert_eq!(signals, n_signals);
}

#[test]
fn bad_usage_explains_itself() {
    let (ok, _, err) = tp_scan(&["replay", "--filters", "filters"]);
    assert!(!ok);
    assert!(err.contains("needs --filters and --bars"), "{err}");
}
