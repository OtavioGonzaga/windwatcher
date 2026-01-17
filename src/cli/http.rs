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

    /// JWT secret
    #[arg(long)]
    pub jwt_secret: Option<String>,
}
