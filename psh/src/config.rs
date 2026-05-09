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
            ollama_url: "http://localhost:11434".to_string(),
            model: "gemma3:4b".to_string(),
            confirm_commands: true,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let path = home_dir()
            .unwrap_or_default()
            .join(".psh")
            .join("config.toml");

        if let Ok(contents) = fs::read_to_string(&path) {
            toml::from_str(&contents).unwrap_or_default()
        } else {
            Self::default()
        }
    }
}
