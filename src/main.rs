mod cache;
mod commands;
mod config;
mod downloadable;
mod java;
mod jbr;
mod lock;
mod paper;
mod sources;

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
    /// Prepare the server environment without starting it (used by the IntelliJ plugin)
    Prepare,
    /// Start the local Paper server (downloads Paper if needed)
    Run {
        /// Force startup commands to run even if already executed in a previous session
        #[arg(long)]
        exec_commands: bool,
        /// Start the server with DCEVM hot-swap support (downloads JBR if needed)
        #[arg(long)]
        debug: bool,
        /// JDWP port IntelliJ will listen on (default: 5005)
        #[arg(long, default_value = "5005")]
        debug_port: u16,
    },
    /// Stop the running Paper server gracefully
    Stop,
    /// Restart the running Paper server
    Restart,
    /// Deploy the built plugin jar into the local server's plugins folder
    Sync,
    /// Remove the local server directory (asks for confirmation)
    Remove,
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

  # Dependency plugins to pre-install in the dev server before first startup.
  # Supports Modrinth, Hangar (official PaperMC registry), GitHub Releases, and direct URLs.
  # On first run, "version: latest" is resolved to the exact build and pinned in minipot.lock.
  # Commit minipot.lock so every teammate gets the exact same versions.
  #
  # Examples:
  #   plugins:
  #     - type: modrinth       # modrinth.com — most popular registry
  #       id: vault
  #       version: latest
  #
  #     - type: hangar         # hangar.papermc.io — official PaperMC registry
  #       id: LuckPerms/LuckPerms
  #       version: latest
  #
  #     - type: ghrel          # GitHub Releases
  #       repo: MilkBowl/Vault
  #       tag: latest
  #       asset: Vault.jar
  #
  #     - type: url            # direct download, no pinning
  #       url: https://example.com/myplugin.jar
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

    update_gitignore()?;

    println!("Project initialized.");
    println!();
    println!("Next steps:");
    println!("  1. Open minipot.yml and set the Paper version (e.g. \"1.21.4\")");
    println!("  2. Run `minipot run` to download Paper and start the server");
    println!("  3. Run `minipot sync` after building to deploy your plugin");
    println!("  4. Commit minipot.yml and minipot.lock to version control");
    Ok(())
}

fn update_gitignore() -> Result<()> {
    const GITIGNORE: &str = ".gitignore";
    const ENTRY: &str = "minipot-server/";

    let current = if Path::new(GITIGNORE).exists() {
        std::fs::read_to_string(GITIGNORE).context("Failed to read .gitignore")?
    } else {
        String::new()
    };

    if current.lines().any(|l| l.trim() == ENTRY) {
        return Ok(());
    }

    let separator = if current.is_empty() || current.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let addition = format!(
        "{separator}\n# minipot — generated server environment (not committed)\n{ENTRY}\n"
    );

    std::fs::write(GITIGNORE, current + &addition).context("Failed to write .gitignore")?;
    println!("  .gitignore updated (minipot-server/ excluded).");
    Ok(())
}


// ─── Entry point ──────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Init => cmd_init(),
        Command::Prepare => commands::prepare::execute(),
        Command::Run { exec_commands, debug, debug_port } => {
            commands::run::execute(exec_commands, debug, debug_port)
        }
        Command::Stop => commands::stop::execute(false),
        Command::Restart => commands::stop::execute(true),
        Command::Sync => commands::sync::execute(),
        Command::Remove => commands::remove::execute(),
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
