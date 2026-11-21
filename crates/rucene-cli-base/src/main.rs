//! `rucene-base`: see the crate README for every command and flag.

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = Vec::new();
    for arg in std::env::args_os().skip(1) {
        match arg.into_string() {
            Ok(arg) => args.push(arg),
            Err(arg) => {
                // Fallible write, as below: stderr may be closed.
                let _ = writeln!(
                    std::io::stderr(),
                    "rucene-base: error: argument {arg:?} is not valid UTF-8"
                );
                return ExitCode::from(2);
            }
        }
    }
    let stdout = std::io::stdout();
    match rucene_cli_base::cli::run(&args, &mut stdout.lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // A fallible write: `eprintln!` panics (exit 101) when stderr is
            // closed too, e.g. after the caller died with both pipes open.
            let _ = writeln!(std::io::stderr(), "rucene-base: error: {error}");
            ExitCode::from(error.exit_code())
        }
    }
}
