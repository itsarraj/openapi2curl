use std::fs;
use std::path::PathBuf;

use clap::Parser;
use openapi2curl::{curl, spec};

#[derive(Parser)]
#[command(
    name = "openapi2curl",
    about = "Generates ready-to-run curl commands for every operation in an OpenAPI 3.x spec"
)]
struct Cli {
    /// Path to an OpenAPI 3.x spec, JSON or YAML.
    spec: PathBuf,

    /// Override the base URL. Defaults to the spec's first `servers[].url`
    /// entry, falling back to http://localhost:8080 if the spec has none.
    #[arg(long)]
    base_url: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let content = fs::read_to_string(&cli.spec)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", cli.spec.display()))?;

    let parsed = spec::parse_spec(&content)?;
    let base_url = cli
        .base_url
        .or(parsed.base_url)
        .unwrap_or_else(|| "http://localhost:8080".to_string());

    if parsed.operations.is_empty() {
        println!("no operations found in spec");
        return Ok(());
    }

    for (i, op) in parsed.operations.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("# {} {}", op.method, op.path);
        println!("{}", curl::render_curl(op, &base_url));
    }

    Ok(())
}
