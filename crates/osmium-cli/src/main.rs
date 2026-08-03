use std::{env, process::ExitCode};

use osmium_cli::{
    ParsedCommand, USAGE, execute, execute_config_check, execute_inspect, execute_market_replay,
    init_config, parse_args,
};

fn main() -> ExitCode {
    match parse_args(env::args_os().skip(1)) {
        Ok(ParsedCommand::Help) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(ParsedCommand::Version) => {
            println!("osmium {}", env!("CARGO_PKG_VERSION"));
            println!("config_schema={}", osmium_config::RUN_CONFIG_VERSION);
            ExitCode::SUCCESS
        }
        Ok(ParsedCommand::Init(path)) => match init_config(&path) {
            Ok(()) => {
                println!("config={}", path.display());
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::from(error.exit_code())
            }
        },
        Ok(ParsedCommand::ConfigCheck(path)) => match execute_config_check(&path) {
            Ok(summary) => {
                println!("{summary}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::from(error.exit_code())
            }
        },
        Ok(ParsedCommand::MarketReplay(command)) => match execute_market_replay(&command) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::from(error.exit_code())
            }
        },
        Ok(ParsedCommand::Command(command)) => match execute(&command) {
            Ok(summary) => {
                println!("{summary}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::from(error.exit_code())
            }
        },
        Ok(ParsedCommand::Inspect(run)) => match execute_inspect(&run) {
            Ok(summary) => {
                println!("{summary}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::from(20)
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
