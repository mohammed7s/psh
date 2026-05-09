use crate::config::Config;
use crate::context::{Db, Entry};
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

    // Files in current directory
    let files = std::process::Command::new("ls")
        .arg("-1a")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    // Git context if inside a repo
    let git_ctx = std::process::Command::new("git")
        .args(["status", "--short", "--branch"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    let history = recent.iter().rev().map(|e| {
        format!("  [{}] $ {}\n  exit:{} output:{}", e.cwd, e.command, e.exit_code,
            e.output.lines().next().unwrap_or(""))
    }).collect::<Vec<_>>().join("\n");

    format!(
        "You are PSH, an AI shell assistant embedded in the terminal.\n\
         OS: {os}\n\
         Shell: {shell}\n\
         Current directory: {cwd}\n\
         Files in current directory:\n{files}\n\
         {git}\
         Recent session history:\n{history}\n\n\
         Rules:\n\
         - Return ONLY the exact command to run, nothing else\n\
         - No markdown, no backticks, no explanation\n\
         - Use correct syntax for {shell} on {os}\n\
         - Use exact filenames from the file listing above when relevant\n\
         - If the request is ambiguous or dangerous, prefix with WARN:",
        git = if git_ctx.is_empty() { String::new() }
              else { format!("Git status:\n{git_ctx}\n") }
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

pub fn translate_nl(config: &Config, db: &Db, session: &str, input: &str) -> Option<String> {
    let recent = db.recent(session, 10);
    let system = build_system_prompt(config, &recent);
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
