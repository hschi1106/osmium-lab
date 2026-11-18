//! The command line, run in-process over the sample market of `osmium-fixture`.

use std::{fs, path::PathBuf};

use osmium_cli::execute;
use osmium_fixture::sample;
use osmium_strategy::Registry;

/// A directory for a configuration file and results, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("osmium-cli-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn config(&self, text: &str) -> String {
        let path = self.0.join("run.toml");
        fs::write(&path, text).unwrap();
        path.to_str().unwrap().to_owned()
    }

    fn results(&self) -> String {
        self.0.join("results").to_str().unwrap().to_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Runs a command line and returns the exit status, standard output and standard
/// error.
fn osmium(arguments: &[&str]) -> (u8, String, String) {
    let mut registry = Registry::new();
    osmium_examples::register(&mut registry);
    let arguments: Vec<String> = arguments.iter().map(|&text| text.to_owned()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let status = execute(&registry, &arguments, &mut out, &mut err);
    (
        status,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

#[test]
fn a_command_line_that_is_not_understood_shows_the_usage() {
    for arguments in [
        &[][..],
        &["backtest"],
        &["run"],
        &["run", "a.toml"],
        &["run", "a.toml", "--output", "dir"],
        &["check"],
        &["strategies", "all"],
    ] {
        let (status, out, err) = osmium(arguments);
        assert_eq!(status, 2, "{arguments:?}");
        assert!(out.is_empty());
        assert!(err.starts_with("error: "), "{err}");
        assert!(err.contains("Usage: osmium <command>"), "{err}");
    }
    let (status, out, err) = osmium(&["help"]);
    assert_eq!(status, 0);
    assert!(out.starts_with("Usage: osmium <command>"));
    assert!(err.is_empty());
}

#[test]
fn the_program_lists_its_strategies_and_versions() {
    let (status, out, _) = osmium(&["strategies"]);
    assert_eq!(status, 0);
    let names: Vec<&str> = out.lines().filter(|line| !line.starts_with(' ')).collect();
    assert_eq!(
        names,
        [
            "buy_and_hold (version 1)",
            "open_to_close (version 1)",
            "passive_quote (version 1)"
        ]
    );

    let (status, out, _) = osmium(&["version"]);
    assert_eq!(status, 0);
    assert!(out.starts_with(concat!("osmium ", env!("CARGO_PKG_VERSION"), "\n")));
    for name in [
        "marketdata schema  1",
        "decoder",
        "ordering",
        "matching rules",
        "ledger rules",
        "output",
    ] {
        assert!(out.contains(name), "{out}");
    }
}

#[test]
fn check_says_what_a_run_would_read() {
    let tree = sample::tree("cli-check");
    let scratch = Scratch::new("check");
    let config = scratch.config(&sample::config(&tree, "buy_and_hold", "symbol = \"2330\""));
    let (status, out, err) = osmium(&["check", &config]);
    assert_eq!((status, err.as_str()), (0, ""));
    assert_eq!(
        out,
        format!(
            "strategy      buy_and_hold (version 1)\n\
             marketdata    {}\n\
             feeds         TWSE_F6\n\
             instruments   all\n\
             trading days  5 (2026-10-15 to 2026-10-21)\n\
             missing       2026-10-19 TWSE_F6\n\
             missing       2026-10-20 TWSE_F6\n\
             missing       2026-10-21 TWSE_F6\n",
            tree.path().display()
        )
    );
    // Nothing was run.
    assert!(!scratch.0.join("results").exists());
}

#[test]
fn run_writes_the_results_and_prints_a_summary() {
    let tree = sample::tree("cli-run");
    let scratch = Scratch::new("run");
    let config = scratch.config(&sample::config(&tree, "open_to_close", "symbol = \"2330\""));
    let results = scratch.results();
    let (status, out, err) = osmium(&["run", &config, "--out", &results]);
    assert_eq!(status, 0, "{err}");
    assert_eq!(
        out,
        format!(
            "trading days  5\nmessages      16\norders        4\nfills         4\n\
             initial cash  1000000\nfinal cash    1000130\nfinal equity  1000130\n\
             results       {results}\n"
        )
    );
    // One line of progress per trading day.
    let progress: Vec<&str> = err.lines().collect();
    assert_eq!(progress.len(), 5);
    assert_eq!(
        progress[0],
        "2026-10-15 messages=8 orders=2 fills=2 equity=1000065"
    );
    assert!(scratch.0.join("results/run.json").exists());

    // The results are never written over.
    let (status, _, err) = osmium(&["run", "--out", &results, &config]);
    assert_eq!(status, 1);
    assert!(err.starts_with("error: "), "{err}");
    assert!(err.contains("results"), "{err}");
}

#[test]
fn mistakes_in_the_configuration_fail_before_anything_runs() {
    let tree = sample::tree("cli-mistakes");
    let scratch = Scratch::new("mistakes");
    let results = scratch.results();
    for (strategy, params, problem) in [
        (
            "momentum",
            "",
            "strategy.name: `momentum` is not a strategy of this program \
             (it has: buy_and_hold, open_to_close, passive_quote)",
        ),
        ("buy_and_hold", "", "strategy parameter `symbol` is missing"),
        (
            "buy_and_hold",
            "symbol = \"2330\"\nlot = 2",
            "strategy parameter `lot` is not a parameter of this strategy",
        ),
    ] {
        let config = scratch.config(&sample::config(&tree, strategy, params));
        for arguments in [
            &["check", config.as_str()][..],
            &["run", config.as_str(), "--out", results.as_str()],
        ] {
            let (status, out, err) = osmium(arguments);
            assert_eq!(status, 1, "{arguments:?}");
            assert!(out.is_empty());
            assert!(err.contains(problem), "{err}");
        }
    }
    let config = scratch.config("[run]\nstart = \"2026-10-15\"\n");
    let (status, _, err) = osmium(&["check", &config]);
    assert_eq!(status, 1);
    assert!(err.contains("run.toml: "), "{err}");

    let (status, _, err) = osmium(&["check", "/nonexistent/run.toml"]);
    assert_eq!(status, 1);
    assert!(err.contains("/nonexistent/run.toml: "), "{err}");

    assert!(!scratch.0.join("results").exists());
}
