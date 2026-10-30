//! A reader that goes away ends jimtime's output, not the command.

#![cfg(unix)]

use std::process::Command;

#[test]
fn closed_stdout_is_not_a_panic() {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);

    let output = Command::new(env!("CARGO_BIN_EXE_jimtime"))
        .args(["config", "path"])
        .env(
            "JIMTIME_HOME",
            std::env::temp_dir().join("jimtime-broken-pipe"),
        )
        .stdout(writer)
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{:?}: {stderr}", output.status);
    assert!(!stderr.contains("panicked"), "{stderr}");
}
