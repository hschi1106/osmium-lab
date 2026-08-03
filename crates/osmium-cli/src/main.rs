use std::{env, process::ExitCode};

use osmium_cli::{
    CLI_CONTRACT_VERSION, OutputFormat, OutputOptions, ParsedCommand, ParsedInvocation, USAGE,
    execute, execute_config_check, execute_inspect, execute_market_replay, format_error_json,
    format_success_json, init_config, parse_args,
};

fn main() -> ExitCode {
    match parse_args(env::args_os().skip(1)) {
        Ok(ParsedInvocation {
            command: ParsedCommand::Help,
            ..
        }) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(ParsedInvocation {
            command: ParsedCommand::Version,
            output,
        }) => {
            let summary = format!(
                "osmium {}\ncli_contract={}\nconfig_schema={}\nevent_schema={}\ncache_format={}\naccounting={}",
                env!("CARGO_PKG_VERSION"),
                CLI_CONTRACT_VERSION,
                osmium_config::RUN_CONFIG_VERSION,
                market_types::EVENT_SCHEMA_VERSION,
                data_sync::CACHE_FORMAT_VERSION,
                execution_sim::ACCOUNTING_VERSION
            );
            emit_success("version", output, &summary)
        }
        Ok(ParsedInvocation {
            command: ParsedCommand::Init(path),
            output,
        }) => match init_config(&path) {
            Ok(()) => emit_success("init", output, &format!("config={}", path.display())),
            Err(error) => emit_error("init", output, &error.to_string(), error.category()),
        },
        Ok(ParsedInvocation {
            command: ParsedCommand::ConfigCheck(path),
            output,
        }) => match execute_config_check(&path) {
            Ok(summary) => emit_success("config check", output, &summary),
            Err(error) => emit_error("config check", output, &error.to_string(), error.category()),
        },
        Ok(ParsedInvocation {
            command: ParsedCommand::MarketReplay(command),
            output,
        }) => match execute_market_replay(&command) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => emit_error("display", output, &error.to_string(), error.category()),
        },
        Ok(ParsedInvocation {
            command: ParsedCommand::Command(command),
            output,
        }) => {
            let name = command.kind.name();
            match execute(&command) {
                Ok(summary) => emit_success(name, output, &summary),
                Err(error) => emit_error(name, output, &error.to_string(), error.category()),
            }
        }
        Ok(ParsedInvocation {
            command: ParsedCommand::Inspect(run),
            output,
        }) => match execute_inspect(&run) {
            Ok(summary) => emit_success("inspect", output, &summary),
            Err(error) => emit_error("inspect", output, &error.to_string(), error.category()),
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

fn emit_success(command: &str, output: OutputOptions, summary: &str) -> ExitCode {
    if output.quiet {
        return ExitCode::SUCCESS;
    }
    match output.format {
        OutputFormat::Human => println!("{summary}"),
        OutputFormat::Json => println!("{}", format_success_json(command, summary)),
    }
    ExitCode::SUCCESS
}

fn emit_error(
    command: &str,
    output: OutputOptions,
    message: &str,
    category: osmium_cli::ExitCategory,
) -> ExitCode {
    match output.format {
        OutputFormat::Human => eprintln!("error: {message}"),
        OutputFormat::Json => eprintln!("{}", format_error_json(command, category, message)),
    }
    ExitCode::from(category.exit_code())
}
