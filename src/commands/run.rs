use anyhow::{Context, Result};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::commands::prepare::prepare_server;
use crate::commands::stop::{PID_FILE, RESTART_MARKER};
use crate::config::MinipotConfig;

/// Paper stampa questa stringa quando il server è pronto a ricevere comandi.
const SERVER_READY_SIGNAL: &str = "]: Done (";

/// Marker scritto dopo l'esecuzione degli startup commands.
/// Se esiste, i comandi non vengono rieseguiti ai successivi avvii (a meno che
/// l'utente non abbia rimosso il server con `minipot remove`).
const STARTUP_DONE_MARKER: &str = ".minipot.startup_done";

pub fn execute(force_exec_commands: bool, debug: bool, debug_port: u16) -> Result<()> {
    let config = MinipotConfig::load()?;

    if config.server.version.trim().is_empty() {
        anyhow::bail!(
            "No server version set in minipot.yml.\n\
             Open the file and set the `version` field (e.g. \"1.21.4\")."
        );
    }

    let server_dir = config.server_dir();

    prepare_server(&config, &server_dir)?;

    // ── Determina il binario Java da usare ────────────────────────────────────
    let java_binary: String;
    if debug {
        let java_version = crate::java::java_version_for_paper(&config.server.version);
        let bin = crate::jbr::ensure_installed(java_version)?;
        println!();
        println!("⚡ Debug mode: using JetBrains Runtime {java_version} (DCEVM)");
        println!("  → {}", bin.display());
        println!("  → JDWP connecting to localhost:{debug_port}");
        println!("  → Hot-swap enabled: -XX:+AllowEnhancedClassRedefinition");
        println!();
        java_binary = bin.to_string_lossy().into_owned();
    } else {
        java_binary = "java".to_string();
    }

    // ── Loop avvio (gestisce anche restart) ───────────────────────────────────
    let mut java_args: Vec<String> = config.server.jvm_flags.clone();
    if debug {
        java_args.push("-XX:+AllowEnhancedClassRedefinition".to_string());
        java_args.push(format!(
            "-agentlib:jdwp=transport=dt_socket,server=n,suspend=n,address=localhost:{debug_port}"
        ));
    }
    java_args.extend(["-jar".to_string(), "paper.jar".to_string(), "nogui".to_string()]);

    let pid_path = server_dir.join(PID_FILE);
    let restart_marker = server_dir.join(RESTART_MARKER);
    let startup_done_marker = server_dir.join(STARTUP_DONE_MARKER);

    loop {
        let startup_commands = config.server.startup_commands.clone();
        let n_cmds = startup_commands.len();
        let startup_already_run = startup_done_marker.exists() && !force_exec_commands;

        println!(
            "[Minipot] Starting Paper {} on port {}...",
            config.server.version, config.server.port
        );
        if n_cmds > 0 {
            if startup_already_run {
                println!(
                    "[Minipot] Startup commands skipped —  \
                     Run `minipot run --exec-commands` or `minipot remove` to re-run the commands."
                );
            } else {
                println!(
                    "[Minipot] {n_cmds} startup command{} will run when the server is ready.",
                    if n_cmds == 1 { "" } else { "s" }
                );
            }
        }
        println!();

        let mut child = Command::new(&java_binary)
            .args(&java_args)
            .current_dir(&server_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("[Minipot] Failed to launch {java_binary}"))?;

        // Scrivi il PID così `minipot stop` e `minipot restart` possono trovare il processo
        fs::write(&pid_path, child.id().to_string())
            .context("[Minipot] Failed to write PID file")?;

        // Shared handle per scrivere sullo stdin del processo server
        let server_stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
        let server_stdin_for_commands = Arc::clone(&server_stdin);
        let server_stdin_for_user = Arc::clone(&server_stdin);

        let server_stdout = child.stdout.take().unwrap();

        // ── Thread stdout: stampa output, rileva "Done", invia startup commands ──
        let startup_done_marker_clone = startup_done_marker.clone();
        let stdout_thread = thread::spawn(move || {
            let reader = BufReader::new(server_stdout);
            let mut commands_sent = false;

            for line in reader.lines() {
                let Ok(line) = line else { break };
                println!("{line}");

                if !commands_sent && line.contains(SERVER_READY_SIGNAL) {
                    commands_sent = true;

                    if startup_commands.is_empty() || startup_already_run {
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

                    // Segna che i comandi sono stati eseguiti: nei prossimi avvii
                    // (finché il server non viene rimosso) verranno saltati.
                    fs::write(&startup_done_marker_clone, "").ok();
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

        let status = child.wait().context("[Minipot] Failed to wait for server process")?;
        stdout_thread.join().ok();

        // Pulizia PID file
        fs::remove_file(&pid_path).ok();

        // Controlla se `minipot restart` ha lasciato il marker
        if restart_marker.exists() {
            fs::remove_file(&restart_marker).ok();
            println!();
            println!("[Minipot] Restarting server...");
            println!();
            continue;
        }

        if !status.success() {
            anyhow::bail!("[Minipot] Server exited with status: {}", status);
        }

        break;
    }

    Ok(())
}
