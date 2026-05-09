# PSH — Prompt Shell
**Alt names:** NSH (Natural Shell), ISH (Intent Shell)

PSH is a shell wrapper that sits between your terminal and bash. You use your existing terminal (GNOME, WezTerm, whatever) and your existing workflows — PSH just makes it smarter. Natural language is a first-class input. Errors are explained automatically. Everything runs locally.

**Key features:**
- No new terminal needed — configure GNOME Terminal (or any terminal) to launch PSH instead of bash in one setting
- Natural language input — prefix with `>` or just describe intent in plain English
- Auto error explanation — non-zero exit triggers instant plain English diagnosis
- Full bash/zsh/fish compatibility — detects and wraps whatever shell is configured on the machine
- Local AI — Ollama runs on your machine, nothing leaves it
- Persistent context — SQLite stores every command, output, cwd, and exit code per session
- OS + shell aware — always generates correct syntax for your environment
- Works offline — no account, no API key, no internet required
- Agentic mode — give a high-level goal, PSH plans and runs a sequence of commands with your confirmation

---

## Architecture

```
GNOME Terminal (or any terminal)
        │
        ▼
┌───────────────────────────────────────────┐
│  PSH  (/usr/local/bin/psh)                 │
│                                           │
│  Input Interceptor                        │
│  ── detects: shell command or NL intent?  │
│                                           │
│  ┌─────────────┐    ┌─────────────────┐  │
│  │ pass to bash │    │ NL Handler      │  │
│  └──────┬──────┘    │ → Ollama        │  │
│         │           │ ← command back  │  │
│         │           │ → confirm? y/n  │  │
│         │           └────────┬────────┘  │
│         └──────────┬─────────┘           │
│                    │                     │
│  Output Interceptor                      │
│  ── captures output + exit code          │
│  ── stores to SQLite                     │
│  ── on failure → Ollama for explanation  │
│                                          │
│  Context Store  (~/.psh/history.db)      │
└────────────────────┬─────────────────────┘
                     │ PTY
                     ▼
                  bash
                  (all execution happens here)
```

**Ollama** runs as a local daemon (`localhost:11434`). PSH auto-starts it on launch if not running. Called in two cases: NL input detected, or command exits with non-zero code.

**SQLite** at `~/.psh/history.db` stores every command, output, exit code, cwd, and timestamp. This is the AI's memory — the last N commands are injected into every Ollama prompt as context.

---

## Stack

| Layer | Choice | Reason |
|---|---|---|
| Language | Rust | Compiles to a single native binary, instant startup, memory safe |
| Shell execution | portable-pty | Spawns bash/zsh/fish, taps full I/O stream. Same library WezTerm uses |
| AI runtime | Ollama | Local HTTP API, auto-managed, no config needed |
| Model | gemma3:4b | Runs on 4GB RAM, ~2.5GB download, fast responses on CPU |
| HTTP client | reqwest (blocking) | Calls Ollama API synchronously |
| Persistence | rusqlite + SQLite | Zero setup, file-based, queryable history |
| Config | TOML (`~/.psh/config.toml`) | Simple, human-readable |

**Why Rust:** PSH starts every time a terminal tab opens. A Python process adds 150-200ms of startup latency — noticeable on every tab. A compiled Rust binary starts in under 5ms. PSH ships as a single binary with no runtime dependency. Users download one file, `chmod +x`, done.

**Why not Python:** Fastest to prototype but wrong for a shell. Startup time matters here more than almost any other class of program.

**Why not Go:** Go would also work and compiles to a single binary. Rust was chosen for memory safety guarantees — PSH sits between the user and their shell with root-level access to the I/O stream. Rust makes that safer by design.

---

## Data

```
~/.psh/
├── history.db       SQLite — all commands, outputs, exit codes, cwd, timestamps
├── config.toml      model choice, behaviour flags, custom prompt
└── psh.log          debug log
```

```sql
CREATE TABLE history (
    id          INTEGER PRIMARY KEY,
    timestamp   DATETIME,
    cwd         TEXT,
    command     TEXT,
    output      TEXT,
    exit_code   INTEGER,
    was_nl      BOOLEAN,
    nl_input    TEXT,
    session_id  TEXT
);
```

---

## Source

```
psh/src/
├── main.rs        entry point — loads config, opens db, starts PTY
├── config.rs      loads ~/.psh/config.toml with sensible defaults
├── context.rs     SQLite — stores every command, output, exit code, cwd
├── ai.rs          calls Ollama — NL translation + error explanation
└── pty.rs         spawns underlying shell, intercepts stream, detects NL input
```

## Install

**Option A — GNOME Terminal only (recommended for development)**
```
Preferences → Profile → Command
→ Run a custom command instead of my shell
→ /usr/local/bin/psh
```
No system changes. Revert by unchecking one checkbox.

**Option B — Set as default shell system-wide**
```bash
sudo cp target/release/psh /usr/local/bin/psh
echo "/usr/local/bin/psh" | sudo tee -a /etc/shells
chsh -s /usr/local/bin/psh
```

Works with any terminal: WezTerm, Kitty, Alacritty, Hyper — all have a one-line shell config.

---

## FAQs

**How is PSH different from Warp?**
Warp is a standalone terminal app that requires an account and sends your data to their cloud. PSH runs inside whatever terminal you already use, runs entirely on your machine, and requires nothing from the internet. Warp replaced your terminal. PSH improves the one you have.

**How is PSH different from Claude Code?**
Claude Code is an agent — it drives autonomously, running sequences of commands on your behalf. PSH keeps you in the driver seat. It translates your intent and explains your errors, but never executes anything you haven't confirmed.

**How is PSH different from fish or zsh?**
Fish and zsh are full shell implementations — they replace bash's parser, scripting engine, and builtins. PSH wraps bash and keeps all of that intact. The difference is that PSH adds AI as a first-class interface rather than building a better version of the same command syntax.

**Why now?**
Every generation of computing raised the level of abstraction. Assembly was the machine's language. C was high-level for its era. Bash was readable enough to call scripting. Each step moved the interface closer to how humans think. The prompt is the next step — plain language as the interface to computation. PSH applies that progression to the shell, the last major interface that still demands you speak the machine's language instead of your own.

---

> **One line:** PSH is bash, but you can talk to it like a person.
