use clap::Args;

#[derive(Args, Debug)]
#[command(next_help_heading = "DATABASE")]
pub struct DatabaseCli {
    /// Database URL
    #[arg(long)]
    pub database_url: Option<String>,
}
