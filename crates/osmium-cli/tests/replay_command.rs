use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const EXPECTED_ARTIFACTS: [&str; 9] = [
    "event-stream.blake3",
    "final-state.blake3",
    "fixture-metadata.yaml",
    "fixture-set.sha256",
    "normalized-events.bin",
    "run-summary.yaml",
    "strategy-output.bin",
    "strategy-output.blake3",
    "warnings.yaml",
];

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/teralion/twse/2330/2026-07-27")
}

fn unique_output_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/osmium-cli-replay-{}-{nonce}",
        std::process::id()
    ))
}

fn file_names(directory: &Path) -> Vec<String> {
    let mut names = fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            entry
                .unwrap()
                .file_name()
                .into_string()
                .expect("artifact names are UTF-8")
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[test]
fn replay_command_exports_the_complete_m1_artifact_set_without_api_key() {
    let fixture = fixture_root();
    let output = unique_output_directory();
    let result = Command::new(env!("CARGO_BIN_EXE_osmium"))
        .args([
            "replay",
            "--fixture",
            fixture.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .env_remove("TERALION_API_KEY")
        .output()
        .unwrap();

    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(stdout.contains("M1 TWSE replay completed"));
    assert!(stdout.contains("normalized_events=73795"));
    assert!(stdout.contains("strategy_callbacks=73795"));
    assert!(stdout.contains("warnings=0"));
    assert_eq!(file_names(&output), EXPECTED_ARTIFACTS.map(str::to_owned));
    assert_eq!(
        fs::read_to_string(output.join("run-summary.yaml")).unwrap(),
        fs::read_to_string(fixture.join("golden/run-summary.yaml")).unwrap()
    );
    assert_eq!(
        fs::read_to_string(output.join("warnings.yaml")).unwrap(),
        fs::read_to_string(fixture.join("golden/warnings.yaml")).unwrap()
    );

    fs::remove_dir_all(output).unwrap();
}
