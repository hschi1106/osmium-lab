use std::path::PathBuf;

use m1_runner::M1FixtureInput;

const NORMALIZED_EVENTS_BLAKE3: &str =
    "7e37ff0ad4a8b15b4c569b295c0f03f26bb6c0f32db1493edac71620e85a28df";
const EVENT_STREAM_BLAKE3: &str =
    "0cecf1f6c3c6a8422955183fe383e787612efee3a4c4a7961d7faa6ee9e1de56";
const FINAL_STATE_BLAKE3: &str = "02f256fc1007ce41e56200a4f82fc0f0cb504ee29afdf7262307a232862e7ea0";
const STRATEGY_OUTPUT_BLAKE3: &str =
    "763a5cb305a7ebbe86ea463e4091e90346421273e61b2f40f0c8ba4247690917";

fn fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/teralion/twse/2330/2026-07-27/regular-quotes")
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
    assert_eq!(summary.quote_snapshot_count, 73_792);
    assert_eq!(summary.trade_batch_count, 3);
    assert_eq!(summary.callback_count, 73_795);
    assert_eq!(summary.strategy_output_record_count, 147_590);
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
