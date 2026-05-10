//! `sqldev config` subcommand — inspect resolved configuration.

use anyhow::{Context, Result};
use clap::{Args as ClapArgs, Subcommand};
use sqldev_config::{AuthBlock, Config, EnvBlock};

use crate::config_ctx::ConfigContext;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Print the resolved config (with secrets redacted).
    Show(ShowArgs),
}

#[derive(ClapArgs, Debug)]
pub struct ShowArgs {
    /// Emit full YAML for every env (default: just the selected env).
    #[arg(long)]
    pub all: bool,

    /// Output as JSON instead of YAML.
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: Args, ctx: &ConfigContext) -> Result<()> {
    match args.action {
        Action::Show(s) => show(&s, ctx),
    }
}

fn show(args: &ShowArgs, ctx: &ConfigContext) -> Result<()> {
    let cfg = ctx
        .config
        .as_ref()
        .context("no .sqldev.yml found (use --config <path> or run from a directory with one)")?;

    if let Some(src) = &ctx.source {
        eprintln!("# source: {}", src.display());
    }
    if let Some(name) = &ctx.selected_env {
        eprintln!("# selected_env: {name}");
    }

    let redacted = redact_config(
        cfg,
        if args.all {
            None
        } else {
            ctx.selected_env.as_deref()
        },
    );
    if args.json {
        println!("{}", serde_json::to_string_pretty(&redacted)?);
    } else {
        println!("{}", serde_yaml::to_string(&redacted)?);
    }
    Ok(())
}

/// Produce a copy of `cfg` with passwords replaced by `***REDACTED***`.
/// If `only` is `Some`, all other env blocks are filtered out.
fn redact_config(cfg: &Config, only: Option<&str>) -> Config {
    let mut out = Config {
        default_env: cfg.default_env.clone(),
        envs: std::collections::BTreeMap::new(),
    };
    for (name, env) in &cfg.envs {
        if let Some(target) = only
            && name != target
        {
            continue;
        }
        out.envs.insert(name.clone(), redact_env(env));
    }
    out
}

fn redact_env(env: &EnvBlock) -> EnvBlock {
    EnvBlock {
        host: env.host.clone(),
        port: env.port,
        database: env.database.clone(),
        auth: match &env.auth {
            AuthBlock::Sql { user, password: _ } => AuthBlock::Sql {
                user: user.clone(),
                password: "***REDACTED***".to_string(),
            },
        },
        trust_server_certificate: env.trust_server_certificate,
        protected: env.protected,
    }
}
