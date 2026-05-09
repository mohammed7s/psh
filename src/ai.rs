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

    // PSH's own process CWD is wherever it was launched from, not the bash
    // shell's CWD inside the PTY. HOME is the reliable anchor.
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
        "You are PSH, a terminal AI that translates natural language into shell commands.\n\
         OS: {os}  Shell: {shell}  CWD: {cwd}\n\
         Files: {files}\n\
         {git_line}\
         {machine_section}\
         {history}\n\
         Respond with EXACTLY one of:\n\
         CMD: <shell command>   — a real, executable shell command\n\
         ANSWER: <text>         — for questions that need no command\n\
         WARN: <reason>         — only if the request is dangerous or impossible\n\
         \n\
         Rules for CMD:\n\
         - Must be valid shell syntax — NEVER put natural language in CMD\n\
         - Use && to chain multiple steps: CMD: mkdir foo && cd foo && git init\n\
         \n\
         Examples:\n\
         list files → CMD: ls -la\n\
         find python files → CMD: find . -name '*.py'\n\
         find file called foo in home → CMD: find ~ -maxdepth 1 -name 'foo'\n\
         what branch → CMD: git branch --show-current\n\
         capital of france → ANSWER: Paris\n\
         delete everything → WARN: This permanently deletes files\n\
         \n\
         One line. No markdown. No explanation."
    )
}

pub fn translate_nl(config: &Config, recent: &[HistoryEntry], input: &str) -> Option<String> {
    let system = build_system_prompt(config, recent);
    call_ollama(config, &system, input)
}

fn call_ollama(config: &Config, system: &str, user_msg: &str) -> Option<String> {
    let url = format!("{}/api/chat", config.ollama_url);
    let body = json!({
        "model": config.model,
        "stream": false,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user",   "content": user_msg }
        ]
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
