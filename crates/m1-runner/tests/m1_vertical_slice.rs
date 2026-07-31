use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use m1_runner::{ArtifactExportError, M1FixtureInput};

const NORMALIZED_EVENTS_BLAKE3: &str =
    "6e5c4cbdb1615b53db770bc8283623da9259b3791f8d008fc42e6753b531803b";
const EVENT_STREAM_BLAKE3: &str =
    "06f5ed8855e7f13813b3c428f2e1429b524e03bec6c1a393e8a0e4580daa9342";
const FINAL_STATE_BLAKE3: &str = "46483a599e221b707a0d99cf7c5dbe6cf909376a31d17af3085685f3dcfcca17";
const STRATEGY_OUTPUT_BLAKE3: &str =
    "7b772890179c4d5364ff73df5a9d6b368ed9adc7aae090b04ed163bbe5551b10";

fn fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/teralion/twse/2330/2026-07-27/regular-quotes")
}

fn fixture_root() -> PathBuf {
    fixture_directory().parent().unwrap().to_path_buf()
}

fn unique_output_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/m1-artifact-export-{}-{nonce}",
        std::process::id()
    ))
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(HEX[usize::from(byte >> 4)]));
        text.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    text
}

fn shuffle<T>(items: &mut [T], mut state: u64) {
    for index in (1..items.len()).rev() {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let selected = usize::try_from(state % u64::try_from(index + 1).unwrap()).unwrap();
        items.swap(index, selected);
    }
}

#[test]
fn m1_vertical_slice_reports_complete_local_run() {
    let input = M1FixtureInput::load(&fixture_directory()).unwrap();
    let artifacts = input.run().unwrap();
    let summary = artifacts.summary();

    assert_eq!(summary.input_record_count, 73_796);
    assert_eq!(summary.outside_replay_window_count, 1);
    assert_eq!(summary.known_skipped_count, 0);
    assert_eq!(summary.normalized_event_count, 73_795);
    assert_eq!(summary.quote_snapshot_count, 73_435);
    assert_eq!(summary.trade_batch_count, 3);
    assert_eq!(summary.indicative_opening_auction_count, 180);
    assert_eq!(summary.indicative_closing_auction_count, 177);
    assert_eq!(summary.callback_count, 73_795);
    assert_eq!(summary.strategy_output_record_count, 147_410);
    assert_eq!(summary.warning_count, 0);
    assert!(artifacts.warnings().is_empty());
    assert_eq!(
        hex(summary.normalized_events_checksum.as_bytes()),
        NORMALIZED_EVENTS_BLAKE3
    );
    assert_eq!(
        hex(summary.event_stream_checksum.as_bytes()),
        EVENT_STREAM_BLAKE3
    );
    assert_eq!(hex(&summary.final_state_checksum), FINAL_STATE_BLAKE3);
    assert_eq!(
        hex(summary.strategy_output_checksum.as_bytes()),
        STRATEGY_OUTPUT_BLAKE3
    );

    let golden = fixture_directory().parent().unwrap().join("golden");
    assert_eq!(
        std::fs::read_to_string(golden.join("normalized-events.blake3"))
            .unwrap()
            .trim(),
        NORMALIZED_EVENTS_BLAKE3
    );
    assert_eq!(
        std::fs::read_to_string(golden.join("event-stream.blake3"))
            .unwrap()
            .trim(),
        EVENT_STREAM_BLAKE3
    );
    assert_eq!(
        std::fs::read_to_string(golden.join("final-state.blake3"))
            .unwrap()
            .trim(),
        FINAL_STATE_BLAKE3
    );
    assert_eq!(
        std::fs::read_to_string(golden.join("strategy-output.blake3"))
            .unwrap()
            .trim(),
        STRATEGY_OUTPUT_BLAKE3
    );

    let output = unique_output_directory();
    artifacts
        .export(
            &output,
            &fixture_root().join("metadata.yaml"),
            &golden.join("fixture-set.sha256"),
        )
        .unwrap();
    assert_eq!(
        std::fs::read(output.join("normalized-events.bin")).unwrap(),
        artifacts.normalized_events()
    );
    assert_eq!(
        std::fs::read(output.join("strategy-output.bin")).unwrap(),
        artifacts.strategy_output()
    );
    assert_eq!(
        std::fs::read_to_string(output.join("run-summary.yaml")).unwrap(),
        std::fs::read_to_string(golden.join("run-summary.yaml")).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(output.join("warnings.yaml")).unwrap(),
        std::fs::read_to_string(golden.join("warnings.yaml")).unwrap()
    );
    assert!(matches!(
        artifacts.export(
            &output,
            &fixture_root().join("metadata.yaml"),
            &golden.join("fixture-set.sha256"),
        ),
        Err(ArtifactExportError::OutputExists(path)) if path == output
    ));
    std::fs::remove_dir_all(output).unwrap();
}

#[test]
fn m1_repeated_and_shuffled_runs_are_byte_identical() {
    let input = M1FixtureInput::load(&fixture_directory()).unwrap();
    let baseline = input.run().unwrap();

    for _ in 0..9 {
        let repeated = input.run().unwrap();
        assert_eq!(repeated.normalized_events(), baseline.normalized_events());
        assert_eq!(repeated.strategy_output(), baseline.strategy_output());
        assert_eq!(repeated.summary(), baseline.summary());
    }

    for seed in [0x5eed_0001, 0x5eed_0002, 0x5eed_0003] {
        let mut events = input.events().to_vec();
        shuffle(&mut events, seed);
        let perturbed = input.run_with_events(events).unwrap();
        assert_eq!(perturbed.normalized_events(), baseline.normalized_events());
        assert_eq!(perturbed.strategy_output(), baseline.strategy_output());
        assert_eq!(perturbed.summary(), baseline.summary());
    }
}

#[test]
fn m1_runner_requires_only_an_explicit_local_fixture_path() {
    let input = M1FixtureInput::load(&fixture_directory()).unwrap();
    let artifacts = input.run().unwrap();
    assert_eq!(artifacts.summary().normalized_event_count, 73_795);
}
