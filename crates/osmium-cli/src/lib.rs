use std::{
    error::Error,
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
};

use m1_runner::{ArtifactExportError, M1FixtureInput, M1RunError, M1RunSummary};

mod m2;
pub use m2::{M2Command, M2CommandError, M2CommandKind, execute as execute_m2};

pub const USAGE: &str = "\
Usage:
  osmium replay --fixture <fixture-root> --output <output-directory>
  osmium plan|sync|verify|replay|backtest|run --config <file> [--output <directory>]

The M1 fixture root must contain metadata.yaml, regular-quotes/, and
golden/fixture-set.sha256. The output directory must not already exist.
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedCommand {
    Help,
    Replay(ReplayCommand),
    M2(M2Command),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayCommand {
    fixture_root: PathBuf,
    output_directory: PathBuf,
}

impl ReplayCommand {
    #[must_use]
    pub const fn new(fixture_root: PathBuf, output_directory: PathBuf) -> Self {
        Self {
            fixture_root,
            output_directory,
        }
    }

    #[must_use]
    pub fn fixture_root(&self) -> &Path {
        &self.fixture_root
    }

    #[must_use]
    pub fn output_directory(&self) -> &Path {
        &self.output_directory
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayOutcome {
    output_directory: PathBuf,
    summary: M1RunSummary,
}

impl ReplayOutcome {
    #[must_use]
    pub fn output_directory(&self) -> &Path {
        &self.output_directory
    }

    #[must_use]
    pub const fn summary(&self) -> M1RunSummary {
        self.summary
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
    if let Some(kind) = match command.to_str() {
        Some("plan") => Some(M2CommandKind::Plan),
        Some("sync") => Some(M2CommandKind::Sync),
        Some("verify") => Some(M2CommandKind::Verify),
        Some("backtest") => Some(M2CommandKind::Backtest),
        Some("run") => Some(M2CommandKind::Run),
        _ => None,
    } {
        return parse_m2(kind, args);
    }
    if command != "replay" {
        return Err(CliError::usage(format!(
            "unknown command: {}",
            command.to_string_lossy()
        )));
    }
    let remaining = args.collect::<Vec<_>>();
    if remaining.iter().any(|argument| argument == "--config") {
        return parse_m2(M2CommandKind::Replay, remaining.into_iter());
    }
    let mut args = remaining.into_iter();

    let mut fixture_root = None;
    let mut output_directory = None;
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            return Ok(ParsedCommand::Help);
        }
        let value = args.next().ok_or_else(|| {
            CliError::usage(format!("missing value for {}", flag.to_string_lossy()))
        })?;
        match flag.to_str() {
            Some("--fixture") if fixture_root.is_none() => {
                fixture_root = Some(PathBuf::from(value));
            }
            Some("--output") if output_directory.is_none() => {
                output_directory = Some(PathBuf::from(value));
            }
            Some("--fixture" | "--output") => {
                return Err(CliError::usage(format!(
                    "duplicate option: {}",
                    flag.to_string_lossy()
                )));
            }
            _ => {
                return Err(CliError::usage(format!(
                    "unknown option: {}",
                    flag.to_string_lossy()
                )));
            }
        }
    }

    let fixture_root =
        fixture_root.ok_or_else(|| CliError::usage("missing required --fixture option"))?;
    let output_directory =
        output_directory.ok_or_else(|| CliError::usage("missing required --output option"))?;
    Ok(ParsedCommand::Replay(ReplayCommand::new(
        fixture_root,
        output_directory,
    )))
}

fn parse_m2(
    kind: M2CommandKind,
    args: impl Iterator<Item = OsString>,
) -> Result<ParsedCommand, CliError> {
    let mut config = None;
    let mut output = None;
    let mut args = args;
    while let Some(flag) = args.next() {
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
            _ => return Err(CliError::usage("unknown M2 option")),
        }
    }
    Ok(ParsedCommand::M2(M2Command {
        kind,
        config: config.ok_or_else(|| CliError::usage("missing required --config option"))?,
        output,
    }))
}

pub fn execute_replay(command: &ReplayCommand) -> Result<ReplayOutcome, CliError> {
    let fixture_root = command.fixture_root();
    let metadata = required_path(fixture_root, "metadata.yaml", "fixture metadata")?;
    let quotes = required_path(fixture_root, "regular-quotes", "fixture quote shards")?;
    let fixture_checksum = required_path(
        fixture_root,
        "golden/fixture-set.sha256",
        "fixture-set checksum",
    )?;

    let input = M1FixtureInput::load(&quotes).map_err(|source| CliError::Replay {
        fixture_root: fixture_root.to_path_buf(),
        source: Box::new(source),
    })?;
    let artifacts = input.run().map_err(|source| CliError::Replay {
        fixture_root: fixture_root.to_path_buf(),
        source: Box::new(source),
    })?;
    artifacts
        .export(command.output_directory(), &metadata, &fixture_checksum)
        .map_err(|source| CliError::Export {
            output_directory: command.output_directory().to_path_buf(),
            source,
        })?;

    Ok(ReplayOutcome {
        output_directory: command.output_directory().to_path_buf(),
        summary: *artifacts.summary(),
    })
}

fn required_path(
    root: &Path,
    relative: &str,
    description: &'static str,
) -> Result<PathBuf, CliError> {
    let path = root.join(relative);
    if path.exists() {
        Ok(path)
    } else {
        Err(CliError::MissingFixturePath { description, path })
    }
}

#[derive(Debug)]
pub enum CliError {
    Usage(Box<str>),
    MissingFixturePath {
        description: &'static str,
        path: PathBuf,
    },
    Replay {
        fixture_root: PathBuf,
        source: Box<M1RunError>,
    },
    Export {
        output_directory: PathBuf,
        source: ArtifactExportError,
    },
}

impl CliError {
    fn usage(message: impl Into<Box<str>>) -> Self {
        Self::Usage(message.into())
    }

    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::MissingFixturePath { .. } | Self::Replay { .. } | Self::Export { .. } => 1,
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
            Self::MissingFixturePath { description, path } => {
                write!(formatter, "missing {description} at {}", path.display())
            }
            Self::Replay {
                fixture_root,
                source,
            } => write!(
                formatter,
                "replay failed for fixture {}: {source}",
                fixture_root.display()
            ),
            Self::Export {
                output_directory,
                source,
            } => write!(
                formatter,
                "artifact export failed for {}: {source}",
                output_directory.display()
            ),
        }
    }
}

impl Error for CliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Replay { source, .. } => Some(source.as_ref()),
            Self::Export { source, .. } => Some(source),
            Self::Usage(_) | Self::MissingFixturePath { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_arguments_accept_either_option_order() {
        let parsed = parse_args([
            "replay".into(),
            "--output".into(),
            "target/run".into(),
            "--fixture".into(),
            "fixtures/day".into(),
        ])
        .unwrap();
        assert_eq!(
            parsed,
            ParsedCommand::Replay(ReplayCommand::new(
                "fixtures/day".into(),
                "target/run".into()
            ))
        );
    }

    #[test]
    fn malformed_arguments_are_usage_errors() {
        for args in [
            vec![],
            vec!["unknown".into()],
            vec!["replay".into(), "--fixture".into()],
            vec![
                "replay".into(),
                "--fixture".into(),
                "fixture".into(),
                "--fixture".into(),
                "again".into(),
            ],
        ] {
            let error = parse_args(args).unwrap_err();
            assert!(error.is_usage_error());
            assert_eq!(error.exit_code(), 2);
        }
    }
}
