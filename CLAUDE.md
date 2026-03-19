# CLAUDE.md — minipot-cli

Guida di navigazione per Claude Code. Descrive dove si trova ogni responsabilità nel codice.

---

## Cos'è questo progetto

CLI Rust (`minipot`) — backbone dell'ecosistema Minipot per developer di plugin Minecraft.
Binario singolo, zero dipendenze esterne a runtime. Espone comandi per gestire un server Paper locale durante lo sviluppo.

**Principio guida:** la CLI è il prodotto. Web UI e plugin IntelliJ sono layer UX sopra di essa.

---

## Struttura file

```
minipot-cli/
├── Cargo.toml              — dipendenze e configurazione build
├── src/
│   ├── main.rs             — entry point, definizione CLI (clap), routing comandi, cmd_init()
│   ├── config.rs           — MinipotConfig, lettura/scrittura minipot.yml
│   ├── paper.rs            — PaperMC API fetch, download paper.jar e server-icon.png
│   └── commands/
│       ├── mod.rs          — aggregator moduli
│       ├── run.rs          — avvio server Paper, thread stdout/stdin, startup_commands, restart loop
│       ├── stop.rs         — stop graceful (SIGTERM/taskkill), restart marker
│       ├── sync.rs         — deploy plugin JAR in plugins/, versioning automatico
│       └── bot.rs          — stub Mineflayer (non implementato)
└── minipot-test/
    └── minipot.yml         — file di configurazione d'esempio per test locali
```

---

## Responsabilità per file

### `src/main.rs`
- Definisce `struct Cli` e `enum Command` tramite clap derive
- Comandi: `Init`, `Run`, `Stop`, `Restart`, `Sync`, `Bot`
- Contiene `MINIPOT_YML_TEMPLATE` — il template YAML embedded nel binario
- `fn cmd_init()` — scrive minipot.yml se non esiste già

### `src/config.rs`
- `MinipotConfig` — struttura root del file minipot.yml
- `ServerConfig` — versione Paper, porta, JVM flags, startup_commands
- `BotConfig` — nome bot e script Mineflayer opzionale
- `MinipotConfig::load()` — legge minipot.yml da disco
- `MinipotConfig::save()` — serializza in YAML e scrive
- `MinipotConfig::server_dir()` — ritorna `PathBuf("minipot-server")`
- Costante `CONFIG_FILE = "minipot.yml"`

### `src/paper.rs`
- `PaperApiResponse` — risposta API con campo `latest` e mappa `versions`
- `PaperApiResponse::fetch()` — GET a `https://qing762.is-a.dev/api/papermc`
- `download_paper_jar(version, server_dir)` — scarica paper.jar se non esiste
- `download_server_icon(server_dir)` — scarica server-icon.png da S3 Minipot
- URL S3 icona: `https://minipot-assets.s3.eu-central-1.amazonaws.com/minipot-icon-server.png`

### `src/commands/run.rs`
- Avvia il processo Java con JVM flags da config
- Scrive PID in `.minipot.pid` (nella server_dir)
- Thread stdout: legge output Paper, detecta `"]: Done ("` → invia startup_commands
- Thread stdin: forwarda input utente alla console Paper
- Restart loop: controlla marker `.minipot.restart` dopo shutdown, se presente riavvia
- Scarica paper.jar e server-icon.png tramite `paper.rs` se non presenti

### `src/commands/stop.rs`
- Legge PID da `.minipot.pid`
- Unix: `libc::kill(pid, SIGTERM)` — Windows: `taskkill /PID <pid>`
- `execute(restart: bool)` — se restart=true scrive `.minipot.restart` prima di killare
- Costanti: `PID_FILE = ".minipot.pid"`, `RESTART_MARKER = ".minipot.restart"`

### `src/commands/sync.rs`
- Cerca il JAR più recente per timestamp in `build/libs/` (Gradle)
- Apre il JAR come archivio ZIP, legge `plugin.yml` interno, estrae il campo `name:`
- Rimuove versioni precedenti dello stesso plugin da `plugins/`
- Copia il nuovo JAR in `minipot-server/plugins/`
- Supporta sia Gradle (`build/libs/`) che Maven (`target/`) — vince il JAR più recente per timestamp

### `src/commands/bot.rs`
- Stub non implementato. Definisce `BotAction` (Spawn, List, Stop) e stampa placeholder.
- **Prossimo sviluppo:** avviare subprocess Node.js con script Mineflayer generato da LLM

---

## IPC e file runtime

| File | Dove | Scopo |
|---|---|---|
| `.minipot.pid` | `minipot-server/` | PID del processo Java, scritto da `run.rs`, letto da `stop.rs` |
| `.minipot.restart` | `minipot-server/` | Marker restart: `stop.rs` lo scrive, `run.rs` lo detecta al riavvio |

---

## Dipendenze chiave (Cargo.toml)

| Crate | Uso |
|---|---|
| `clap 4` (derive) | Parsing CLI con macro |
| `serde` + `serde_yaml` | Serializzazione minipot.yml |
| `serde_json` | Parsing risposte API JSON |
| `anyhow` | Error handling con contesto |
| `reqwest 0.12` (blocking, json) | HTTP per PaperMC API e download JAR |
| `zip 2` | Lettura plugin.yml dentro i JAR |
| `libc` | SIGTERM su Unix per stop graceful |
| `tokio` | (da aggiungere) pre-requisito per axum e chiamate LLM async |

---

## Prossimi step pianificati

### Priorità alta
- **`bot.rs`** — implementazione reale: subprocess Node.js con script Mineflayer
  - Direzione: generazione AI (LLM via reqwest/tokio) partendo da linguaggio naturale
  - Il developer descrive lo scenario → Minipot legge `plugin.yml` → chiama LLM → esegue script sandboxato
  - Sandbox Node.js: `--experimental-permission --allow-net=localhost:25565`
- **Server HTTP axum** su `localhost:7420` — REST API + WebSocket (pre-requisito Web UI)
  - WebSocket per: log server real-time, stato bot, eventi build
  - REST per: start/stop server, gestione preset, spawn bot

### Priorità media
- `run.rs` — progress bar per download Paper
- `run.rs` — progress bar per download Paper

### Priorità bassa (post-v1)
- `minipot test` — comando headless CI/CD
- Bot Orchestrator — esecuzione parallela di scenari multipli
- Sistema crediti AI nel Pro Plan

---

## File di configurazione utente (minipot.yml)

```yaml
server:
  version: "1.21.4"   # Versione Paper — obbligatoria
  type: paper
  port: 25565
  jvm_flags:
    - -Xms512M
    - -Xmx2G
    - -XX:+UseG1GC
  startup_commands: [] # Comandi inviati alla console Paper appena server è ready

bots: []
```

**Nota:** `startup_commands` viene rilevato quando l'output Paper contiene `"]: Done ("`.
La qualità dei test AI dipende dalla ricchezza del `plugin.yml` del progetto — garbage in, garbage out.
