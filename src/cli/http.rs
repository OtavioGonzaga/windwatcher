use clap::Args;

#[derive(Args, Debug)]
#[command(next_help_heading = "HTTP OPTIONS")]
pub struct HttpCli {
    /// Host/IP to bind the HTTP server
    #[arg(long)]
    pub http_host: Option<String>,

    /// Port to bind the HTTP server
    #[arg(long)]
    pub http_port: Option<u16>,

    /// Token secret
    #[arg(long)]
    pub token_secret: Option<String>,

    /// Token time to live in seconds
    #[arg(long)]
    pub token_ttl: Option<String>,

    /// Refresh token time to live in seconds
    #[arg(long)]
    pub refresh_token_ttl: Option<String>,
}
