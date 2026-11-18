//! `osmium`: the command line with the example strategies. A program with other
//! strategies is the same three lines with its own registrations
//! (`docs/guide/writing-a-strategy.md`).

use std::process::ExitCode;

use osmium_strategy::Registry;

fn main() -> ExitCode {
    let mut registry = Registry::new();
    osmium_examples::register(&mut registry);
    osmium_cli::main(&registry)
}
