use serde::Deserialize;
use std::fs;
use dirs::home_dir;

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    pub underlying_shell: String,
    pub ollama_url: String,
    pub model: String,
    pub confirm_commands: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            underlying_shell: std::env::var("PSH_SHELL")
                .unwrap_or_else(|_| "/bin/bash".to_string()),
            ollama_url: "http://127.0.0.1:11434".to_string(),
            model: "gemma3:4b".to_string(),
            confirm_commands: true,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let dir = home_dir().unwrap_or_default().join(".psh");
        fs::create_dir_all(&dir).ok();
        let path = dir.join("config.toml");

        if let Ok(contents) = fs::read_to_string(&path) {
            toml::from_str(&contents).unwrap_or_default()
        } else {
            let cfg = Self::default();
            let toml = format!(
                "underlying_shell = \"{}\"\nollama_url = \"{}\"\nmodel = \"{}\"\nconfirm_commands = {}\n",
                cfg.underlying_shell, cfg.ollama_url, cfg.model, cfg.confirm_commands
            );
            fs::write(&path, toml).ok();
            cfg
        }
    }
}
