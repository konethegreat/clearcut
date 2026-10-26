// Written by Kone Tshivhinda & Claude
//! Thin entry point for the operator command. All the logic is in the library.

use clap::Parser;
use clearcut_admin::{run, Cli};
use clearcut_store::Store;
use std::io;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    // Making a master key needs no database, so it is done first.
    if matches!(cli.command, clearcut_admin::Command::GenerateMasterKey) {
        return match clearcut_admin::generate_master_key(&mut io::stdout(), &mut io::stderr()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        };
    }

    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("error: DATABASE_URL must be set to the OWNER database connection");
        return ExitCode::from(2);
    };
    let pool = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
    {
        Ok(pool) => pool,
        // The URL is deliberately not printed: it may contain a password.
        Err(error) => {
            eprintln!("error: could not connect to the database: {error}");
            return ExitCode::FAILURE;
        }
    };

    let store = Store::new(pool);
    match run(cli.command, &store, &mut io::stdout(), &mut io::stderr()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
