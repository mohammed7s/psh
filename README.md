# PSH — Prompt Shell

A shell wrapper that sits between your terminal and bash. Type commands normally or describe what you want in plain English. Everything runs locally — no account, no cloud, no API key.

```
ubuntu@machine:~$  find all python files modified today
  ⠹  thinking
  ❯  find . -name "*.py" -mtime -1
./src/main.py
./tests/test_core.py

ubuntu@machine:~$  compress the logs folder into a tar.gz
  ❯  tar -czf logs.tar.gz logs  [y/n] y
```

## Requirements

- Rust (https://rustup.rs)
- Ollama (https://ollama.com/install) with a local model

## Install

```bash
# 1. Install Ollama
curl -fsSL https://ollama.com/install.sh | sh

# 2. Pull a model (gemma3:4b is small and fast; llama3.1:8b is more accurate)
ollama pull gemma3:4b

# 3. Build and install
cargo build --release
sudo cp target/release/psh /usr/local/bin/psh
```

## Run

**Try it without installing:**
```bash
cargo build
./target/debug/psh
```

**Set as default in GNOME Terminal:**
```
Preferences → Profile → Command
→ Run a custom command instead of my shell
→ /usr/local/bin/psh
```

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
 create a new git repo in this folder and make an initial commit
```

**Controls while in NL mode:**
| Key | Action |
|-----|--------|
| Enter | Send query to AI |
| ESC or Ctrl+C | Cancel |
| Backspace | Delete last character |

**Controls after AI responds:**
| Key | Action |
|-----|--------|
| `y` or Enter | Run the suggested command |
| `n`, ESC, or Ctrl+C | Cancel |

**Safe read-only commands** (`ls`, `find`, `grep`, `cat`, `git status`, etc.) run immediately without asking — no `[y/n]` prompt.

**Multi-step tasks** are shown as a chain and confirmed once:
```
  ❯  mkdir my-project
     &&  cd my-project
     &&  git init  [y/n]
```

## Config

`~/.psh/config.toml` — created automatically on first launch.

```toml
underlying_shell = "/bin/bash"
ollama_url = "http://127.0.0.1:11434"
model = "gemma3:4b"
confirm_commands = true   # false = auto-run all commands without asking
```

## Storage

All storage lives in `~/.psh/`:

| File | Contents |
|------|----------|
| `machine_context.txt` | Snapshot of your OS, installed tools, memory, disk — generated once on first launch and injected into every AI prompt |
| `history.md` | Rolling last 50 entries (commands + NL prompts) — plain text, injected as context for follow-up queries |
| `config.toml` | Your config |

Nothing is sent anywhere except your local Ollama instance.

## How it works

PSH intercepts a **leading space** at the prompt as the signal to enter NL mode. Everything else passes straight through to bash unchanged.

The AI receives as context:
- Your OS, shell, current directory, and files
- Your machine's installed tools (from `machine_context.txt`)
- Your git branch (if in a repo)
- The last 10 entries from `history.md`, including the original NL prompts

This means follow-up queries like `"compress it"`, `"run that again"`, or `"do the same for the other folder"` work as expected.

## Model quality

`gemma3:4b` is fast but occasionally returns incorrect commands for complex queries. For better accuracy:

```bash
ollama pull llama3.1:8b
```

Then update `~/.psh/config.toml`:
```toml
model = "llama3.1:8b"
```
