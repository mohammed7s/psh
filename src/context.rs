use dirs::home_dir;
use std::fs;
use std::path::PathBuf;

// ── History entry ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct HistoryEntry {
    pub cwd:       String,
    pub nl_prompt: Option<String>, // natural language input, if this came from NL mode
    pub command:   String,
    pub exit_code: i32,
}

// ── File-based rolling history ────────────────────────────────────────────────

pub struct History {
    path: PathBuf,
}

const MAX_ENTRIES: usize = 50;

impl History {
    pub fn open() -> Self {
        let dir = home_dir().unwrap_or_default().join(".psh");
        fs::create_dir_all(&dir).ok();
        Self { path: dir.join("history.md") }
    }

    pub fn append(&self, entry: &HistoryEntry) {
        let line = encode(entry);
        let existing = fs::read_to_string(&self.path).unwrap_or_default();
        let mut lines: Vec<&str> = existing.lines().collect();
        lines.push(Box::leak(line.into_boxed_str()));

        // Keep rolling window
        let start = lines.len().saturating_sub(MAX_ENTRIES);
        let trimmed = lines[start..].join("\n") + "\n";
        fs::write(&self.path, trimmed).ok();
    }

    /// Last `limit` entries, oldest first.
    pub fn recent(&self, limit: usize) -> Vec<HistoryEntry> {
        let raw = fs::read_to_string(&self.path).unwrap_or_default();
        raw.lines()
            .rev()
            .take(limit)
            .filter_map(decode)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}

// Tab-separated: cwd \t nl_prompt_or_empty \t command \t exit_code
fn encode(e: &HistoryEntry) -> String {
    format!(
        "{}\t{}\t{}\t{}",
        e.cwd.replace('\t', " "),
        e.nl_prompt.as_deref().unwrap_or("").replace('\t', " "),
        e.command.replace('\t', " "),
        e.exit_code,
    )
}

fn decode(line: &str) -> Option<HistoryEntry> {
    let parts: Vec<&str> = line.splitn(4, '\t').collect();
    if parts.len() != 4 { return None; }
    Some(HistoryEntry {
        cwd:       parts[0].to_string(),
        nl_prompt: if parts[1].is_empty() { None } else { Some(parts[1].to_string()) },
        command:   parts[2].to_string(),
        exit_code: parts[3].parse().unwrap_or(0),
    })
}

// ── Machine context (generated once, stored permanently) ─────────────────────

pub fn refresh_machine_context() {
    let dir = home_dir().unwrap_or_default().join(".psh");
    fs::create_dir_all(&dir).ok();
    let path = dir.join("machine_context.txt");
    if path.exists() { return; } // already generated
    fs::write(&path, build_machine_context()).ok();
}

pub fn load_machine_context() -> String {
    let path = home_dir().unwrap_or_default().join(".psh/machine_context.txt");
    fs::read_to_string(path).unwrap_or_default()
}

fn build_machine_context() -> String {
    let probes: &[(&str, &str)] = &[
        ("OS",      "uname -srm"),
        ("Distro",  "grep PRETTY_NAME /etc/os-release 2>/dev/null | cut -d'\"' -f2"),
        ("User",    "whoami"),
        ("Host",    "hostname"),
        ("git",     "git --version"),
        ("node",    "node --version 2>/dev/null"),
        ("npm",     "npm --version 2>/dev/null"),
        ("yarn",    "yarn --version 2>/dev/null"),
        ("python3", "python3 --version 2>/dev/null"),
        ("pip3",    "pip3 --version 2>/dev/null | cut -d' ' -f1-2"),
        ("cargo",   "cargo --version 2>/dev/null"),
        ("go",      "go version 2>/dev/null"),
        ("docker",  "docker --version 2>/dev/null"),
        ("make",    "make --version 2>/dev/null | head -1"),
        ("Memory",  "free -h 2>/dev/null | grep Mem | awk '{print $2\" total, \"$7\" available\"}'"),
        ("Disk(~)", "df -h ~ 2>/dev/null | tail -1 | awk '{print $4\" free of \"$2}'"),
    ];

    probes.iter()
        .filter_map(|(label, cmd)| {
            let out = sh(cmd);
            let first = out.trim().lines().next().unwrap_or("").trim();
            if first.is_empty() { None } else { Some(format!("{}: {}", label, first)) }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn sh(cmd: &str) -> String {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}
