use std::{
    error::Error,
    ffi::OsString,
    fmt, fs,
    path::{Path, PathBuf},
};

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

pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<ParsedCommand, CliError> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Err(CliError::usage("missing command"));
    };
    if command == "--help" || command == "-h" {
        return Ok(ParsedCommand::Help);
    }
    if command == "version" {
        if args.next().is_some() {
            return Err(CliError::usage("version does not accept options"));
        }
        return Ok(ParsedCommand::Version);
    }
    if command == "init" {
        return parse_init(args);
    }
    if command == "config" {
        let subcommand = args
            .next()
            .ok_or_else(|| CliError::usage("config requires check"))?;
        if subcommand == "--help" || subcommand == "-h" {
            return Ok(ParsedCommand::Help);
        }
        if subcommand != "check" {
            return Err(CliError::usage("config requires check"));
        }
        return parse_config_check(args);
    }
    if command == "data" {
        let subcommand = args
            .next()
            .ok_or_else(|| CliError::usage("data requires sync or verify"))?;
        if subcommand == "--help" || subcommand == "-h" {
            return Ok(ParsedCommand::Help);
        }
        let kind = match subcommand.to_str() {
            Some("sync") => CommandKind::DataSync,
            Some("verify") => CommandKind::DataVerify,
            _ => return Err(CliError::usage("data requires sync or verify")),
        };
        return parse_command(kind, args);
    }
    if command == "display" {
        return parse_display(args);
    }
    if command == "inspect" {
        let flag = args
            .next()
            .ok_or_else(|| CliError::usage("missing required --run option"))?;
        if flag == "--help" || flag == "-h" {
            return Ok(ParsedCommand::Help);
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
        return Ok(ParsedCommand::Inspect(PathBuf::from(run)));
    }
    if command == "cache" {
        let subcommand = args
            .next()
            .ok_or_else(|| CliError::usage("cache requires prepare"))?;
        if subcommand == "--help" || subcommand == "-h" {
            return Ok(ParsedCommand::Help);
        }
        if subcommand != "prepare" {
            return Err(CliError::usage("cache requires prepare"));
        }
        return parse_command(CommandKind::CachePrepare, args);
    }
    if let Some(kind) = match command.to_str() {
        Some("plan") => Some(CommandKind::Plan),
        Some("sync") | Some("verify") => None,
        Some("backtest") => Some(CommandKind::Backtest),
        Some("run") => Some(CommandKind::Run),
        Some("replay") => Some(CommandKind::Replay),
        _ => None,
    } {
        return parse_command(kind, args);
    }
    Err(CliError::usage(format!(
        "unknown command: {}",
        command.to_string_lossy()
    )))
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
        match self {
            Self::Usage(_) => 2,
            Self::MarketReplay(error) => error.exit_code(),
            Self::Io(_) => 1,
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
            parsed,
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
            parse_args(["inspect".into(), "--run".into(), "target/run".into()]).unwrap(),
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
            .unwrap(),
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
            .unwrap(),
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
            .unwrap(),
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
            assert_eq!(parse_args(args).unwrap(), ParsedCommand::Help);
        }
    }

    #[test]
    fn release_meta_commands_parse_without_legacy_aliases() {
        assert_eq!(
            parse_args(["version".into()]).unwrap(),
            ParsedCommand::Version
        );
        assert_eq!(
            parse_args(["init".into(), "--path".into(), "private.yaml".into()]).unwrap(),
            ParsedCommand::Init("private.yaml".into())
        );
        assert_eq!(
            parse_args([
                "config".into(),
                "check".into(),
                "--config".into(),
                "private.yaml".into(),
            ])
            .unwrap(),
            ParsedCommand::ConfigCheck("private.yaml".into())
        );
    }
}
