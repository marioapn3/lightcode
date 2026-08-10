use crate::permissions::Policy;
use anyhow::{bail, Context};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub provider: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub permissions: Policy,
    /// Named agents (mode) with optional model override and system prompt.
    #[serde(default)]
    pub agents: HashMap<String, AgentDef>,
    /// Optional dedicated evaluator for goal completion checks.
    #[serde(default)]
    pub evaluator: EvaluatorConfig,
}

/// A named agent configuration from `[agents.<name>]`.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct AgentDef {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default, alias = "systemPrompt", alias = "prompt")]
    pub system_prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    /// USD per million tokens for the session cost estimate.
    #[serde(default = "default_input_price_per_m")]
    pub input_price_per_m: f64,
    #[serde(default = "default_output_price_per_m")]
    pub output_price_per_m: f64,
    /// Default turn limit for `/goal` when the goal does not specify one.
    #[serde(default = "default_max_goal_turns")]
    pub max_goal_turns: u32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            max_context_tokens: default_max_context_tokens(),
            max_iterations: default_max_iterations(),
            input_price_per_m: default_input_price_per_m(),
            output_price_per_m: default_output_price_per_m(),
            max_goal_turns: default_max_goal_turns(),
        }
    }
}

/// Optional dedicated evaluator model for goal completion checks.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct EvaluatorConfig {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ModelConfig {
    #[serde(default)]
    pub name: Option<String>,
}

/// opencode-style nested `options` block (baseURL / apiKey).
#[derive(Debug, Deserialize, Clone, Default)]
pub struct ProviderOptions {
    #[serde(default, alias = "baseURL")]
    pub base_url: Option<String>,
    #[serde(default, alias = "apiKey")]
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ProviderConfig {
    #[serde(default)]
    pub model: String,
    #[serde(default, alias = "apiKey")]
    pub api_key: Option<String>,
    #[serde(default, alias = "baseURL")]
    pub base_url: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Available models: key = model id, value = optional metadata.
    #[serde(default)]
    pub models: HashMap<String, ModelConfig>,
    /// opencode-style `options` block, merged as a fallback for base_url/api_key.
    #[serde(default)]
    pub options: Option<ProviderOptions>,
}

impl ProviderConfig {
    pub fn resolved_base_url(&self) -> Option<String> {
        self.base_url
            .clone()
            .or_else(|| self.options.as_ref().and_then(|o| o.base_url.clone()))
    }

    pub fn resolved_api_key(&self) -> Option<String> {
        self.api_key
            .clone()
            .or_else(|| self.options.as_ref().and_then(|o| o.api_key.clone()))
    }
}

fn default_provider() -> String {
    "openai".to_string()
}

fn default_max_context_tokens() -> usize {
    60_000
}

fn default_max_iterations() -> usize {
    crate::agent::MAX_ITERATIONS
}

fn default_input_price_per_m() -> f64 {
    0.30
}

fn default_output_price_per_m() -> f64 {
    1.20
}

fn default_max_goal_turns() -> u32 {
    10
}

impl Config {
    /// Load config from (in order): --config flag, LIGHTCODE_CONFIG env,
    /// a project `lightcode.json`/`lightcode.toml`, then `~/.config/lightcode/config.{toml,json}`.
    /// Missing optional files fall back to defaults; an explicitly requested path must exist.
    pub fn load(cli_config: Option<&Path>) -> anyhow::Result<Config> {
        if let Some(p) = cli_config {
            return load_from(p);
        }
        if let Some(p) = std::env::var_os("LIGHTCODE_CONFIG").map(PathBuf::from) {
            if p.is_file() {
                return load_from(&p);
            }
            bail!("LIGHTCODE_CONFIG points to missing file: {}", p.display());
        }
        if let Some(p) = project_config_path() {
            return load_from(&p);
        }
        if let Some(p) = home_config_path() {
            if p.is_file() {
                return load_from(&p);
            }
        }
        Ok(Config::default())
    }
}

/// Look for `lightcode.json` / `lightcode.toml` in the working directory.
fn project_config_path() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    for name in ["lightcode.json", "lightcode.toml"] {
        let p = cwd.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn load_from(path: &Path) -> anyhow::Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading config {}", path.display()))?;
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => serde_json::from_str(&text)
            .with_context(|| format!("parsing config {}", path.display())),
        Some("toml") => {
            toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))
        }
        _ => {
            if let Ok(cfg) = serde_json::from_str::<Config>(&text) {
                return Ok(cfg);
            }
            toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))
        }
    }
}

/// Platform-appropriate config directory. On macOS: `~/Library/Application Support/lightcode`.
fn platform_config_dir() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    let home = PathBuf::from(home);
    if cfg!(target_os = "macos") {
        home.join("Library")
            .join("Application Support")
            .join("lightcode")
    } else {
        home.join(".config").join("lightcode")
    }
}

/// Legacy location some setups may still have: `~/.config/lightcode`.
fn legacy_config_dir() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".config").join("lightcode")
}

fn home_config_path() -> Option<PathBuf> {
    for dir in [platform_config_dir(), legacy_config_dir()] {
        for name in ["config.toml", "config.json"] {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    // Default to the platform dir even if it does not exist yet.
    Some(platform_config_dir().join("config.toml"))
}

/// Default path `lightcode init` writes to: `~/.config/lightcode/config.toml`
/// (macOS: `~/Library/Application Support/lightcode/config.toml`).
pub fn default_config_path() -> PathBuf {
    platform_config_dir().join("config.toml")
}

/// Resolve an API key for a provider. An explicitly set config value wins (empty = no auth
/// needed); otherwise fall back to `{PROVIDER}_API_KEY` env var.
pub fn api_key_for(provider: &str, cfg: &ProviderConfig) -> Option<String> {
    if let Some(k) = cfg.resolved_api_key() {
        return Some(k);
    }
    let env = format!("{}_API_KEY", provider.to_uppercase().replace('-', "_"));
    std::env::var(env).ok()
}

/// Build the human-readable env var name for a provider, for error messages.
pub fn api_key_env(provider: &str) -> String {
    format!("{}_API_KEY", provider.to_uppercase().replace('-', "_"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config() {
        let text = r#"
[agent]
provider = "openai"

[provider.openai]
model = "gpt-4o-mini"
api_key = "sk-test"

[permissions]
shell = "allow"
"#;
        let cfg: Config = toml::from_str(text).unwrap();
        assert_eq!(cfg.agent.provider, "openai");
        assert_eq!(cfg.provider["openai"].model, "gpt-4o-mini");
        assert_eq!(cfg.provider["openai"].api_key.as_deref(), Some("sk-test"));
        assert_eq!(cfg.permissions.shell, crate::permissions::Level::Allow);
    }

    #[test]
    fn defaults_to_openai() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.agent.provider, "openai");
    }

    #[test]
    fn parses_json_config() {
        let text = r#"{
            "agent": { "provider": "opencode-go" },
            "provider": {
                "opencode-go": { "model": "deepseek-v4-flash", "api_key": "zen-test" }
            },
            "permissions": { "shell": "allow" }
        }"#;
        let cfg: Config = serde_json::from_str(text).unwrap();
        assert_eq!(cfg.agent.provider, "opencode-go");
        assert_eq!(cfg.provider["opencode-go"].model, "deepseek-v4-flash");
        assert_eq!(
            cfg.provider["opencode-go"].api_key.as_deref(),
            Some("zen-test")
        );
        assert_eq!(cfg.permissions.shell, crate::permissions::Level::Allow);
    }

    #[test]
    fn parses_opencode_style_provider() {
        let text = r#"{
            "agent": { "provider": "lokal-kantor" },
            "provider": {
                "lokal-kantor": {
                    "name": "LOKAL-KANTOR",
                    "options": { "baseURL": "http://100.118.237.83:20128/v1", "apiKey": "" },
                    "models": {
                        "codex/gpt-5.6-sol": { "name": "GPT 5.6 Sol" },
                        "codex/gpt-5.5": {}
                    }
                }
            }
        }"#;
        let cfg: Config = serde_json::from_str(text).unwrap();
        let p = &cfg.provider["lokal-kantor"];
        assert_eq!(
            p.resolved_base_url().as_deref(),
            Some("http://100.118.237.83:20128/v1")
        );
        assert_eq!(p.resolved_api_key().as_deref(), Some("")); // empty → env fallback
        assert!(p.models.contains_key("codex/gpt-5.6-sol"));
        assert_eq!(
            p.models["codex/gpt-5.6-sol"].name.as_deref(),
            Some("GPT 5.6 Sol")
        );
        assert_eq!(p.models["codex/gpt-5.5"].name, None);
    }

    #[test]
    fn empty_api_key_means_no_auth() {
        let pcfg = ProviderConfig {
            model: "m".into(),
            api_key: Some("".into()),
            base_url: None,
            max_tokens: None,
            models: Default::default(),
            options: None,
        };
        assert_eq!(api_key_for("opencode-go", &pcfg).as_deref(), Some(""));
    }

    #[test]
    fn api_key_env_name() {
        assert_eq!(api_key_env("openrouter"), "OPENROUTER_API_KEY");
    }

    #[test]
    fn parses_agents() {
        let text = r#"{
            "agents": {
                "coder": { "model": "gpt-5", "system_prompt": "You are a careful coder." },
                "reviewer": { "systemPrompt": "Review diffs strictly." }
            }
        }"#;
        let cfg: Config = serde_json::from_str(text).unwrap();
        assert_eq!(cfg.agents["coder"].model.as_deref(), Some("gpt-5"));
        assert_eq!(
            cfg.agents["coder"].system_prompt.as_deref(),
            Some("You are a careful coder.")
        );
        assert_eq!(
            cfg.agents["reviewer"].system_prompt.as_deref(),
            Some("Review diffs strictly.")
        );
        assert_eq!(cfg.agents["reviewer"].model, None);
    }
}
