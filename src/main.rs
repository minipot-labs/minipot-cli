mod commands;
mod config;
mod paper;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use config::CONFIG_FILE;
use std::path::Path;

// ─── CLI ─────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "minipot",
    about = "Dev toolchain for Minecraft plugin developers",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a new Minipot project in the current directory
    Init,
    /// Start the local Paper server (downloads Paper if needed)
    Run,
    /// Stop the running Paper server gracefully
    Stop,
    /// Restart the running Paper server
    Restart,
    /// Deploy the built plugin jar into the local server's plugins folder
    Sync,
    /// Manage Mineflayer bots for player simulation
    Bot {
        #[command(subcommand)]
        action: BotAction,
    },
}

#[derive(Subcommand)]
enum BotAction {
    /// Spawn a new bot
    Spawn { name: String },
    /// List active bots
    List,
    /// Stop a bot
    Stop { name: String },
}

// ─── Init ─────────────────────────────────────────────────────────────────────

const MINIPOT_YML_TEMPLATE: &str = r#"# ─────────────────────────────────────────────────────────
#  minipot.yml — Minipot project configuration
#  Docs: https://minipot.io/docs
# ─────────────────────────────────────────────────────────

server:
  # Paper server version to run.
  # Find all available versions at: https://papermc.io/downloads/paper
  # Example: "1.21.4", "1.20.6", "1.19.4"
  version: ""

  # Server software. Currently only "paper" is supported.
  type: paper

  # Port the server will listen on.
  port: 25565

  # Extra plugin JARs to pre-install before first startup (coming soon).
  plugins: []

  # JVM flags passed to the Paper server process.
  # The defaults below work well for most local development setups.
  # Increase -Xmx if the server crashes with out-of-memory errors.
  jvm_flags:
    - -Xms512M       # Initial heap allocation
    - -Xmx2G         # Maximum heap size
    - -XX:+UseG1GC   # G1 garbage collector — recommended by Paper

  # Commands sent automatically to the Paper console once the server is ready.
  # Write them as you would type them in the server console — without the leading slash.
  # Minipot will handle the execution automatically.
  #
  # Note: commands that target a specific player (e.g. "gamemode") will not work unless,
  # by some miracle, that player happens to be logged in right when they execute.
  #
  # Example:
  #   startup_commands:
  #     - op YourUsername
  #     - gamerule doDaylightCycle false
  #     - gamerule doWeatherCycle false
  startup_commands: []

# Mineflayer bots to simulate player behaviour during development.
# Each bot can optionally run a script from the Minipot Marketplace.
#
# Example:
#   bots:
#     - name: tester
#       script: basic-movement.js
bots: []
"#;

fn cmd_init() -> Result<()> {
    if Path::new(CONFIG_FILE).exists() {
        println!("minipot.yml already exists in this directory.");
        return Ok(());
    }

    std::fs::write(CONFIG_FILE, MINIPOT_YML_TEMPLATE)
        .context("Failed to write minipot.yml")?;

    println!("Project initialized.");
    println!();
    println!("Next steps:");
    println!("  1. Open minipot.yml and set the Paper version (e.g. \"1.21.4\")");
    println!("  2. Run `minipot run` to download Paper and start the server");
    println!("  3. Run `minipot sync` after building to deploy your plugin");
    Ok(())
}

// ─── Entry point ──────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Init => cmd_init(),
        Command::Run => commands::run::execute(),
        Command::Stop => commands::stop::execute(false),
        Command::Restart => commands::stop::execute(true),
        Command::Sync => commands::sync::execute(),
        Command::Bot { action } => commands::bot::execute(match action {
            BotAction::Spawn { name } => commands::bot::BotAction::Spawn { name },
            BotAction::List => commands::bot::BotAction::List,
            BotAction::Stop { name } => commands::bot::BotAction::Stop { name },
        }),
    };

    if let Err(e) = result {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}
