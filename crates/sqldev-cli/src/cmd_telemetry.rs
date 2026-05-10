//! `sqldev telemetry {enable,disable,status}` subcommand.

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::telemetry::{self, Consent};

#[derive(Parser, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Opt in to anonymous usage telemetry.
    Enable,
    /// Opt out of telemetry. Existing queued events are kept on disk
    /// (so they can be inspected) but no new events are written.
    Disable,
    /// Show the current opt-in state, env-var overrides, and where the
    /// state and queue files live.
    Status,
}

pub fn run(args: &Args) -> Result<()> {
    let Some(rec) = telemetry::Recorder::from_env() else {
        anyhow::bail!(
            "could not locate a config directory; set SQLDEV_CONFIG_DIR \
             or HOME and retry"
        );
    };
    match args.action {
        Action::Enable => {
            let _ = rec.set_consent(Consent::Enabled)?;
            println!(
                "Telemetry enabled.\n\
                 Anonymous usage events (command name + exit code +\n\
                 duration bucket only) are recorded locally.\n\
                 Run `sqldev telemetry status` for details, or\n\
                 `sqldev telemetry disable` to opt out at any time."
            );
        }
        Action::Disable => {
            let _ = rec.set_consent(Consent::Disabled)?;
            println!("Telemetry disabled.");
        }
        Action::Status => {
            print!("{}", rec.status_report());
        }
    }
    Ok(())
}
