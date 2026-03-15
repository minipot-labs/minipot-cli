use anyhow::{Context, Result};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::config::MinipotConfig;
use crate::paper::{download_paper_jar, download_server_icon};
use crate::commands::stop::{PID_FILE, RESTART_MARKER};

/// Paper stampa questa stringa quando il server è pronto a ricevere comandi.
const SERVER_READY_SIGNAL: &str = "]: Done (";

pub fn execute() -> Result<()> {
    let config = MinipotConfig::load()?;

    if config.server.version.trim().is_empty() {
        anyhow::bail!(
            "No server version set in minipot.yml.\n\
             Open the file and set the `version` field (e.g. \"1.21.4\")."
        );
    }

    let server_dir = config.server_dir();

    // ── [1/4] Cartella server ─────────────────────────────────────────────────
    println!("[1/4] Preparing server directory...");
    if !server_dir.exists() {
        fs::create_dir_all(&server_dir)
            .with_context(|| format!("Failed to create {}", server_dir.display()))?;
    }
    let plugins_dir = server_dir.join("plugins");
    if !plugins_dir.exists() {
        fs::create_dir_all(&plugins_dir).context("Failed to create plugins directory")?;
    }
    let eula_path = server_dir.join("eula.txt");
    if !eula_path.exists() {
        fs::write(&eula_path, "eula=true\n").context("Failed to write eula.txt")?;
    }

    // ── [2/4] Paper JAR ───────────────────────────────────────────────────────
    println!("[2/4] Checking Paper {}...", config.server.version);
    download_paper_jar(&config.server.version, &server_dir)?;

    // ── [3/4] Icona server ────────────────────────────────────────────────────
    println!("[3/4] Checking server icon...");
    if let Err(e) = download_server_icon(&server_dir) {
        eprintln!("Warning: could not download server icon: {e}");
    }

    // ── [4/4] Loop avvio (gestisce anche restart) ──────────────────────────────
    let mut java_args: Vec<String> = config.server.jvm_flags.clone();
    java_args.extend(["-jar".to_string(), "paper.jar".to_string(), "nogui".to_string()]);

    let pid_path = server_dir.join(PID_FILE);
    let restart_marker = server_dir.join(RESTART_MARKER);

    loop {
        let startup_commands = config.server.startup_commands.clone();
        let n_cmds = startup_commands.len();

        println!(
            "[4/4] Starting Paper {} on port {}...",
            config.server.version, config.server.port
        );
        if n_cmds > 0 {
            println!(
                "      {n_cmds} startup command{} will run when the server is ready.",
                if n_cmds == 1 { "" } else { "s" }
            );
        }
        println!();

        let mut child = Command::new("java")
            .args(&java_args)
            .current_dir(&server_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Failed to launch java — make sure it is installed and in PATH")?;

        // Scrivi il PID così `minipot stop` e `minipot restart` possono trovare il processo
        fs::write(&pid_path, child.id().to_string())
            .context("Failed to write PID file")?;

        // Shared handle per scrivere sullo stdin del processo server
        let server_stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
        let server_stdin_for_commands = Arc::clone(&server_stdin);
        let server_stdin_for_user = Arc::clone(&server_stdin);

        let server_stdout = child.stdout.take().unwrap();

        // ── Thread stdout: stampa output, rileva "Done", invia startup commands ──
        let stdout_thread = thread::spawn(move || {
            let reader = BufReader::new(server_stdout);
            let mut commands_sent = false;

            for line in reader.lines() {
                let Ok(line) = line else { break };
                println!("{line}");

                if !commands_sent && line.contains(SERVER_READY_SIGNAL) {
                    commands_sent = true;

                    if startup_commands.is_empty() {
                        continue;
                    }

                    println!();
                    println!("[Minipot] Server ready — running startup commands...");

                    let mut writer = server_stdin_for_commands.lock().unwrap();
                    for cmd in &startup_commands {
                        println!("[Minipot] > {cmd}");
                        writeln!(writer, "{cmd}").ok();
                    }
                    writer.flush().ok();

                    println!("[Minipot] Done.");
                    println!();
                }
            }
        });

        // ── Thread stdin: forwarda l'input dell'utente al server ──────────────
        thread::spawn(move || {
            let reader = BufReader::new(std::io::stdin());
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let mut writer = server_stdin_for_user.lock().unwrap();
                writeln!(writer, "{line}").ok();
                writer.flush().ok();
            }
        });

        let status = child.wait().context("Failed to wait for server process")?;
        stdout_thread.join().ok();

        // Pulizia PID file
        fs::remove_file(&pid_path).ok();

        // Controlla se `minipot restart` ha lasciato il marker
        if restart_marker.exists() {
            fs::remove_file(&restart_marker).ok();
            println!();
            println!("[Minipot] Restarting server...");
            println!();
            continue; // riparte il loop
        }

        if !status.success() {
            anyhow::bail!("Server exited with status: {}", status);
        }

        break;
    }

    Ok(())
}
