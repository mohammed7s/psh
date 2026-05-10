# PSH — Prompt Shell

A shell wrapper that sits between your terminal and bash. Type commands normally, or describe what you want in plain English and let the AI figure out the command. Everything runs locally — no account, no cloud, no API key.

```
ubuntu@machine:~$ find all python files modified today
  ⠹  thinking
  ❯  find . -name "*.py" -mtime -1
./src/main.py
./tests/test_core.py

ubuntu@machine:~$ compress the logs folder into a tar.gz
  ❯  tar -czf logs.tar.gz logs  [y/n] y

ubuntu@machine:~$ what is using the most memory
  ps aux --sort=-%mem | head -10
```

## Requirements

- Rust — https://rustup.rs
- Ollama — https://ollama.com/install

## Install

```bash
# 1. Install Ollama
curl -fsSL https://ollama.com/install.sh | sh

# 2. Pull a model
ollama pull gemma3:4b

# 3. Build
cargo build --release
sudo cp target/release/psh /usr/local/bin/psh
```

**Set as your default shell:**
```bash
echo /usr/local/bin/psh | sudo tee -a /etc/shells
chsh -s /usr/local/bin/psh
```

Or keep it opt-in — just run `psh` whenever you want it.

## Try it

```bash
cargo build
./target/debug/psh
```

## Usage

Type commands exactly as you normally would. To send something to the AI instead of bash, press **Alt+Enter** after typing.

```bash
# Normal bash — just press Enter as usual
ls -la
git status
cd ~/projects

# Natural language — press Ctrl+J or Alt+Enter to submit
find all python files modified today
what branch am i on
compress this folder into a tar.gz
show disk usage sorted by size
how much memory is postgres using
```

### Controls

| Key | Action |
|-----|--------|
| **Alt+Enter** | Send what you typed to the AI |
| **Up / Down arrow** | Browse history (bash commands and NL prompts, unified) |
| **Backspace** | Edit as normal |
| **ESC** or **Ctrl+C** | Cancel NL mode / cancel AI query |

### After the AI responds

| Key | Action |
|-----|--------|
| `y` or Enter | Run the suggested command |
| `n`, ESC, or Ctrl+C | Cancel |

**Safe read-only commands** (`ls`, `find`, `grep`, `cat`, `git status`, etc.) run immediately without a confirmation prompt.

**Multi-step commands** are shown as a chain:
```
  ❯  mkdir my-project
     &&  cd my-project
     &&  git init  [y/n]
```

## Config

`~/.psh/config.toml` is created automatically on first launch.

```toml
underlying_shell = "/bin/bash"
ollama_url = "http://127.0.0.1:11434"
model = "gemma3:4b"
confirm_commands = true   # false = auto-run everything without asking
```

## Storage

All storage lives in `~/.psh/` — nothing leaves your machine.

| File | Contents |
|------|----------|
| `config.toml` | Your config |
| `history.md` | Rolling last 50 entries — bash commands and NL prompts, unified |
| `machine_context.txt` | Snapshot of your OS, shell, and installed tools — generated once on first launch |

## How it works

PSH wraps bash inside a PTY. Every keystroke passes through PSH first. If you press Enter, the input goes to bash as normal. If you press **Alt+Enter**, the input goes to the AI instead.

The AI receives as context:
- Your OS, shell, current directory, and git branch
- Your machine's installed tools (from `machine_context.txt`)
- The last 5 entries from your history

The AI can run read-only commands (`ls`, `df`, `ps`, etc.) to inspect your system before answering — so queries like *"what is eating my disk"* or *"which node version do I have"* work without you having to run anything first.

**Up arrow** recalls your full history in chronological order — both bash commands you ran and natural language queries you made — so you can re-run or refine anything without switching modes.

## Model notes

`gemma3:4b` (3.3 GB) is the recommended default — small enough to run on CPU, good enough for most shell tasks. For harder queries:

```bash
ollama pull llama3.1:8b
```

Then in `~/.psh/config.toml`:
```toml
model = "llama3.1:8b"
```
