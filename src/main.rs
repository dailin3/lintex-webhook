use lintex_webhook::{AppConfig, AppState, app};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("lintex_webhook=info,tower_http=info")),
        )
        .init();

    let config = AppConfig::from_env()?;
    let listener = tokio::net::TcpListener::bind(config.listen_addr).await?;
    let state = AppState::new(
        config.token,
        config.config_repository,
        config.services_config,
        config.runs_directory,
    );

    info!(address = %config.listen_addr, "lintex webhook listening");
    axum::serve(listener, app(state)).await?;
    Ok(())
}
