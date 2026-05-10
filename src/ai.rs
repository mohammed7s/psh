use crate::config::Config;
use crate::context::{HistoryEntry, load_machine_context};
use serde_json::{json, Value};

fn build_system_prompt(config: &Config, recent: &[HistoryEntry]) -> String {
    let shell = std::path::Path::new(&config.underlying_shell)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();

    let os = std::env::consts::OS;

    let home = std::env::var("HOME").unwrap_or_else(|_| "/home".to_string());

    let git_branch = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let git_line = if git_branch.is_empty() {
        String::new()
    } else {
        format!("Git branch: {git_branch}\n")
    };

    // Identity only: OS/distro/user/shell. Versions and disk are fetched via EXEC.
    let machine_ctx = load_machine_context();
    let machine_section = if machine_ctx.is_empty() {
        String::new()
    } else {
        let trimmed = machine_ctx.lines().take(6).collect::<Vec<_>>().join("; ");
        format!("Machine: {trimmed}\n")
    };

    let history = if recent.is_empty() {
        String::new()
    } else {
        let lines: String = recent.iter().take(5)
            .map(|e| match &e.nl_prompt {
                Some(p) => format!("  \"{}\" → {}", p, e.command),
                None    => format!("  $ {}", e.command),
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("Recent history:\n{lines}\n")
    };

    format!(
        "You are PSH, a terminal AI assistant. You help users by running shell commands and answering questions about their system.\n\
         OS: {os}  Shell: {shell}  Home: {home}\n\
         {git_line}\
         {machine_section}\
         {history}\n\
         RESPONSE TYPES (pick exactly one per reply, one line only):\n\
         EXEC: <command>   — run a read-only shell command to gather info, then reason about its output\n\
         CMD: <command>    — suggest a command for the user to run\n\
         ANSWER: <text>    — answer a question directly\n\
         WARN: <reason>    — only if the request is truly dangerous or impossible\n\
         \n\
         RULES:\n\
         - ALWAYS use EXEC first for anything about files, folders, disk, memory, processes, versions, or system state\n\
         - EXEC must be read-only (ls, find, cat, df, ps, free, git status, etc.) — never write, delete, or install\n\
         - After EXEC output, respond with another EXEC, or CMD, or ANSWER\n\
         - CMD must be valid shell — never put natural language inside CMD\n\
         - Never use WARN unless the task is genuinely impossible or destructive\n\
         - Natural language questions (what is X, who is Y, explain Z) always use ANSWER — never WARN\n\
         \n\
         EXAMPLES:\n\
         'what is in my downloads'       → EXEC: ls -lt ~/Downloads | head -20\n\
         'last file added to downloads'  → EXEC: ls -lt ~/Downloads | head -5\n\
         'how much disk space left'      → EXEC: df -h ~\n\
         'what is using most memory'     → EXEC: ps aux --sort=-%mem | head -10\n\
         'how to update rust'            → EXEC: rustc --version && rustup show\n\
         'list python files here'        → CMD: find . -name '*.py'\n\
         'capital of france'             → ANSWER: Paris\n\
         'install node'                  → CMD: curl -fsSL https://fnm.vercel.app/install | bash\n\
         \n\
         One line per response. No markdown. No explanation outside the format."
    )
}

/// Translate natural language to a shell action, with an agentic EXEC loop
/// so the AI can gather information before answering.
pub fn translate_nl(config: &Config, recent: &[HistoryEntry], input: &str) -> Option<String> {
    let system = build_system_prompt(config, recent);

    let mut messages: Vec<Value> = vec![
        json!({"role": "system", "content": system}),
        json!({"role": "user",   "content": input}),
    ];

    for _ in 0..6 {
        let resp = call_ollama_msgs(config, &messages)?;

        if let Some(cmd) = resp.strip_prefix("EXEC:") {
            let cmd = cmd.trim();
            let output = capture(cmd);
            messages.push(json!({"role": "assistant", "content": resp}));
            messages.push(json!({"role": "user", "content": format!("Command output:\n{}", output)}));
        } else {
            return Some(resp);
        }
    }

    None
}

/// Run a command and capture its combined stdout+stderr (for EXEC loop).
fn capture(cmd: &str) -> String {
    match std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
    {
        Ok(o) => {
            let mut out = String::new();
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            if !stdout.trim().is_empty() { out.push_str(stdout.trim()); }
            if !stderr.trim().is_empty() {
                if !out.is_empty() { out.push('\n'); }
                out.push_str(stderr.trim());
            }
            if out.is_empty() { "(no output)".to_string() } else { out }
        }
        Err(e) => format!("error: {}", e),
    }
}

fn call_ollama_msgs(config: &Config, messages: &[Value]) -> Option<String> {
    let url = format!("{}/api/chat", config.ollama_url);
    let body = json!({
        "model": config.model,
        "stream": false,
        "keep_alive": "30m",
        "messages": messages
    });

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .ok()?;

    let resp = match client.post(&url).json(&body).send() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("\r\npsh error: {}\r\n", e);
            return None;
        }
    };

    let json: Value = resp.json().ok()?;
    let text = json["message"]["content"].as_str()?.trim().to_string();

    if text.is_empty() { None } else { Some(text) }
}
