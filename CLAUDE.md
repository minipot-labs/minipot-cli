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
├── .cargo/config.toml      — linker Windows per cross-compilazione (x86_64-pc-windows-gnu)
├── src/
│   ├── main.rs             — entry point, CLI (clap), cmd_init(), update_gitignore()
│   ├── config.rs           — MinipotConfig, lettura/scrittura minipot.yml
│   ├── lock.rs             — MinipotLock + LockedPlugin, lettura/scrittura minipot.lock
│   ├── paper.rs            — PaperMC API ufficiale, resolve_latest_build(), download_paper_jar() con SHA256
│   ├── downloadable.rs     — Downloadable enum, SourceContext, ResolvedFile, CacheStrategy, Resolvable trait
│   ├── cache.rs            — Cache struct (~/.cache/minipot/), get/exists/read/write JSON
│   ├── sources/
│   │   ├── mod.rs          — aggregator
│   │   ├── modrinth.rs     — ModrinthAPI: resolve versioni Paper-compatibili con rate limit
│   │   ├── hangar.rs       — HangarAPI: registro ufficiale PaperMC, SHA256 nativo, paginazione
│   │   └── github.rs       — GithubAPI: GitHub Releases con ETag cache per rate limit
│   └── commands/
│       ├── mod.rs          — aggregator moduli
│       ├── prepare.rs      — prepare_server(); download plugin parallelo (tokio JoinSet); cache; pulizia
│       ├── run.rs          — avvio server Paper, thread stdout/stdin, startup_commands, restart loop
│       ├── stop.rs         — stop graceful (SIGTERM/taskkill), restart marker
│       ├── sync.rs         — deploy plugin JAR in plugins/, versioning automatico (Gradle + Maven)
│       ├── remove.rs       — rimuove minipot-server/ con conferma interattiva
│       └── bot.rs          — stub Mineflayer (non implementato)
└── minipot-test/
    └── minipot.yml         — file di configurazione d'esempio per test locali
```

---

## Responsabilità per file

### `src/main.rs`
- Definisce `struct Cli` e `enum Command` tramite clap derive
- Comandi: `Init`, `Prepare`, `Run`, `Stop`, `Restart`, `Sync`, `Remove`, `Bot`
- Contiene `MINIPOT_YML_TEMPLATE` — il template YAML embedded nel binario (commenti in inglese)
- `fn cmd_init()` — scrive minipot.yml se non esiste già, poi chiama `update_gitignore()`
- `fn update_gitignore()` — aggiunge `minipot-server/` al `.gitignore` se non già presente

### `src/lock.rs`
- `MinipotLock` — struttura root di `minipot.lock`
  - `paper_build: u32` — numero build Paper esatto
  - `paper_sha256: String` — SHA256 atteso del JAR
  - `paper_url: String` — URL diretto di download (non richiede API alla seconda run)
  - `plugins: Vec<LockedPlugin>` — dipendenze pinnate (vuoto se nessuna)
- `LockedPlugin` — versione pinned di un plugin: `source` (Downloadable dichiarato), `filename`, `url`, `sha256`, `size`
  - Il campo `source` viene confrontato con minipot.yml per rilevare cambiamenti → ri-risoluzione automatica
- `MinipotLock::load()` → `Result<Option<Self>>` — ritorna `None` se il file non esiste
- `MinipotLock::save()` — serializza come JSON pretty-printed
- Costante `LOCK_FILE = "minipot.lock"` — deve essere committato nel repo, mai gitignored

### `src/config.rs`
- `MinipotConfig` — struttura root del file minipot.yml
- `ServerConfig` — versione Paper, porta, JVM flags, startup_commands, `plugins: Vec<Downloadable>`
- `BotConfig` — nome bot e script Mineflayer opzionale
- `MinipotConfig::load()` — legge minipot.yml da disco
- `MinipotConfig::save()` — serializza in YAML e scrive
- `MinipotConfig::server_dir()` — ritorna `PathBuf("minipot-server")`
- Costante `CONFIG_FILE = "minipot.yml"`

### `src/paper.rs`
- Usa l'API ufficiale PaperMC v2: `https://api.papermc.io/v2/projects/paper`
- `resolve_latest_build(version)` → `Result<PaperBuild>` — chiama `/versions/{v}/builds`, filtra `channel == "default"`, prende l'ultima. Restituisce build number, SHA256 e URL diretto.
- `download_paper_jar(url, expected_sha256, server_dir)` — scarica paper.jar in streaming con progress bar `"█▓░"` magenta; calcola SHA256 durante il download; errore esplicito se hash non coincide; skip se file già presente e hash corretto; cancella il file se il download fallisce la verifica
- `sha256_of_file(path)` — hash SHA256 di un file già presente su disco (privata)
- `download_server_icon(server_dir)` — scarica server-icon.png da S3 Minipot; skip se già presente

### `src/downloadable.rs`
- `SourceContext` — contesto passato ai resolver: `http_client: reqwest::Client` + `mc_version: String`
- `ResolvedFile` — risultato della risoluzione: url, filename, size, hashes (BTreeMap), cache strategy
- `CacheStrategy` — `File { namespace, path }` o `None`
- `Resolvable` — trait con `async fn resolve_source(&self, ctx: &SourceContext) -> Result<ResolvedFile>`
- `Downloadable` — enum sorgenti dichiarabili in minipot.yml:
  - `Url { url, filename? }` — URL diretto, no cache
  - `Modrinth { id, version }` — alias `mr`
  - `Hangar { id, version }`
  - `GithubRelease { repo, tag, asset }` — serializzato come `type: ghrel`
- Display: `modrinth:vault@latest`, `hangar:LuckPerms/LuckPerms@latest`, ecc. (usato nelle progress bar)

### `src/cache.rs`
- `Cache(PathBuf)` — wrapper attorno a una directory di cache
- `Cache::cache_root()` → `~/.cache/minipot/`
- `Cache::get(namespace)` → `Option<Cache>` per `~/.cache/minipot/{namespace}/`
- `exists(path)`, `path(path)`, `get_json`, `try_get_json`, `write_json`
- Usata da: `prepare.rs` (cache plugin JAR), `github.rs` (cache ETag response API)

### `src/sources/modrinth.rs`
- `ModrinthAPI<'a>(&'a SourceContext)` — wrapper API Modrinth v2
- `filter_versions()` — filtra per mc_version e loader (paper/spigot/bukkit/purpur/folia); fallback senza filtro loader se nessuna versione trovata
- `fetch_version(id, version)` — risolve `"latest"` o versione specifica (per id, name, version_number)
- `resolve_source(id, version)` → `ResolvedFile` con hashes (sha1+sha256) e cache `modrinth/{id}/{version_id}/{filename}`
- Rate limit: `ModrinthWaitRatelimit` trait su `reqwest::Response` — attende se `x-ratelimit-remaining == 1`

### `src/sources/hangar.rs`
- `HangarAPI<'a>(&'a SourceContext)` — wrapper API Hangar v1 (registro ufficiale PaperMC)
- Platform fisso a `Paper` — minipot supporta solo server Paper
- Paginazione automatica in `get_project_version()` (da mcman)
- `resolve_source(id, version)` → `ResolvedFile` con sha256 nativo da `FileInfo` e cache `hangar/{id}/{version}/{filename}`
- `id` formato: `owner/slug` (es. `LuckPerms/LuckPerms`) o solo `slug`

### `src/sources/github.rs`
- `GithubAPI<'a>(&'a SourceContext)` — wrapper GitHub Releases API
- ETag cache delle risposte API in `~/.cache/minipot/github/` — protegge il rate limit di 60 req/h senza token
- `fetch_asset(repo, tag, asset_name)` — risolve `"latest"` o tag specifico; asset per nome esatto o pattern
- `resolve_source()` → `ResolvedFile` senza hash (GitHub non li espone nelle API releases) — SHA256 calcolato al download e memorizzato nel lock
- Token GitHub configurabile in futuro (attualmente non richiesto)

### `src/commands/prepare.rs`
- `prepare_server(config, server_dir)` — funzione pubblica condivisa da `prepare::execute()` e `run::execute()`
  - [1/4] Crea `minipot-server/` e `plugins/`, scrive `eula.txt`
  - [2/4] Legge lock → usa build pinned; se assente risolve latest e scrive lock
  - [3/4] Scarica plugin di dipendenza in **parallelo** con `tokio::task::JoinSet` + `indicatif::MultiProgress`; pulizia automatica JAR rimossi da minipot.yml
  - [4/4] Scarica `server-icon.png` (non fatale)
- `download_plugins()` — crea `SourceContext`, spawna un task tokio per plugin, raccoglie risultati
- `download_single_plugin()` — risolve da lock o API, chiama `download_resolved_file()`
- `download_resolved_file()` — check file esistente (hash) → check cache → download + SHA256 → salva in cache → copia in dest
- `derive_cache_strategy()` — ricostruisce `CacheStrategy` da `Downloadable` + filename (evita ri-chiamata API per plugin già in lock)
- Runtime tokio creato con `Builder::new_multi_thread().enable_all().build()` — **non** modifica `main()` (run.rs usa thread std)
- `execute()` — entry point per `minipot prepare`
- **Richiede `minipot.yml`** — errore esplicito se non esiste

### `src/commands/run.rs`
- Avvia il processo Java con JVM flags da config
- Scrive PID in `.minipot.pid` (nella server_dir)
- Thread stdout: legge output Paper, detecta `"]: Done ("` → invia startup_commands
- Thread stdin: forwarda input utente alla console Paper
- Restart loop: controlla marker `.minipot.restart` dopo shutdown, se presente riavvia
- **Flag `--exec-commands`** — forza l'esecuzione degli startup_commands anche se `.minipot.startup_done` è presente
- **Marker `.minipot.startup_done`** — scritto dopo la prima esecuzione degli startup_commands

### `src/commands/stop.rs`
- Legge PID da `.minipot.pid`
- Unix: `libc::kill(pid, SIGTERM)` — Windows: `taskkill /PID <pid>`
- `execute(restart: bool)` — se restart=true scrive `.minipot.restart` prima di killare
- Costanti: `PID_FILE = ".minipot.pid"`, `RESTART_MARKER = ".minipot.restart"`

### `src/commands/remove.rs`
- Chiede conferma `[y/N]` all'utente
- Se confermato, chiama `fs::remove_dir_all(server_dir)` su `minipot-server/`

### `src/commands/sync.rs`
- Cerca il JAR più recente per timestamp in `build/libs/` (Gradle) e `target/` (Maven)
- Vince il JAR con timestamp di modifica più recente tra le due directory; directory mancante ignorata
- Apre il JAR come archivio ZIP, legge `plugin.yml` interno, estrae il campo `name:`
- Rimuove versioni precedenti dello stesso plugin da `plugins/`
- Copia il nuovo JAR in `minipot-server/plugins/`

### `src/commands/bot.rs`
- Stub non implementato. Definisce `BotAction` (Spawn, List, Stop) e stampa placeholder.
- **Prossimo sviluppo:** avviare subprocess Node.js con script Mineflayer generato da LLM

---

## IPC e file runtime

| File | Dove | Scopo |
|---|---|---|
| `.minipot.pid` | `minipot-server/` | PID del processo Java, scritto da `run.rs`, letto da `stop.rs` |
| `.minipot.restart` | `minipot-server/` | Marker restart: `stop.rs` lo scrive, `run.rs` lo detecta al riavvio |
| `.minipot.startup_done` | `minipot-server/` | Marker startup commands: scritto da `run.rs` dopo la prima esecuzione |

---

## Riprodubilità dell'ambiente

| File | Dove | Committed |
|---|---|---|
| `minipot.yml` | root progetto | ✅ sì |
| `minipot.lock` | root progetto | ✅ sì — va committato |
| `minipot-server/` | root progetto | ❌ gitignored |

**Flusso lock:**
- Prima run senza lock → risolve build Paper + versioni plugin → scrive `minipot.lock`
- Run successive con lock → usa URL e SHA256 pinnati, nessuna chiamata API
- Cambio versione in minipot.yml → `source` nel lock non corrisponde → ri-risoluzione automatica
- `minipot remove` + `minipot run` → plugin non riscaricati da internet (sono in `~/.cache/minipot/`)

---

## Cross-compilazione Windows

**Prerequisiti (Fedora/WSL2):**
```bash
sudo dnf install -y mingw64-gcc
rustup target add x86_64-pc-windows-gnu
```

**Build:**
```bash
cargo build --release --target x86_64-pc-windows-gnu
# output: target/x86_64-pc-windows-gnu/release/minipot.exe
```

Il linker è configurato in `.cargo/config.toml`. Il binario è standalone — nessun installer.

**Setup Windows per l'utente finale:**
1. Copiare `minipot.exe` in una cartella (es. `C:\tools\minipot\`)
2. Aggiungere `C:\tools\minipot\` al PATH di sistema
3. Installare JDK 17+ (Oracle o Temurin); aggiungere `C:\Program Files\Java\jdk-17\bin` al PATH
4. Rimuovere eventuali entry precedenti (es. JDK 1.8) dal PATH per evitare conflitti
5. Riaprire il terminale — verificare con `java -version` e `minipot --version`

---

## Dipendenze chiave (Cargo.toml)

| Crate | Uso |
|---|---|
| `clap 4` (derive) | Parsing CLI con macro |
| `serde` + `serde_yaml` | Serializzazione minipot.yml |
| `serde_json` | Parsing risposte API JSON e minipot.lock |
| `anyhow` | Error handling con contesto |
| `reqwest 0.12` (blocking, json) | HTTP per PaperMC API e download JAR (blocking); client async per plugin sources |
| `zip 2` | Lettura plugin.yml dentro i JAR |
| `libc` | SIGTERM su Unix per stop graceful |
| `sha2 0.10` | Calcolo SHA256 JAR e plugin |
| `hex 0.4` | Encode SHA256 digest in stringa esadecimale |
| `tokio 1` (rt-multi-thread, fs, time) | Runtime async per download plugin parallelo |
| `dirs 5` | Percorso cache OS (`~/.cache/minipot/`) |
| `indicatif 0.17` | Progress bar download |

---

## Prossimi step pianificati

### Priorità alta
- **`bot.rs`** — implementazione reale: subprocess Node.js con script Mineflayer
  - Direzione: generazione AI (LLM via tokio) partendo da linguaggio naturale
  - Il developer descrive lo scenario → Minipot legge `plugin.yml` → chiama LLM → esegue script sandboxato
  - Sandbox Node.js: `--experimental-permission --allow-net=localhost:25565`
- **Server HTTP axum** su `localhost:7420` — REST API + WebSocket (pre-requisito Web UI)
  - WebSocket per: log server real-time, stato bot, eventi build
  - REST per: start/stop server, gestione preset, spawn bot

### Priorità media
- `run.rs` — output Paper nel terminale ha ancora gestione stdin/stdout basica; valutare log colorati

### Priorità bassa (post-v1)
- `minipot test` — comando headless CI/CD
- Bot Orchestrator — esecuzione parallela di scenari multipli
- Sistema crediti AI nel Pro Plan

---

## File di configurazione utente (minipot.yml)

```yaml
server:
  version: "1.21.4"
  type: paper
  port: 25565

  plugins:
    - type: hangar
      id: LuckPerms/LuckPerms
      version: latest
    - type: modrinth
      id: vault
      version: latest

  jvm_flags:
    - -Xms512M
    - -Xmx2G
    - -XX:+UseG1GC
  startup_commands: []

bots: []
```

**Nota:** `startup_commands` viene rilevato quando l'output Paper contiene `"]: Done ("`.
La qualità dei test AI dipende dalla ricchezza del `plugin.yml` del progetto — garbage in, garbage out.
