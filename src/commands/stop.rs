use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use crate::config::MinipotConfig;

/// File in cui `minipot run` scrive il PID del processo Paper.
pub const PID_FILE: &str = ".minipot.pid";

/// File marker: se esiste quando Paper si spegne, `minipot run` fa ripartire il server.
pub const RESTART_MARKER: &str = ".minipot.restart";

/// Legge il PID dal file `.minipot.pid` nella cartella server.
pub fn read_pid(server_dir: &Path) -> Result<u32> {
    let pid_path = server_dir.join(PID_FILE);
    if !pid_path.exists() {
        anyhow::bail!(
            "No running server found.\n\
             Start one with `minipot run`."
        );
    }
    let raw = fs::read_to_string(&pid_path)
        .context("Failed to read PID file")?;
    raw.trim()
        .parse::<u32>()
        .context("PID file contains invalid data")
}

/// Invia SIGTERM al processo Paper (stop graceful).
/// Su Paper, SIGTERM attiva lo shutdown hook che salva il mondo e chiude le connessioni.
fn terminate(pid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        // SIGTERM: Paper lo intercetta e avvia lo shutdown sequence
        let result = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        if result != 0 {
            let err = std::io::Error::last_os_error();
            // errno ESRCH = no such process → il server si è già fermato
            if err.raw_os_error() == Some(libc::ESRCH) {
                anyhow::bail!("Server process (PID {pid}) is not running.");
            }
            return Err(err).context(format!("Failed to send SIGTERM to PID {pid}"));
        }
    }
    #[cfg(windows)]
    {
        // Su Windows usa taskkill /PID <pid> (non /F per dare tempo allo shutdown)
        let status = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string()])
            .status()
            .context("Failed to run taskkill")?;
        if !status.success() {
            anyhow::bail!("taskkill failed for PID {pid} — is the server still running?");
        }
    }
    Ok(())
}

/// Esegue `minipot stop` (restart=false) o `minipot restart` (restart=true).
pub fn execute(restart: bool) -> Result<()> {
    let config = MinipotConfig::load()?;
    let server_dir = config.server_dir();
    let pid = read_pid(&server_dir)?;

    if restart {
        // Scrivi il marker prima di uccidere il processo.
        // `minipot run` lo troverà dopo lo shutdown e farà ripartire il server.
        fs::write(server_dir.join(RESTART_MARKER), "")
            .context("Failed to write restart marker")?;
        println!("Restarting server (PID {pid})...");
    } else {
        println!("Stopping server (PID {pid})...");
    }

    terminate(pid)?;

    if restart {
        println!("Server is shutting down — it will restart automatically.");
    } else {
        println!("Stop signal sent. The server will shut down shortly.");
    }

    Ok(())
}
