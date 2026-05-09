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

    let cwd = home.clone();

    let files: String = std::process::Command::new("ls")
        .arg("-1")
        .arg(&home)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().take(20).collect::<Vec<_>>().join("  "))
        .unwrap_or_default();

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

    let machine_ctx = load_machine_context();
    let machine_section = if machine_ctx.is_empty() {
        String::new()
    } else {
        format!("Machine:\n{}\n", machine_ctx)
    };

    let history = if recent.is_empty() {
        String::new()
    } else {
        let lines: String = recent.iter()
            .map(|e| match &e.nl_prompt {
                Some(p) => format!("  \"{}\" → {}", p, e.command),
                None    => format!("  $ {}", e.command),
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("Recent history:\n{lines}\n")
    };

    format!(
        "You are PSH, a terminal AI assistant. You help users by running commands and reasoning about their output.\n\
         OS: {os}  Shell: {shell}  CWD: {cwd}\n\
         Home dir files: {files}\n\
         {git_line}\
         {machine_section}\
         {history}\n\
         You have four response types:\n\
         EXEC: <command>   — run a read-only command to gather information, then reason about its output\n\
         CMD: <command>    — a command for the user to run (shown on screen, user confirms)\n\
         ANSWER: <text>    — a direct answer to the user's question\n\
         WARN: <reason>    — if the request is dangerous or impossible\n\
         \n\
         Rules:\n\
         - Use EXEC to look up information before answering (check versions, list files, read configs, etc.)\n\
         - EXEC must be READ-ONLY — no writes, deletes, or installs\n\
         - After EXEC output arrives, reason about it and respond with the next EXEC, CMD, or ANSWER\n\
         - CMD must be valid shell syntax — never put natural language in CMD\n\
         - Use && in CMD to chain steps: CMD: mkdir foo && cd foo && git init\n\
         - ANSWER for questions; CMD for actions\n\
         \n\
         Examples:\n\
         'how to update rust' → EXEC: rustc --version && rustup show\n\
         'list python files'  → CMD: find . -name '*.py'\n\
         'capital of france'  → ANSWER: Paris\n\
         'install node'       → CMD: curl -fsSL https://fnm.vercel.app/install | bash\n\
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
