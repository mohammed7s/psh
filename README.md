# PSH — Prompt Shell

A shell wrapper that sits between your terminal and bash. Type commands normally or describe what you want in plain English. Everything runs locally — no account, no cloud, no API key.

```
ubuntu@machine:~$ > find all python files modified today
psh: thinking...
psh: find . -name "*.py" -newer $(date +%Y-%m-%d) [y/n] y
./src/main.py
./tests/test_core.py
```

## Requirements

- Rust (https://rustup.rs)
- Ollama (https://ollama.com/install)
- gemma3:4b model

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

# Natural language — prefix with >
> list all files modified today
> show me what branch im on
> find the config file for nginx
> compress this folder into a tar.gz

# PSH shows the command and asks to confirm
psh: find . -mtime -1 [y/n]
```

## Config

PSH reads `~/.psh/config.toml` on startup. Created automatically with defaults if missing.

```toml
underlying_shell = "/bin/bash"   # shell to wrap
ollama_url = "http://localhost:11434"
model = "gemma3:4b"
confirm_commands = true          # set false to auto-run without confirming
```

## Data

All command history stored locally at `~/.psh/history.db` (SQLite).
Nothing is sent anywhere except your local Ollama instance.
