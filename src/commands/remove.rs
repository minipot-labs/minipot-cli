use anyhow::{Context, Result};
use std::fs;
use std::io::{self, Write};

use crate::config::MinipotConfig;

pub fn execute() -> Result<()> {
    let config = MinipotConfig::load()?;
    let server_dir = config.server_dir();

    if !server_dir.exists() {
        println!("Nothing to remove: {} does not exist.", server_dir.display());
        return Ok(());
    }

    print!(
        "This will permanently delete {}. Are you sure? [y/N] ",
        server_dir.display()
    );
    io::stdout().flush().context("Failed to flush stdout")?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("Failed to read confirmation")?;

    if input.trim().eq_ignore_ascii_case("y") {
        fs::remove_dir_all(&server_dir)
            .with_context(|| format!("Failed to remove {}", server_dir.display()))?;
        println!("Removed {}.", server_dir.display());
    } else {
        println!("Aborted.");
    }

    Ok(())
}
