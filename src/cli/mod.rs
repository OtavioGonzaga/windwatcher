use clap::Parser;

pub mod database;
pub mod http;
pub mod logging;

#[derive(Parser, Debug)]
#[command(
    name = "windwatcher",
    version,
    about = "Realtime chat server",
    long_about = None
)]
pub struct Cli {
    #[command(flatten)]
    pub http: http::HttpCli,

    #[command(flatten)]
    pub logging: logging::LoggingCli,

    #[command(flatten)]
    pub database: database::DatabaseCli,
}
