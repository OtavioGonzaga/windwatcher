use clap::Args;

#[derive(Args, Debug)]
#[command(next_help_heading = "LOGGING")]
pub struct LoggingCli {
    /// Log verbosity level
    #[arg(long)]
    pub log_level: Option<String>,
}
