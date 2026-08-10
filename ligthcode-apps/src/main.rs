mod agent;
mod config;
mod diff;
mod files;
mod history;
mod log;
mod mentions;
mod permissions;
mod providers;
mod session;
mod tools;
mod tui;
mod web;
mod workspace;

use agent::Agent;
use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use permissions::Choice;
use providers::anthropic::AnthropicProvider;
use providers::openai::OpenAiProvider;
use providers::Provider;
use std::collections::HashMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use tokio::sync::mpsc;

#[derive(Parser, Clone)]
#[command(
    name = "lightcode",
    version,
    about = "A lightweight native Rust coding agent"
)]
struct Cli {
    /// Path to a config file
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,
    /// Provider name (e.g. openai)
    #[arg(short, long, global = true)]
    provider: Option<String>,
    /// Model name override
    #[arg(short, long, global = true)]
    model: Option<String>,
    /// Resume the most recent session in the TUI
    #[arg(long, global = true)]
    continue_session: bool,
    /// Resume an existing session by id in the TUI
    #[arg(short, long, global = true)]
    session: Option<String>,
    /// Auto-approve all permission prompts (dangerous; use with care)
    #[arg(long, global = true)]
    auto: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Clone)]
enum Command {
    /// Run a prompt non-interactively and exit
    Run(RunArgs),
    /// Manage saved sessions
    Session {
        #[command(subcommand)]
        cmd: SessionCmd,
    },
    /// Create or update the LightCode config file (interactive wizard)
    Init(InitArgs),
    /// List models for the configured provider
    Models,
    /// List configured providers
    Providers,
    /// Show config file location and resolved settings
    Config,
    /// Show session stats (message counts, estimated tokens)
    Stats {
        /// Session id; omit for the most recent session
        id: Option<String>,
    },
}

#[derive(Args, Clone)]
struct InitArgs {
    /// Provider name: openai, opencode-go, openrouter, anthropic, or a custom OpenAI-compatible name
    #[arg(long)]
    provider: Option<String>,
    /// Default model for the provider
    #[arg(long)]
    model: Option<String>,
    /// API key to store in the config file (omit to use the {PROVIDER}_API_KEY env var)
    #[arg(long)]
    key: Option<String>,
    /// Base URL for a custom OpenAI-compatible provider
    #[arg(long)]
    base_url: Option<String>,
    /// Config file to write instead of the default location
    #[arg(long)]
    path: Option<PathBuf>,
    /// Overwrite an existing config file without asking
    #[arg(long)]
    force: bool,
}

#[derive(Args, Clone)]
struct RunArgs {
    /// The prompt to run. If omitted, stdin is read.
    prompt: Option<String>,
    /// Output format: `default` or `json` (NDJSON event stream)
    #[arg(long, default_value = "default", value_parser = ["default", "json"])]
    format: String,
    /// Attach a file's contents to the prompt (repeatable, limit 10 MiB)
    #[arg(short, long)]
    file: Vec<PathBuf>,
    /// Run as the given named agent from config
    #[arg(long)]
    agent: Option<String>,
}

#[derive(Subcommand, Clone)]
enum SessionCmd {
    /// List sessions in the current workspace
    List {
        /// List sessions from every workspace and legacy
        #[arg(long)]
        all: bool,
    },
    /// Move a session from another workspace/unscoped into the current one
    /// (`adopt all` adopts every un-scoped session)
    Adopt { id: String },
    /// Show a session's message history
    Show { id: String },
    /// Delete a session
    Delete { id: String },
    /// Rename a session
    Rename { id: String, title: String },
    /// Fork a session into a new one
    Fork { id: String },
    /// Export a session as JSON to stdout
    Export { id: String },
    /// Import a session from a JSON file
    Import { file: PathBuf },
}

#[tokio::main]
async fn main() -> Result<()> {
    log::init();
    // Scope sessions to the current workspace (git root or normalized cwd).
    let cwd_path = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let workspace = workspace::resolve(&cwd_path);
    session::storage::set_workspace(&workspace);
    let cli = Cli::parse();
    match &cli.command {
        None => run_tui(&cli).await,
        Some(Command::Run(args)) => run_noninteractive(&cli, args.clone()).await,
        Some(Command::Session { cmd }) => match cmd {
            SessionCmd::List { all: false } => session::cmd_list(),
            SessionCmd::List { all: true } => session::cmd_list_all(),
            SessionCmd::Adopt { id } => {
                if id == "all" {
                    let n = session::storage::adopt_all()?;
                    println!("adopted {n} un-scoped session(s) into the current workspace");
                    Ok(())
                } else {
                    session::cmd_adopt(id)
                }
            }
            SessionCmd::Show { id } => session::cmd_show(id),
            SessionCmd::Delete { id } => session::cmd_delete(id),
            SessionCmd::Rename { id, title } => session::cmd_rename(id, title),
            SessionCmd::Fork { id } => session::cmd_fork(id),
            SessionCmd::Export { id } => session::cmd_export(id),
            SessionCmd::Import { file } => session::cmd_import(file),
        },
        Some(Command::Models) => list_models(&cli),
        Some(Command::Providers) => list_providers(&cli),
        Some(Command::Config) => show_config(&cli),
        Some(Command::Init(args)) => cmd_init(args.clone()),
        Some(Command::Stats { id }) => show_stats(id.as_deref()),
    }
}

/// Interactive wizard that scaffolds a LightCode config file (default:
/// `~/.config/lightcode/config.toml`). Prompts are skipped when stdin is not a
/// terminal or when `--provider` is given; pass flags for non-interactive use.
fn cmd_init(args: InitArgs) -> Result<()> {
    let path = match &args.path {
        Some(p) => p.clone(),
        None => config::default_config_path(),
    };
    // Any explicit flag selects scripted (non-interactive) mode; only a bare
    // `lightcode init` in a terminal runs the full wizard.
    let fully_scripted = args.provider.is_some()
        || args.model.is_some()
        || args.key.is_some()
        || args.base_url.is_some();
    let interactive = std::io::stdin().is_terminal() && !fully_scripted;

    if path.exists() && !args.force {
        if interactive {
            let ans = prompt_line(
                &format!(
                    "Config already exists at {} — overwrite? (y/N)",
                    path.display()
                ),
                "n",
            );
            if !matches!(ans.to_lowercase().as_str(), "y" | "yes") {
                println!("Aborted — existing config left untouched.");
                return Ok(());
            }
        } else {
            bail!(
                "config already exists at {} — pass --force to overwrite",
                path.display()
            );
        }
    }

    // Provider.
    let provider = match &args.provider {
        Some(p) if !p.trim().is_empty() => p.trim().to_string(),
        _ if interactive => prompt_line(
            "Provider (openai / opencode-go / openrouter / anthropic / custom)",
            "openai",
        ),
        _ => "openai".to_string(),
    }
    .trim()
    .to_lowercase();

    // Base URL is only required for custom (non-built-in) providers.
    let known = matches!(
        provider.as_str(),
        "openai" | "opencode-go" | "openrouter" | "anthropic"
    );
    let base_url = if args.base_url.is_some() {
        args.base_url.clone()
    } else if !known && interactive {
        let b = prompt_line(
            &format!("No default endpoint for '{provider}' — base_url (e.g. https://host/v1)"),
            "",
        );
        if b.is_empty() {
            None
        } else {
            Some(b)
        }
    } else if !known {
        eprintln!(
            "warning: provider '{provider}' has no default endpoint; \
             set --base-url now or add base_url to the config later"
        );
        None
    } else {
        None
    };

    // Model (providers fall back to their own default when omitted).
    let default_model = default_model(&provider);
    let model = match &args.model {
        Some(m) => m.trim().to_string(),
        _ if interactive => prompt_line("Default model", &default_model),
        _ => default_model,
    };

    // API key: empty means the {PROVIDER}_API_KEY env var is used at runtime.
    let key = match &args.key {
        Some(k) => k.trim().to_string(),
        _ if interactive => prompt_line("API key (Enter to skip — use env var)", ""),
        _ => String::new(),
    };

    if key.is_empty() {
        println!(
            "note: no API key stored — set {} to use this provider",
            config::api_key_env(&provider)
        );
    }

    let text = build_init_toml(&provider, &model, &key, base_url.as_deref());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;
    }
    std::fs::write(&path, text).with_context(|| format!("writing config {}", path.display()))?;

    // Fail loudly if the file we just wrote does not parse as a valid config.
    let _cfg = config::Config::load(Some(&path))
        .with_context(|| format!("validating config {}", path.display()))?;

    println!();
    println!("✓ Config written to {}", path.display());
    println!();
    println!("Next steps:");
    println!("  1. Run `lightcode` to start a session");
    println!("  2. `lightcode providers` — list configured providers");
    println!("  3. `lightcode config` — show resolved settings");
    println!(
        "  4. Edit {} anytime, or re-run `lightcode init`.",
        path.display()
    );
    Ok(())
}

/// Prompt for a line on stderr/stdin. Empty input returns `default`.
fn prompt_line(label: &str, default: &str) -> String {
    if default.is_empty() {
        eprint!("{label}: ");
    } else {
        eprint!("{label} [{default}]: ");
    }
    std::io::stderr().flush().ok();
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() || line.trim().is_empty() {
        return default.to_string();
    }
    line.trim().to_string()
}

/// Serialize a value as a TOML string literal (quoted + escaped).
fn toml_str(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// Render a key as a TOML bare key when safe, otherwise as a quoted key.
fn toml_key(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        s.to_string()
    } else {
        toml::Value::String(s.to_string()).to_string()
    }
}

/// Build the starter config file content for `lightcode init`.
fn build_init_toml(provider: &str, model: &str, api_key: &str, base_url: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str("# LightCode configuration — generated by `lightcode init`.\n");
    out.push_str("# See README for all available options.\n\n");
    out.push_str("[agent]\n");
    out.push_str(&format!("provider = {}\n", toml_str(provider)));
    out.push_str("max_context_tokens = 60000\n\n");
    out.push_str(&format!("[provider.{}]\n", toml_key(provider)));
    if let Some(b) = base_url {
        if !b.is_empty() {
            out.push_str(&format!("base_url = {}\n", toml_str(b)));
        }
    }
    if !model.is_empty() {
        out.push_str(&format!("model = {}\n", toml_str(model)));
    }
    if !api_key.is_empty() {
        out.push_str(&format!("api_key = {}\n", toml_str(api_key)));
    }
    out.push_str("\n[permissions]\n");
    out.push_str("shell = \"ask\"\n");
    out.push_str("edit = \"ask\"\n");
    out.push_str("write = \"ask\"\n");
    out
}

fn show_stats(id: Option<&str>) -> Result<()> {
    let id = match id {
        Some(id) => id.to_string(),
        None => session::storage::list()?
            .first()
            .map(|m| m.id.clone())
            .ok_or_else(|| anyhow!("no sessions yet"))?,
    };
    let s = session::storage::open(&id)?;
    let msgs = s.load_history()?;
    let mut users = 0usize;
    let mut assistants = 0usize;
    let mut tools = 0usize;
    let mut chars = 0usize;
    for m in &msgs {
        match m {
            providers::Message::User { content } => {
                users += 1;
                chars += content.chars().count();
            }
            providers::Message::Assistant {
                content,
                tool_calls,
                ..
            } => {
                assistants += 1;
                chars += content.as_ref().map_or(0, |c| c.chars().count());
                tools += tool_calls.len();
            }
            providers::Message::Tool { content, .. } => {
                tools += 1;
                chars += content.chars().count();
            }
            providers::Message::System { content } => chars += content.chars().count(),
        }
    }
    println!("session: {id}");
    println!("users: {users}  assistants: {assistants}  tool calls: {tools}");
    println!("estimated tokens (chars/4): {}", chars / 4);
    Ok(())
}

/// Interactive mode: launch the TUI, optionally resuming a session.
async fn run_tui(cli: &Cli) -> Result<()> {
    let cfg = config::Config::load(cli.config.as_deref())?;
    let provider_name = cli
        .provider
        .as_deref()
        .unwrap_or(&cfg.agent.provider)
        .to_lowercase();

    // Build every configured provider that has usable credentials, so the model
    // picker can switch provider at runtime.
    let mut provider_map: HashMap<String, Box<dyn Provider>> = HashMap::new();
    for name in cfg.provider.keys() {
        if let Ok(p) = build_provider(&cfg, Some(name), None) {
            provider_map.insert(name.clone(), p);
        }
    }
    let mut provider = provider_map
        .get(&provider_name)
        .map(|p| p.clone_box())
        .ok_or_else(|| anyhow!("provider '{provider_name}' is not configured or has no API key"))?;
    if let Some(m) = &cli.model {
        provider.set_model(m);
    }
    let model = cli
        .model
        .clone()
        .unwrap_or_else(|| default_model_for(&cfg, &provider_name));
    let all_models = all_models(&cfg, &provider_map);
    let policy = cfg.permissions.clone();
    let prompter: Box<dyn FnMut(&str) -> Choice + Send> = if cli.auto {
        Box::new(|_| Choice::Allow)
    } else {
        Box::new(interactive_prompter())
    };

    // Resolve + scope sessions to the current workspace (git root / dir).
    let cwd_path = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let workspace = workspace::resolve(&cwd_path);

    let session = if let Some(id) = &cli.session {
        let s = session::storage::open(id)?;
        eprintln!("resuming session {id}");
        Some(s)
    } else if cli.continue_session {
        let metas = session::storage::list()?;
        match metas.first() {
            Some(m) => {
                eprintln!("resuming session {}", m.id);
                Some(session::storage::open(&m.id)?)
            }
            None => {
                eprintln!("no saved sessions to resume");
                None
            }
        }
    } else {
        let s = session::storage::create()?;
        eprintln!("session: {}", s.id);
        Some(s)
    };

    let mut agent = Agent::new(
        provider,
        tools::Registry::default(),
        false,
        policy,
        prompter,
    );
    agent.set_max_context_tokens(cfg.agent.max_context_tokens);
    agent.set_agent_defs(cfg.agents.clone());
    agent.set_provider_map(provider_map);
    agent.repo_root = std::env::current_dir().ok();
    let mut restored_mode = crate::agent::AgentMode::Build;
    if let Some(s) = &session {
        let loaded = s.load_history()?;
        agent.history.extend(loaded);
        agent.set_session(session.clone());
        agent.set_approved(&s.approved_actions());
        if let Some(m) = s
            .read_mode()
            .and_then(|m| crate::agent::AgentMode::from_str(&m))
        {
            agent.mode = m;
            restored_mode = m;
        }
    }
    // --auto in the TUI switches the runtime to AUTO mode so the event-based
    // permission path auto-approves routine actions.
    if cli.auto {
        agent.mode = crate::agent::AgentMode::Auto;
        restored_mode = crate::agent::AgentMode::Auto;
    }
    let session_label = session.as_ref().map(|s| s.id.clone()).unwrap_or_default();
    let cwd = cwd_path.to_string_lossy().into_owned();
    let workspace_label = workspace.to_string_lossy().into_owned();
    let agents: Vec<(String, String)> = cfg
        .agents
        .iter()
        .map(|(k, v)| (k.clone(), v.model.clone().unwrap_or_default()))
        .collect();
    let saved_history = history::load();

    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        tui::run_tui(
            agent,
            tui::app::StatusInfo {
                model,
                provider: provider_name,
                session: session_label,
                cwd,
                workspace: workspace_label,
                models: all_models,
                history: saved_history,
                agents,
                mode: restored_mode,
            },
        )
        .await?;
    } else {
        agent.set_display_stream(true);
        repl(&mut agent).await?;
    }
    Ok(())
}

/// One-shot prompt execution, optionally with an NDJSON event stream.
async fn run_noninteractive(cli: &Cli, args: RunArgs) -> Result<()> {
    let cfg = config::Config::load(cli.config.as_deref())?;
    let provider = build_provider(&cfg, cli.provider.as_deref(), cli.model.as_deref())?;
    let policy = cfg.permissions.clone();

    let mut prompt = args.prompt.clone().unwrap_or_default();
    for f in &args.file {
        let meta = tokio::fs::metadata(f)
            .await
            .with_context(|| format!("--file {}", f.display()))?;
        if meta.len() > 10 * 1024 * 1024 {
            bail!("--file {} exceeds 10 MiB", f.display());
        }
        let content = tokio::fs::read_to_string(f)
            .await
            .with_context(|| format!("reading --file {}", f.display()))?;
        prompt.push_str(&format!("\n\n--- {} ---\n{content}", f.display()));
    }
    if prompt.trim().is_empty() && !std::io::stdin().is_terminal() {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        prompt.push_str(&buf);
    }
    let prompt = prompt.trim();
    if prompt.is_empty() {
        bail!("no prompt given; pass a prompt argument, --file, or pipe stdin");
    }

    let mut agent = Agent::new(
        provider,
        tools::Registry::default(),
        false,
        policy,
        Box::new(|_| Choice::Deny { feedback: None }),
    );
    agent.set_max_context_tokens(cfg.agent.max_context_tokens);
    agent.set_agent_defs(cfg.agents.clone());
    agent.repo_root = std::env::current_dir().ok();
    if let Some(name) = &args.agent {
        agent.set_agent(name);
    }
    if cli.auto {
        agent.prompter = Box::new(|_| Choice::Allow);
        agent.mode = crate::agent::AgentMode::Auto;
    }

    if args.format == "json" {
        let (tx, mut rx) = mpsc::channel(256);
        agent.set_events(Some(tx));
        let streamer = tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                print_json_event(&ev);
            }
        });
        let result = agent.run(prompt).await;
        streamer.abort();
        result?;
    } else {
        agent.set_display_stream(true);
        agent.run(prompt).await?;
    }
    Ok(())
}

fn print_json_event(ev: &agent::AgentEvent) {
    use serde_json::json;
    let out = match ev {
        agent::AgentEvent::Text(t) => json!({"type": "text", "content": t}),
        agent::AgentEvent::Reasoning(r) => json!({"type": "reasoning", "content": r}),
        agent::AgentEvent::ToolStart { name, args } => json!({
            "type": "tool_use", "name": name, "input": serde_json::from_str::<serde_json::Value>(args).unwrap_or_else(|_| json!(args))
        }),
        agent::AgentEvent::ToolOutput { name, output } => {
            json!({"type": "tool_result", "name": name, "output": output})
        }
        agent::AgentEvent::Diff { file, body } => {
            json!({"type": "diff", "file": file, "body": body})
        }
        agent::AgentEvent::Permission { prompt, .. } => {
            json!({"type": "permission", "message": prompt, "answer": "deny"})
        }
        agent::AgentEvent::Question {
            prompt, options, ..
        } => {
            json!({"type": "question", "prompt": prompt, "options": options})
        }
        agent::AgentEvent::Compact { removed } => {
            json!({"type": "compacted", "removed": removed})
        }
        agent::AgentEvent::Done { ok, message } => {
            json!({"type": "done", "ok": ok, "message": message})
        }
    };
    let _ = std::io::stdout().write_all(format!("{out}\n").as_bytes());
}

fn list_models(cli: &Cli) -> Result<()> {
    let cfg = config::Config::load(cli.config.as_deref())?;
    let provider_name = cli
        .provider
        .as_deref()
        .unwrap_or(&cfg.agent.provider)
        .to_lowercase();
    match cfg.provider.get(&provider_name) {
        Some(p) => {
            let mut ids: Vec<_> = p.models.iter().collect();
            ids.sort_by(|a, b| a.0.cmp(b.0));
            for (id, m) in ids {
                println!("{id}  {}", m.name.as_deref().unwrap_or(""));
            }
            if p.models.is_empty() {
                eprintln!("provider '{provider_name}' has no model list; use -m <model>");
            }
        }
        None => eprintln!("provider '{provider_name}' not in config"),
    }
    Ok(())
}

fn list_providers(cli: &Cli) -> Result<()> {
    let cfg = config::Config::load(cli.config.as_deref())?;
    let mut names: Vec<_> = cfg.provider.keys().collect();
    names.sort();
    for n in &names {
        let p = &cfg.provider[*n];
        let key = config::api_key_for(n, p)
            .map(|k| {
                if k.is_empty() {
                    "(no auth)"
                } else {
                    "(key set)"
                }
            })
            .unwrap_or_else(|| "(no key)");
        let model = if p.model.is_empty() {
            "<default>".to_string()
        } else {
            p.model.clone()
        };
        println!("{n:<20} {model:<40} {key}");
    }
    if names.is_empty() {
        eprintln!("no providers configured");
    }
    Ok(())
}

fn show_config(cli: &Cli) -> Result<()> {
    let cfg = config::Config::load(cli.config.as_deref())?;
    println!("provider: {}", cfg.agent.provider);
    println!("max_context_tokens: {}", cfg.agent.max_context_tokens);
    println!(
        "config: {}",
        cli.config
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "defaults".into())
    );
    if let Some(p) = cfg.provider.get(&cfg.agent.provider) {
        println!(
            "base_url: {}",
            p.resolved_base_url().as_deref().unwrap_or("<default>")
        );
        println!(
            "model: {}",
            if p.model.is_empty() {
                "<default>"
            } else {
                &p.model
            }
        );
    }
    Ok(())
}

fn build_provider(
    cfg: &config::Config,
    cli_provider: Option<&str>,
    cli_model: Option<&str>,
) -> Result<Box<dyn Provider>> {
    let name = cli_provider.unwrap_or(&cfg.agent.provider).to_lowercase();
    let mut pcfg = cfg.provider.get(&name).cloned().unwrap_or_default();
    if let Some(m) = cli_model {
        pcfg.model = m.to_string();
    }
    let api_key = config::api_key_for(&name, &pcfg).ok_or_else(|| {
        anyhow!(
            "no API key for provider '{name}'. Set {} or [provider.{name}].api_key in config.",
            config::api_key_env(&name)
        )
    })?;

    match name.as_str() {
        "anthropic" => Ok(Box::new(AnthropicProvider::new(pcfg, api_key))),
        _ => {
            // Any other provider uses the OpenAI-compatible chat completions protocol,
            // with known defaults for well-known names.
            let base_url = pcfg.resolved_base_url();
            if base_url.is_none() {
                pcfg.base_url = Some(default_base_url(&name)?);
            } else {
                pcfg.base_url = base_url;
            }
            if pcfg.model.is_empty() {
                let mut ids: Vec<_> = pcfg.models.keys().collect();
                ids.sort();
                pcfg.model = ids
                    .first()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| default_model(&name));
            }
            Ok(Box::new(OpenAiProvider::new(pcfg, api_key)))
        }
    }
}

fn default_base_url(name: &str) -> Result<String> {
    let url = match name {
        "openai" => "https://api.openai.com/v1",
        "opencode-go" => "https://opencode.ai/zen/go/v1",
        "openrouter" => "https://openrouter.ai/api/v1",
        other => {
            return Err(anyhow!(
                "provider '{other}' has no default endpoint; set [provider.{other}].base_url or implement it"
            ))
        }
    };
    Ok(url.to_string())
}

fn default_model(name: &str) -> String {
    match name {
        "opencode-go" => "deepseek-v4-flash".to_string(),
        "anthropic" => "claude-sonnet-4-5".to_string(),
        _ => "gpt-4o-mini".to_string(),
    }
}

fn default_model_for(cfg: &config::Config, provider_name: &str) -> String {
    cfg.provider
        .get(provider_name)
        .and_then(|p| {
            if !p.model.is_empty() {
                Some(p.model.clone())
            } else {
                let mut ids: Vec<_> = p.models.keys().collect();
                ids.sort();
                ids.first().map(|s| s.to_string())
            }
        })
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| default_model(provider_name))
}

/// Models from every usable provider, plus the active provider's fallback list.
fn all_models(
    cfg: &config::Config,
    provider_map: &HashMap<String, Box<dyn Provider>>,
) -> Vec<tui::app::ModelItem> {
    let mut out = Vec::new();
    let mut names: Vec<&String> = cfg.provider.keys().collect();
    names.sort();
    for name in names {
        if !provider_map.contains_key(name) {
            continue; // unusable (no key) → skip from the picker
        }
        let p = &cfg.provider[name];
        let mut ids: Vec<_> = p.models.keys().collect();
        ids.sort();
        for id in ids {
            out.push(tui::app::ModelItem {
                provider: name.clone(),
                id: id.clone(),
                name: p.models[id].name.clone().unwrap_or_default(),
            });
        }
    }
    out
}

fn interactive_prompter() -> impl FnMut(&str) -> Choice + Send {
    move |msg| {
        eprintln!("{msg}");
        loop {
            eprint!("  [y] Allow  [n] Deny  [s] Session  [w] Always: ");
            std::io::stderr().flush().ok();
            let mut line = String::new();
            if std::io::stdin().lock().read_line(&mut line).is_err() {
                return Choice::Deny { feedback: None };
            }
            match line.trim().to_lowercase().as_str() {
                "y" | "yes" | "allow" => return Choice::Allow,
                "s" | "session" => return Choice::AllowForSession,
                "w" | "always" => return Choice::Always,
                "" | "n" | "no" | "deny" => return Choice::Deny { feedback: None },
                _ => eprintln!("  (y/n/s/w)"),
            }
        }
    }
}

async fn repl(agent: &mut Agent) -> Result<()> {
    eprintln!("LightCode — type a request (Ctrl-D to exit)");
    let stdin = std::io::stdin();
    loop {
        eprint!("lightcode> ");
        std::io::stderr().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break;
        }
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        match agent.run(input).await {
            Ok(_) => eprintln!(),
            Err(e) => eprintln!("error: {e}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_toml_roundtrips_for_known_provider() {
        let text = build_init_toml("opencode-go", "deepseek-v4-flash", "zen-key", None);
        let cfg: config::Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg.agent.provider, "opencode-go");
        assert_eq!(cfg.agent.max_context_tokens, 60_000);
        assert_eq!(cfg.provider["opencode-go"].model, "deepseek-v4-flash");
        assert_eq!(
            cfg.provider["opencode-go"].api_key.as_deref(),
            Some("zen-key")
        );
        assert_eq!(cfg.permissions.shell, crate::permissions::Level::Ask);
    }

    #[test]
    fn init_toml_skips_key_when_empty() {
        let text = build_init_toml("openai", "gpt-4o-mini", "", None);
        let cfg: config::Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg.provider["openai"].api_key, None);
    }

    #[test]
    fn init_toml_custom_provider_with_base_url() {
        let text = build_init_toml(
            "lokal-kantor",
            "codex/gpt-5.6-sol",
            "",
            Some("http://host/v1"),
        );
        let cfg: config::Config = toml::from_str(&text).unwrap();
        assert_eq!(
            cfg.provider["lokal-kantor"].resolved_base_url().as_deref(),
            Some("http://host/v1")
        );
    }

    #[test]
    fn init_toml_quotes_odd_provider_names() {
        let text = build_init_toml("my prov", "m", "", None);
        assert!(text.contains(r#"[provider."my prov"]"#));
        let cfg: config::Config = toml::from_str(&text).unwrap();
        assert!(cfg.provider.contains_key("my prov"));
    }

    #[test]
    fn toml_strings_are_escaped() {
        let s = toml_str(r#"a"b\c"#);
        // Must be valid TOML that round-trips (quotes/backslashes survive).
        let doc: toml::Value = toml::from_str(&format!("x = {s}")).unwrap();
        assert_eq!(doc["x"].as_str(), Some(r#"a"b\c"#));
        assert_eq!(toml_key("plain-name"), "plain-name");
        assert_eq!(toml_key("with space"), r#""with space""#);
    }
}
