use std::{env, process::ExitCode};

use osmium_cli::{ParsedCommand, USAGE, execute_m2, execute_replay, parse_args};

fn main() -> ExitCode {
    match parse_args(env::args_os().skip(1)) {
        Ok(ParsedCommand::Help) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(ParsedCommand::Replay(command)) => match execute_replay(&command) {
            Ok(outcome) => {
                let summary = outcome.summary();
                println!("M1 TWSE replay completed");
                println!("input_records={}", summary.input_record_count);
                println!("normalized_events={}", summary.normalized_event_count);
                println!("strategy_callbacks={}", summary.callback_count);
                println!(
                    "strategy_output_records={}",
                    summary.strategy_output_record_count
                );
                println!("warnings={}", summary.warning_count);
                println!("output={}", outcome.output_directory().display());
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::from(error.exit_code())
            }
        },
        Ok(ParsedCommand::M2(command)) => match execute_m2(&command) {
            Ok(summary) => {
                println!("{summary}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::from(1)
            }
        },
        Err(error) => {
            eprintln!("error: {error}");
            if error.is_usage_error() {
                eprintln!();
                eprint!("{USAGE}");
            }
            ExitCode::from(error.exit_code())
        }
    }
}
