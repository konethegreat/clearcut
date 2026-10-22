// Written by Kone Tshivhinda & Claude
//! `clearcut-gold`: tools for the gold set. See [`clearcut_eval::cli`].

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = clearcut_eval::cli::run(&args, &mut std::io::stdout(), &mut std::io::stderr());
    ExitCode::from(code)
}
