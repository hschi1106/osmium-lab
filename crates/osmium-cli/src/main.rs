use std::{env, process::ExitCode};

fn main() -> ExitCode {
    osmium_cli::run(env::args_os().skip(1))
}
