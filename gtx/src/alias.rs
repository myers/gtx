use clap::{Args, Subcommand};
use eyre::Result;

use crate::config;

#[derive(Args)]
pub struct AliasCommand {
    #[command(subcommand)]
    action: AliasAction,
}

#[derive(Subcommand)]
enum AliasAction {
    /// List all aliases
    List,
    /// Set an alias
    Set(SetArgs),
    /// Delete an alias
    Delete(DeleteArgs),
}

#[derive(Args)]
struct SetArgs {
    /// Alias name
    name: String,

    /// Expansion (gt command, or !shell command)
    expansion: String,
}

#[derive(Args)]
struct DeleteArgs {
    /// Alias name
    name: String,
}

impl AliasCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            AliasAction::List => list_aliases().await,
            AliasAction::Set(args) => set_alias(args).await,
            AliasAction::Delete(args) => delete_alias(args).await,
        }
    }
}

async fn list_aliases() -> Result<()> {
    let aliases = config::load_aliases();

    if aliases.is_empty() {
        eprintln!("No aliases configured");
        return Ok(());
    }

    for (name, expansion) in &aliases {
        println!("{name}: {expansion}");
    }

    Ok(())
}

async fn set_alias(args: &SetArgs) -> Result<()> {
    config::set_alias(&args.name, &args.expansion)?;
    eprintln!("Alias '{}' set to '{}'", args.name, args.expansion);
    Ok(())
}

async fn delete_alias(args: &DeleteArgs) -> Result<()> {
    if config::delete_alias(&args.name)? {
        eprintln!("Alias '{}' deleted", args.name);
    } else {
        eyre::bail!("Alias '{}' not found", args.name);
    }
    Ok(())
}

/// Expand a regular alias, replacing $N placeholders with args.
pub fn expand_alias(expansion: &str, args: &[String]) -> Vec<String> {
    let mut result = expansion.to_string();

    // Replace $1, $2, etc. with positional args
    let mut used = vec![false; args.len()];
    for (i, arg) in args.iter().enumerate() {
        let placeholder = format!("${}", i + 1);
        if result.contains(&placeholder) {
            result = result.replace(&placeholder, arg);
            used[i] = true;
        }
    }

    // Split the expansion into tokens
    let mut tokens: Vec<String> = result
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    // Append unused args
    for (i, arg) in args.iter().enumerate() {
        if !used[i] {
            tokens.push(arg.clone());
        }
    }

    tokens
}

/// Run a shell alias (expansion starts with !).
pub fn run_shell_alias(expansion: &str, args: &[String]) -> Result<()> {
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(expansion)
        .arg("--")
        .args(args)
        .status()
        .map_err(|e| eyre::eyre!("Failed to run shell alias: {e}"))?;

    std::process::exit(status.code().unwrap_or(1));
}
