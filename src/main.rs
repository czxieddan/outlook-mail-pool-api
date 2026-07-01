use std::sync::Arc;

use anyhow::Result;
use outlook_mail_pool_api::{
    app::{AppState, router},
    config::AppConfig,
    store::Store,
};
use tokio::{net::TcpListener, sync::Semaphore};
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let config = AppConfig::from_env()?;
    std::fs::create_dir_all("data")?;
    let store = Store::connect(&config.database_url).await?;
    let state = AppState {
        store,
        imap_permits: Arc::new(Semaphore::new(config.max_imap_workers.max(1))),
        config: config.clone(),
    };

    let listener = TcpListener::bind(config.bind).await?;
    tracing::info!(addr = %config.bind, "outlook mail pool api listening");
    axum::serve(listener, router(state)).await?;
    Ok(())
}
