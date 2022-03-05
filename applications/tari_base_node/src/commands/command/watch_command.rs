use std::fmt;

use anyhow::Error;
use async_trait::async_trait;
use clap::Parser;

use super::{CommandContext, HandleCommand};

pub type WatchCommand = Args;

impl fmt::Display for WatchCommand {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.line().fmt(f)
    }
}

/// Repeat a command within an interval.
#[derive(Debug, Parser)]
pub struct Args {
    /// Interval in seconds
    #[clap(short, long)]
    pub interval: Option<u64>,
    /// The command to perform. `status` if empty.
    pub command: Option<String>,
}

impl Args {
    pub fn line(&self) -> &str {
        self.command.as_ref().map(String::as_ref).unwrap_or("status")
    }
}

#[async_trait]
impl HandleCommand<Args> for CommandContext {
    async fn handle_command(&mut self, _: Args) -> Result<(), Error> {
        Ok(())
    }
}
