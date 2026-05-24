use std::{net::SocketAddr, sync::Arc};

use tokio::net::TcpListener;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use windwatcher::{
    api::{http::router as http_router, ws::handler::ws_handler, ws::manager::WsManager},
    application::{chat_service::ChatService, user_service::UserService},
    config::{AppConfig, DatabaseProvider},
    db::seaorm::{SeaOrmChatRepository, SeaOrmUserRepository},
    domain::ports,
    jobs::build_job_runtime,
    state::AppState,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = AppConfig::load().map_err(|e| anyhow::anyhow!("config error: {e}"))?;
    tracing::info!(
        host = %config.server_host,
        port = %config.server_port,
        db = %config.database_url,
        "starting windwatcher"
    );

    let (user_repo, chat_repo): (
        Arc<dyn ports::UserRepository>,
        Arc<dyn ports::ChatRepository>,
    ) = match config.database_provider {
        DatabaseProvider::MongoDb => {
            #[cfg(feature = "mongodb")]
            {
                let mongo_db = windwatcher::db::mongodb::setup_mongodb(&config).await?;
                let user_repo =
                    Arc::new(windwatcher::db::mongodb::user_repo::MongoUserRepository {
                        col: mongo_db.collection("users"),
                    });
                let chat_repo =
                    Arc::new(windwatcher::db::mongodb::chat_repo::MongoChatRepository {
                        db: mongo_db,
                    });
                (user_repo, chat_repo)
            }
            #[cfg(not(feature = "mongodb"))]
            {
                anyhow::bail!(
                    "DATABASE_URL points to MongoDB but the binary was compiled without \
                     the `mongodb` feature. Recompile with `--features mongodb`."
                );
            }
        }
        _ => {
            let db = windwatcher::db::seaorm::setup_database(&config).await?;
            let user_repo = Arc::new(SeaOrmUserRepository { db: db.clone() });
            let chat_repo = Arc::new(SeaOrmChatRepository { db });
            (user_repo, chat_repo)
        }
    };

    let ws_manager = Arc::new(WsManager::new());
    let job_runtime =
        build_job_runtime(&config, Arc::clone(&chat_repo), Arc::clone(&ws_manager)).await?;
    let job_queue: Arc<dyn ports::JobQueue> = job_runtime.queue();

    let user_service = Arc::new(UserService::new(
        Arc::clone(&user_repo),
        config.jwt_secret.clone(),
        config.jwt_expiry_secs,
    ));
    let chat_service = Arc::new(ChatService::new(
        Arc::clone(&chat_repo),
        Arc::clone(&job_queue),
    ));

    let state = AppState {
        config: Arc::new(config.clone()),
        user_service,
        chat_service,
        ws_manager,
    };

    let app = http_router()
        .route("/ws", axum::routing::get(ws_handler))
        .route("/health", axum::routing::get(health))
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", config.server_host, config.server_port)
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid bind address: {e}"))?;

    tracing::info!(%addr, "listening");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health() -> &'static str {
    "ok"
}
