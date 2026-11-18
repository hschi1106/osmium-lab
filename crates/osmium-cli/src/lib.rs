//! The command line of osmium (`docs/guide/running.md`): runs a strategy of a
//! [`Registry`] as a configuration file says.
//!
//! A program that adds its own strategies builds a registry and calls [`main`].

use std::{
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

use osmium_runner::{ConfigError, DaySummary, RunConfig, RunError, RunRequest};
use osmium_strategy::{ParamError, Registry, Strategy, StrategyEntry};

const USAGE: &str = "\
Usage: osmium <command>

Commands:
  run <config.toml> --out <dir>   Run the backtest and write its results to <dir>,
                                  which must be empty or not exist
  check <config.toml>             Validate the configuration and list what the run
                                  would read, without running it
  strategies                      List the strategies this program can run
  version                         Print the versions that results depend on
  help                            Print this text

Exit status: 0 on success, 1 when the command failed, 2 when it was not understood.
";

/// The command was understood and failed.
const FAILED: u8 = 1;
/// The command line was not understood.
const USAGE_ERROR: u8 = 2;

#[derive(Debug)]
enum Failure {
    Usage(String),
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Config {
        path: PathBuf,
        source: ConfigError,
    },
    UnknownStrategy {
        name: String,
        known: Vec<&'static str>,
    },
    Param(ParamError),
    Run(RunError),
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(problem) => write!(formatter, "{problem}\n\n{USAGE}"),
            Self::Read { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Config { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::UnknownStrategy { name, known } => write!(
                formatter,
                "strategy.name: `{name}` is not a strategy of this program (it has: {})",
                known.join(", ")
            ),
            Self::Param(error) => error.fmt(formatter),
            Self::Run(error) => error.fmt(formatter),
        }
    }
}

/// A configuration file with the strategy it names, built from its parameters.
struct Loaded {
    text: String,
    config: RunConfig,
    entry: StrategyEntry,
    strategy: Box<dyn Strategy>,
}

fn load(registry: &Registry, path: &Path) -> Result<Loaded, Failure> {
    let text = fs::read_to_string(path).map_err(|source| Failure::Read {
        path: path.to_owned(),
        source,
    })?;
    let mut config = RunConfig::parse(&text).map_err(|source| Failure::Config {
        path: path.to_owned(),
        source,
    })?;
    let entry = *registry
        .get(&config.strategy)
        .ok_or_else(|| Failure::UnknownStrategy {
            name: config.strategy.clone(),
            known: registry.entries().iter().map(|entry| entry.name).collect(),
        })?;
    let strategy = (entry.build)(&mut config.params).map_err(Failure::Param)?;
    config.params.finish().map_err(Failure::Param)?;
    Ok(Loaded {
        text,
        config,
        entry,
        strategy,
    })
}

fn run(
    registry: &Registry,
    arguments: &[String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<(), Failure> {
    let usage = || Failure::Usage("run needs a configuration file and --out <dir>".to_owned());
    let (config, output) = match arguments {
        [config, flag, output] if flag == "--out" => (config, output),
        [flag, output, config] if flag == "--out" => (config, output),
        _ => return Err(usage()),
    };
    let mut loaded = load(registry, Path::new(config))?;
    let mut progress = |day: &DaySummary| {
        // Progress is not a result: it goes to standard error and may fail silently.
        let _ = writeln!(
            err,
            "{} messages={} orders={} fills={} equity={}",
            day.date, day.messages, day.orders, day.fills, day.equity
        );
    };
    let summary = osmium_runner::run(RunRequest {
        config: &loaded.config,
        config_text: &loaded.text,
        strategy: loaded.strategy.as_mut(),
        strategy_version: loaded.entry.version,
        output: Path::new(output),
        progress: &mut progress,
    })
    .map_err(Failure::Run)?;
    let _ = writeln!(
        out,
        "trading days  {}\nmessages      {}\norders        {}\nfills         {}\n\
         initial cash  {}\nfinal cash    {}\nfinal equity  {}\nresults       {}",
        summary.trading_days,
        summary.messages,
        summary.orders,
        summary.fills,
        summary.initial_cash,
        summary.final_cash,
        summary.final_equity,
        summary.output.display(),
    );
    Ok(())
}

fn check(registry: &Registry, arguments: &[String], out: &mut dyn Write) -> Result<(), Failure> {
    let [config] = arguments else {
        return Err(Failure::Usage(
            "check needs a configuration file".to_owned(),
        ));
    };
    let loaded = load(registry, Path::new(config))?;
    let plan = osmium_runner::plan(&loaded.config).map_err(Failure::Run)?;
    let feeds: Vec<&str> = loaded.config.feeds.iter().map(|feed| feed.part()).collect();
    let _ = writeln!(
        out,
        "strategy      {} (version {})\nmarketdata    {}\nfeeds         {}\ninstruments   {}\n\
         trading days  {} ({} to {})",
        loaded.entry.name,
        loaded.entry.version,
        loaded.config.md_root.display(),
        feeds.join(", "),
        loaded
            .config
            .symbols
            .as_ref()
            .map_or_else(|| "all".to_owned(), |symbols| symbols.len().to_string()),
        plan.days.len(),
        plan.days.first().expect("a plan has a trading day"),
        plan.days.last().expect("a plan has a trading day"),
    );
    for (date, feed) in &plan.missing {
        let _ = writeln!(out, "missing       {date} {}", feed.part());
    }
    if plan.missing.is_empty() {
        let _ = writeln!(out, "missing       none");
    }
    Ok(())
}

fn strategies(registry: &Registry, out: &mut dyn Write) {
    for entry in registry.entries() {
        let _ = writeln!(
            out,
            "{} (version {})\n    {}",
            entry.name, entry.version, entry.about
        );
    }
}

fn version(out: &mut dyn Write) {
    let _ = writeln!(
        out,
        "osmium {}\nmarketdata schema  {}\ndecoder            {}\nordering           {}\n\
         matching rules     {}\nledger rules       {}\noutput             {}",
        env!("CARGO_PKG_VERSION"),
        osmium_md::SCHEMA_VERSION,
        osmium_md::DECODER_VERSION,
        osmium_replay::ORDERING_VERSION,
        osmium_matching::RULES_VERSION,
        osmium_ledger::RULES_VERSION,
        osmium_runner::OUTPUT_VERSION,
    );
}

/// Runs one command line (without the program name), writing results to `out` and
/// progress and errors to `err`, and returns the exit status.
pub fn execute(
    registry: &Registry,
    arguments: &[String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let result = match arguments.split_first() {
        None => Err(Failure::Usage("no command".to_owned())),
        Some((command, rest)) => match command.as_str() {
            "run" => run(registry, rest, out, err),
            "check" => check(registry, rest, out),
            "strategies" if rest.is_empty() => {
                strategies(registry, out);
                Ok(())
            }
            "version" | "--version" | "-V" if rest.is_empty() => {
                version(out);
                Ok(())
            }
            "help" | "--help" | "-h" => {
                let _ = out.write_all(USAGE.as_bytes());
                Ok(())
            }
            other => Err(Failure::Usage(format!("`{other}` is not a command"))),
        },
    };
    match result {
        Ok(()) => 0,
        Err(failure) => {
            let _ = writeln!(err, "error: {failure}");
            if matches!(failure, Failure::Usage(_)) {
                USAGE_ERROR
            } else {
                FAILED
            }
        }
    }
}

/// The whole program: runs the command line of the process and returns its status.
#[must_use]
pub fn main(registry: &Registry) -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let status = execute(
        registry,
        &arguments,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    ExitCode::from(status)
}
