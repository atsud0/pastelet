use pastelet::{AppState, config::Config, db, routes, services::cleanup};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = Config::from_env()?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_new(&config.rust_log)?)
        .try_init()?;

    let pool = db::connect(&config).await?;
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    tracing::info!(address = %listener.local_addr()?, "server listening");

    let (cleanup_stop, cleanup_receiver) = tokio::sync::watch::channel(false);
    let cleanup_task = tokio::spawn(cleanup::run(
        pool.clone(),
        config.cleanup_interval,
        cleanup_receiver,
    ));

    let app = routes::router(AppState {
        config: Arc::new(config),
        pool: pool.clone(),
    });
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;
    let _ = cleanup_stop.send(true);
    let cleanup_result = cleanup_task.await;
    pool.close().await;
    cleanup_result?;
    result?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    result = tokio::signal::ctrl_c() => {
                        if let Err(error) = result {
                            tracing::error!(%error, "cannot listen for interrupt");
                        }
                    }
                    _ = terminate.recv() => {}
                }
            }
            Err(error) => tracing::error!(%error, "cannot listen for termination"),
        }
    }
    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "cannot listen for interrupt");
    }
}
