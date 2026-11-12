//! `rucene-cli`: see the crate README for every command and flag.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = Vec::new();
    for arg in std::env::args_os().skip(1) {
        match arg.into_string() {
            Ok(arg) => args.push(arg),
            Err(arg) => {
                eprintln!("rucene-cli: error: argument {arg:?} is not valid UTF-8");
                return ExitCode::from(2);
            }
        }
    }
    let stdout = std::io::stdout();
    match rucene_cli_example::cli::run(&args, &mut stdout.lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rucene-cli: error: {error}");
            ExitCode::from(error.exit_code())
        }
    }
}
