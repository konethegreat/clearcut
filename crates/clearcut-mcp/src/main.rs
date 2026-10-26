// Written by Kone Tshivhinda & Claude
//! ClearCut MCP server over stdio. Settings come from the environment (see `config`): never put a
//! key on the command line.

use clearcut_mcp::config::Config;
use clearcut_mcp::ClearCut;
use rmcp::ServiceExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = match Config::from_env(|name| std::env::var(name).ok()) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("error: {message}");
            std::process::exit(2);
        }
    };
    if config.api_key.is_none() {
        eprintln!("note: CLEARCUT_API_KEY is not set, so only the calculation tools will work");
    }
    let service = ClearCut::new(config).serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
