use crate::config::Config;
use crate::context::Entry;
use serde_json::{json, Value};

fn build_system_prompt(config: &Config, recent: &[Entry]) -> String {
    let shell = std::path::Path::new(&config.underlying_shell)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();

    let os = std::env::consts::OS;

    let cwd = std::env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let files: String = std::process::Command::new("ls")
        .arg("-1")
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

    let history = if recent.is_empty() {
        String::new()
    } else {
        let lines: String = recent.iter().rev()
            .map(|e| format!("  $ {}  (exit {})", e.command, e.exit_code))
            .collect::<Vec<_>>()
            .join("\n");
        format!("Recent commands:\n{lines}\n")
    };

    format!(
        "You are PSH, an AI assistant embedded in the terminal.\n\
         OS: {os}  Shell: {shell}  CWD: {cwd}\n\
         Files: {files}\n\
         {git_line}\
         {history}\n\
         Reply in exactly one of these formats:\n\
         CMD: <shell command>        — when the user wants to do something\n\
         ANSWER: <text>              — when the user asks a question\n\
         WARN: <reason>              — if the request is dangerous or impossible\n\
         No markdown. No explanation. One line."
    )
}

fn build_error_prompt(config: &Config, command: &str, output: &str, exit_code: i32) -> String {
    let shell = std::path::Path::new(&config.underlying_shell)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();

    format!(
        "You are PSH. A shell command failed.\n\
         Shell: {shell}\n\
         Command: {command}\n\
         Exit code: {exit_code}\n\
         Output:\n{output}\n\n\
         Explain what went wrong in 1-2 sentences and give the exact fix command.\n\
         Format: REASON: <reason> | FIX: <command>"
    )
}

pub fn translate_nl(config: &Config, recent: &[Entry], input: &str) -> Option<String> {
    let system = build_system_prompt(config, recent);
    call_ollama(config, &system, input)
}

pub fn explain_error(config: &Config, command: &str, output: &str, exit_code: i32) -> Option<String> {
    let system = build_error_prompt(config, command, output, exit_code);
    call_ollama(config, &system, "Explain this error and give the fix.")
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
