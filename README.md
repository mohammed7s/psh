# PSH — Prompt Shell

A shell wrapper that sits between your terminal and bash. Type commands normally or describe what you want in plain English. Everything runs locally — no account, no cloud, no API key.

```
ubuntu@machine:~$  find all python files modified today
  ⠹  thinking
  ❯  find . -name "*.py" -mtime -1  [y/n] y
./src/main.py
./tests/test_core.py
```

## Requirements

- Rust (https://rustup.rs)
- Ollama (https://ollama.com/install)
- A local model — default is `gemma3:4b`

## Install

```bash
# 1. Install Ollama
curl -fsSL https://ollama.com/install.sh | sh

# 2. Pull the model
ollama pull gemma3:4b

# 3. Build PSH
cargo build --release

# 4. Install the binary
sudo cp target/release/psh /usr/local/bin/psh
```

## Run

**Option A — Just try it (no system changes):**
```bash
cargo build
./target/debug/psh
```

**Option B — Set as default in GNOME Terminal:**
```
Preferences → Profile → Command
→ Run a custom command instead of my shell
→ /usr/local/bin/psh
```
Open a new tab — you're in PSH.

## Usage

```bash
# Normal commands work exactly as before
ls -la
git status
cd ~/projects

# Natural language — prefix with a leading space
 find all python files modified today
 what branch am i on
 compress this folder into a tar.gz
 show disk usage sorted by size

# PSH shows the generated command and asks to confirm
  ❯  find . -name "*.py" -mtime -1  [y/n]
```

**Controls:**
- `y` or Enter — run the suggested command
- `n` or ESC — cancel
- ESC while thinking — cancel the AI call immediately
- Ctrl+D — exit PSH

## Config

PSH reads `~/.psh/config.toml` on startup. Created automatically with defaults if missing.

```toml
underlying_shell = "/bin/bash"
ollama_url = "http://127.0.0.1:11434"
model = "gemma3:4b"
confirm_commands = true   # set false to auto-run without confirming
```

## How it works

PSH intercepts a **leading space** at the prompt as a signal to enter natural language mode. Everything else passes straight through to bash unchanged.

The AI has context of your current directory, recent files, git branch, and the last 10 commands from the session — so queries like "compress it" or "run the tests again" work as expected.

Three response types:
- `CMD:` — a shell command to run (shown with `[y/n]` confirm)
- `ANSWER:` — a direct text answer (for questions like "what does this flag do?")
- `WARN:` — a warning if the request is dangerous or not possible

## Data

All command history stored locally at `~/.psh/history.db` (SQLite).
Nothing is sent anywhere except your local Ollama instance.
