use std::{
    error::Error,
    ffi::OsString,
    fmt, fs,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCategory {
    Usage,
    Config,
    Source,
    Cache,
    Replay,
    Simulation,
    Integrity,
    Internal,
}

pub const CLI_CONTRACT_VERSION: u16 = 3;

impl ExitCategory {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::Config => "config",
            Self::Source => "source",
            Self::Cache => "cache",
            Self::Replay => "replay",
            Self::Simulation => "simulation",
            Self::Integrity => "integrity",
            Self::Internal => "internal",
        }
    }

    #[must_use]
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Usage => 2,
            Self::Config => 10,
            Self::Source => 20,
            Self::Cache => 21,
            Self::Replay => 30,
            Self::Simulation => 40,
            Self::Integrity => 50,
            Self::Internal => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OutputOptions {
    pub format: OutputFormat,
    pub quiet: bool,
    pub no_color: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInvocation {
    pub command: ParsedCommand,
    pub output: OutputOptions,
}

mod command;
mod market_replay;
mod market_replay_ui;
pub use command::{
    Command, CommandError, CommandKind, execute, execute_config_check, execute_inspect,
};
pub use market_replay::{
    MarketReplay, MarketReplayError, PLAYBACK_SPEEDS_MILLI, PlaybackSpeed, PlaybackStatus,
    ReplayHistory, TradeRow,
};

pub const USAGE: &str = "\
Usage:
  osmium version
  osmium init [--path <config.yaml>]
  osmium config check --config <file>
  osmium plan --config <file>
  osmium data sync|verify --config <file>
  osmium replay --config <file>
  osmium backtest --config <file> --output <new-directory>
  osmium run --config <file> [--output <new-directory>]
  osmium display --config <file>
  osmium cache prepare --config <file>
  osmium inspect --run <run-directory>

Non-interactive output options:
  --format human|json   Select human-readable or machine-readable output
  --quiet               Suppress successful command output
  --no-color            Disable terminal color

config_version 2 is required. Legacy config_version 1 is not supported. Output directories
must not already exist.
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedCommand {
    Help,
    Version,
    Init(PathBuf),
    ConfigCheck(PathBuf),
    Command(Command),
    MarketReplay(MarketReplayCommand),
    Inspect(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketReplayCommand {
    config: PathBuf,
}

impl MarketReplayCommand {
    #[must_use]
    pub const fn new(config: PathBuf) -> Self {
        Self { config }
    }

    #[must_use]
    pub fn config(&self) -> &Path {
        &self.config
    }
}

pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<ParsedInvocation, CliError> {
    let mut values = args.into_iter().collect::<Vec<_>>();
    let output = parse_output_options(&mut values)?;
    let mut args = values.into_iter();
    let Some(command) = args.next() else {
        return Err(CliError::usage("missing command"));
    };
    if command == "--help" || command == "-h" {
        return Ok(ParsedInvocation {
            command: ParsedCommand::Help,
            output,
        });
    }
    if command == "version" {
        if args.next().is_some() {
            return Err(CliError::usage("version does not accept options"));
        }
        return Ok(ParsedInvocation {
            command: ParsedCommand::Version,
            output,
        });
    }
    if command == "init" {
        return Ok(ParsedInvocation {
            command: parse_init(args)?,
            output,
        });
    }
    if command == "config" {
        let subcommand = args
            .next()
            .ok_or_else(|| CliError::usage("config requires check"))?;
        if subcommand == "--help" || subcommand == "-h" {
            return Ok(ParsedInvocation {
                command: ParsedCommand::Help,
                output,
            });
        }
        if subcommand != "check" {
            return Err(CliError::usage("config requires check"));
        }
        return Ok(ParsedInvocation {
            command: parse_config_check(args)?,
            output,
        });
    }
    if command == "data" {
        let subcommand = args
            .next()
            .ok_or_else(|| CliError::usage("data requires sync or verify"))?;
        if subcommand == "--help" || subcommand == "-h" {
            return Ok(ParsedInvocation {
                command: ParsedCommand::Help,
                output,
            });
        }
        let kind = match subcommand.to_str() {
            Some("sync") => CommandKind::DataSync,
            Some("verify") => CommandKind::DataVerify,
            _ => return Err(CliError::usage("data requires sync or verify")),
        };
        return Ok(ParsedInvocation {
            command: parse_command(kind, args)?,
            output,
        });
    }
    if command == "display" {
        if output.format != OutputFormat::Human || output.quiet || output.no_color {
            return Err(CliError::usage(
                "display is interactive and does not accept output options",
            ));
        }
        return Ok(ParsedInvocation {
            command: parse_display(args)?,
            output,
        });
    }
    if command == "inspect" {
        let flag = args
            .next()
            .ok_or_else(|| CliError::usage("missing required --run option"))?;
        if flag == "--help" || flag == "-h" {
            return Ok(ParsedInvocation {
                command: ParsedCommand::Help,
                output,
            });
        }
        if flag != "--run" {
            return Err(CliError::usage("inspect requires --run <run-directory>"));
        }
        let run = args
            .next()
            .ok_or_else(|| CliError::usage("missing value for --run"))?;
        if args.next().is_some() {
            return Err(CliError::usage("unexpected inspect option"));
        }
        return Ok(ParsedInvocation {
            command: ParsedCommand::Inspect(PathBuf::from(run)),
            output,
        });
    }
    if command == "cache" {
        let subcommand = args
            .next()
            .ok_or_else(|| CliError::usage("cache requires prepare"))?;
        if subcommand == "--help" || subcommand == "-h" {
            return Ok(ParsedInvocation {
                command: ParsedCommand::Help,
                output,
            });
        }
        if subcommand != "prepare" {
            return Err(CliError::usage("cache requires prepare"));
        }
        return Ok(ParsedInvocation {
            command: parse_command(CommandKind::CachePrepare, args)?,
            output,
        });
    }
    if let Some(kind) = match command.to_str() {
        Some("plan") => Some(CommandKind::Plan),
        Some("sync") | Some("verify") => None,
        Some("backtest") => Some(CommandKind::Backtest),
        Some("run") => Some(CommandKind::Run),
        Some("replay") => Some(CommandKind::Replay),
        _ => None,
    } {
        return Ok(ParsedInvocation {
            command: parse_command(kind, args)?,
            output,
        });
    }
    Err(CliError::usage(format!(
        "unknown command: {}",
        command.to_string_lossy()
    )))
}

fn parse_output_options(args: &mut Vec<OsString>) -> Result<OutputOptions, CliError> {
    let mut output = OutputOptions::default();
    let mut format_seen = false;
    let mut filtered = Vec::with_capacity(args.len());
    let mut index = 0;
    while index < args.len() {
        match args[index].to_str() {
            Some("--format") => {
                if format_seen {
                    return Err(CliError::usage("duplicate option: --format"));
                }
                format_seen = true;
                let value = args.get(index + 1).ok_or_else(|| {
                    CliError::usage("missing value for --format (expected human or json)")
                })?;
                output.format = match value.to_str() {
                    Some("human") => OutputFormat::Human,
                    Some("json") => OutputFormat::Json,
                    _ => return Err(CliError::usage("--format accepts human or json")),
                };
                index += 2;
            }
            Some("--quiet") => {
                if output.quiet {
                    return Err(CliError::usage("duplicate option: --quiet"));
                }
                output.quiet = true;
                index += 1;
            }
            Some("--no-color") => {
                if output.no_color {
                    return Err(CliError::usage("duplicate option: --no-color"));
                }
                output.no_color = true;
                index += 1;
            }
            _ => {
                filtered.push(args[index].clone());
                index += 1;
            }
        }
    }
    *args = filtered;
    Ok(output)
}

#[must_use]
pub fn format_success_json(command: &str, summary: &str) -> String {
    let mut fields = Map::new();
    let mut records = Vec::new();
    for line in summary.lines() {
        if let Some((key, value)) = line.split_once('=') {
            let value = parse_output_value(key, value);
            match fields.get_mut(key) {
                Some(Value::Array(values)) => values.push(value),
                Some(previous) => {
                    let old = std::mem::replace(previous, Value::Null);
                    *previous = Value::Array(vec![old, value]);
                }
                None => {
                    fields.insert(key.to_owned(), value);
                }
            }
        } else if !line.is_empty() {
            records.push(Value::String(line.to_owned()));
        }
    }
    if command == "version"
        && let Some(Value::String(version)) = records.first()
        && let Some(version) = version.strip_prefix("osmium ")
    {
        fields.insert("product".to_owned(), Value::String("osmium".to_owned()));
        fields.insert("version".to_owned(), Value::String(version.to_owned()));
        records.remove(0);
    }
    json!({
        "schema_version": 1,
        "status": "success",
        "command": command,
        "result": {
            "fields": fields,
            "records": records,
        },
    })
    .to_string()
}

#[must_use]
pub fn format_error_json(command: &str, category: ExitCategory, message: &str) -> String {
    json!({
        "schema_version": 1,
        "status": "error",
        "command": command,
        "error": {
            "category": category.as_str(),
            "code": category.exit_code(),
            "message": message,
        },
    })
    .to_string()
}

fn parse_output_value(key: &str, value: &str) -> Value {
    if matches!(value, "true" | "false") {
        return Value::Bool(value == "true");
    }
    if key.ends_with("_atoms") || key.ends_with("_checksum") || value.len() > 15 {
        return Value::String(value.to_owned());
    }
    value
        .parse::<u64>()
        .map(|number| Value::Number(number.into()))
        .unwrap_or_else(|_| Value::String(value.to_owned()))
}

fn parse_init(args: impl Iterator<Item = OsString>) -> Result<ParsedCommand, CliError> {
    let mut path = None;
    let mut args = args;
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            return Ok(ParsedCommand::Help);
        }
        if flag != "--path" {
            return Err(CliError::usage("init accepts only --path <config.yaml>"));
        }
        if path.is_some() {
            return Err(CliError::usage("duplicate option: --path"));
        }
        path =
            Some(PathBuf::from(args.next().ok_or_else(|| {
                CliError::usage("missing value for --path")
            })?));
    }
    Ok(ParsedCommand::Init(
        path.unwrap_or_else(|| PathBuf::from("config.yaml")),
    ))
}

fn parse_config_check(args: impl Iterator<Item = OsString>) -> Result<ParsedCommand, CliError> {
    let mut config = None;
    let mut args = args;
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            return Ok(ParsedCommand::Help);
        }
        if flag != "--config" {
            return Err(CliError::usage("config check accepts only --config <file>"));
        }
        if config.is_some() {
            return Err(CliError::usage("duplicate option: --config"));
        }
        config =
            Some(PathBuf::from(args.next().ok_or_else(|| {
                CliError::usage("missing value for --config")
            })?));
    }
    Ok(ParsedCommand::ConfigCheck(config.ok_or_else(|| {
        CliError::usage("missing required --config option")
    })?))
}

fn parse_display(args: impl Iterator<Item = OsString>) -> Result<ParsedCommand, CliError> {
    let mut config = None;
    let mut args = args;
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            return Ok(ParsedCommand::Help);
        }
        let value = args.next().ok_or_else(|| {
            CliError::usage(format!("missing value for {}", flag.to_string_lossy()))
        })?;
        if flag == "--config" && config.is_none() {
            config = Some(PathBuf::from(value));
        } else if flag == "--config" {
            return Err(CliError::usage("duplicate option: --config"));
        } else {
            return Err(CliError::usage(format!(
                "unknown display option: {}",
                flag.to_string_lossy()
            )));
        }
    }
    Ok(ParsedCommand::MarketReplay(MarketReplayCommand::new(
        config.ok_or_else(|| CliError::usage("missing required --config option"))?,
    )))
}

fn parse_command(
    kind: CommandKind,
    args: impl Iterator<Item = OsString>,
) -> Result<ParsedCommand, CliError> {
    let mut config = None;
    let mut output = None;
    let mut args = args;
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            return Ok(ParsedCommand::Help);
        }
        let value = args.next().ok_or_else(|| {
            CliError::usage(format!("missing value for {}", flag.to_string_lossy()))
        })?;
        match flag.to_str() {
            Some("--config") if config.is_none() => config = Some(PathBuf::from(value)),
            Some("--output") if output.is_none() => output = Some(PathBuf::from(value)),
            Some("--config" | "--output") => {
                return Err(CliError::usage(format!(
                    "duplicate option: {}",
                    flag.to_string_lossy()
                )));
            }
            _ => return Err(CliError::usage("unknown config option")),
        }
    }
    if output.is_some() && !matches!(kind, CommandKind::Backtest | CommandKind::Run) {
        return Err(CliError::usage(
            "--output is supported only by backtest and run",
        ));
    }
    Ok(ParsedCommand::Command(Command {
        kind,
        config: config.ok_or_else(|| CliError::usage("missing required --config option"))?,
        output,
    }))
}

pub fn execute_market_replay(command: &MarketReplayCommand) -> Result<(), CliError> {
    market_replay_ui::run(command.config()).map_err(CliError::MarketReplay)
}

const INIT_CONFIG: &str = r#"# Edit the placeholders before running `osmium config check`.
config_version: 2
data:
  source: teralion
  data_root: data
  source_policy: strict
  cache_policy: reuse_or_rebuild
universe:
  trading_dates: ["2026-01-01"]
  instruments: []
strategy:
  id: acceptance.example
  version: "1"
  parameters: {}
replay:
  data_policy: strict
simulation:
  fill: { evidence: top_of_book, quantity: observed }
  market_data_latency_ms: 0
  order_latency_ms: 0
  allocation: acceptance_sequence
  slippage: { model: adverse_fixed_delta, delta: "0" }
  fee:
    model: configured_rate
    rate: "0"
    applicable_sides: [buy, sell]
    minimum: "0"
    precision: 0
    rounding: half_up
    provenance: "user configuration"
  tax:
    model: configured_rate
    rate: "0"
    applicable_sides: [sell]
    minimum: "0"
    precision: 0
    rounding: down
    provenance: "user configuration"
  initial_cash: { currency: TWD, amount: "1000000" }
  position_accounting: average_cost_v1
  marking: { model: last_observable_mark_v1, allow_midpoint_fallback: false }
instrument_economics: []
output: { publication: create_new }
"#;

pub fn init_config(path: &Path) -> Result<(), CliError> {
    if path.exists() {
        return Err(CliError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("config already exists: {}", path.display()),
        )));
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, INIT_CONFIG)?;
    Ok(())
}

#[derive(Debug)]
pub enum CliError {
    Usage(Box<str>),
    MarketReplay(MarketReplayError),
    Io(std::io::Error),
}

impl CliError {
    fn usage(message: impl Into<Box<str>>) -> Self {
        Self::Usage(message.into())
    }

    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        self.category().exit_code()
    }

    #[must_use]
    pub const fn category(&self) -> ExitCategory {
        match self {
            Self::Usage(_) => ExitCategory::Usage,
            Self::MarketReplay(_) => ExitCategory::Replay,
            Self::Io(_) => ExitCategory::Internal,
        }
    }

    #[must_use]
    pub const fn is_usage_error(&self) -> bool {
        matches!(self, Self::Usage(_))
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => formatter.write_str(message),
            Self::MarketReplay(source) => write!(formatter, "{source}"),
            Self::Io(source) => write!(formatter, "{source}"),
        }
    }
}

impl Error for CliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MarketReplay(source) => Some(source),
            Self::Io(source) => Some(source),
            Self::Usage(_) => None,
        }
    }
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_arguments_require_config() {
        let parsed = parse_args([
            "replay".into(),
            "--config".into(),
            "config/m5-option.yaml".into(),
        ])
        .unwrap();
        assert_eq!(
            parsed.command,
            ParsedCommand::Command(Command {
                kind: CommandKind::Replay,
                config: "config/m5-option.yaml".into(),
                output: None,
            })
        );
    }

    #[test]
    fn malformed_arguments_are_usage_errors() {
        for args in [
            vec![],
            vec!["unknown".into()],
            vec!["replay".into(), "--config".into()],
            vec![
                "replay".into(),
                "--config".into(),
                "config.yaml".into(),
                "--config".into(),
                "again.yaml".into(),
            ],
        ] {
            let error = parse_args(args).unwrap_err();
            assert!(error.is_usage_error());
            assert_eq!(error.exit_code(), 2);
        }
    }

    #[test]
    fn inspect_requires_exactly_one_run_directory() {
        assert_eq!(
            parse_args(["inspect".into(), "--run".into(), "target/run".into()])
                .unwrap()
                .command,
            ParsedCommand::Inspect("target/run".into())
        );
        assert!(parse_args(["inspect".into()]).is_err());
        assert!(
            parse_args([
                "inspect".into(),
                "--run".into(),
                "target/run".into(),
                "--orders".into()
            ])
            .is_err()
        );
    }

    #[test]
    fn display_accepts_a_frozen_config_path() {
        assert_eq!(
            parse_args([
                "display".into(),
                "--config".into(),
                "config/m4-day-multi.yaml".into(),
            ])
            .unwrap()
            .command,
            ParsedCommand::MarketReplay(MarketReplayCommand::new(
                "config/m4-day-multi.yaml".into()
            ))
        );
        assert!(parse_args(["display".into()]).is_err());
        assert!(parse_args(["market".into(), "replay".into()]).is_err());
    }

    #[test]
    fn cache_prepare_is_a_first_class_config_command() {
        assert_eq!(
            parse_args([
                "cache".into(),
                "prepare".into(),
                "--config".into(),
                "config/m3-taifex-three.yaml".into(),
            ])
            .unwrap()
            .command,
            ParsedCommand::Command(Command {
                kind: CommandKind::CachePrepare,
                config: "config/m3-taifex-three.yaml".into(),
                output: None,
            })
        );
    }

    #[test]
    fn data_commands_use_the_release_namespace() {
        assert_eq!(
            parse_args([
                "data".into(),
                "sync".into(),
                "--config".into(),
                "config/example.yaml".into(),
            ])
            .unwrap()
            .command,
            ParsedCommand::Command(Command {
                kind: CommandKind::DataSync,
                config: "config/example.yaml".into(),
                output: None,
            })
        );
        assert!(parse_args(["sync".into(), "--config".into(), "config.yaml".into()]).is_err());
    }

    #[test]
    fn output_is_restricted_to_publishing_commands() {
        assert!(
            parse_args([
                "plan".into(),
                "--config".into(),
                "config.yaml".into(),
                "--output".into(),
                "target/ignored".into(),
            ])
            .is_err()
        );
        assert!(
            parse_args([
                "replay".into(),
                "--config".into(),
                "config.yaml".into(),
                "--output".into(),
                "target/ignored".into(),
            ])
            .is_err()
        );
    }

    #[test]
    fn subcommands_accept_help_without_required_arguments() {
        for args in [
            vec!["init".into(), "--help".into()],
            vec!["config".into(), "check".into(), "--help".into()],
            vec!["data".into(), "sync".into(), "--help".into()],
            vec!["cache".into(), "prepare".into(), "--help".into()],
            vec!["inspect".into(), "--help".into()],
        ] {
            assert_eq!(parse_args(args).unwrap().command, ParsedCommand::Help);
        }
    }

    #[test]
    fn release_meta_commands_parse_without_legacy_aliases() {
        assert_eq!(
            parse_args(["version".into()]).unwrap().command,
            ParsedCommand::Version
        );
        assert_eq!(
            parse_args(["init".into(), "--path".into(), "private.yaml".into()])
                .unwrap()
                .command,
            ParsedCommand::Init("private.yaml".into())
        );
        assert_eq!(
            parse_args([
                "config".into(),
                "check".into(),
                "--config".into(),
                "private.yaml".into(),
            ])
            .unwrap()
            .command,
            ParsedCommand::ConfigCheck("private.yaml".into())
        );
    }

    #[test]
    fn non_interactive_output_options_are_parsed_once() {
        let parsed = parse_args([
            "plan".into(),
            "--config".into(),
            "config.yaml".into(),
            "--format".into(),
            "json".into(),
            "--quiet".into(),
            "--no-color".into(),
        ])
        .unwrap();
        assert_eq!(parsed.output.format, OutputFormat::Json);
        assert!(parsed.output.quiet);
        assert!(parsed.output.no_color);
        assert_eq!(
            parsed.command,
            ParsedCommand::Command(Command {
                kind: CommandKind::Plan,
                config: "config.yaml".into(),
                output: None,
            })
        );
        assert!(parse_args(["version".into(), "--format".into(), "yaml".into(),]).is_err());
    }

    #[test]
    fn success_json_is_an_object_with_typed_fields() {
        let value: serde_json::Value =
            serde_json::from_str(&format_success_json("plan", "partitions=2\nready=true")).unwrap();
        assert_eq!(value["status"], "success");
        assert_eq!(value["result"]["fields"]["partitions"], 2);
        assert_eq!(value["result"]["fields"]["ready"], true);
    }
}
