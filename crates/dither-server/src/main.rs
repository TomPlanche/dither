//! Reads the configuration, binds the socket, serves until ctrl-c.

use std::process::ExitCode;
use std::sync::Arc;

use dither_server::{Config, router};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("dither_server=info,tower_http=info")),
        )
        .init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            tracing::error!("{message}");
            ExitCode::FAILURE
        },
    }
}

async fn run() -> Result<(), String> {
    let config = Arc::new(Config::from_env()?);

    let listener = TcpListener::bind(config.addr)
        .await
        .map_err(|e| format!("could not bind {}: {e}", config.addr))?;

    tracing::info!(
        addr = %config.addr,
        origins = ?config.origins,
        max_upload_bytes = config.max_upload_bytes,
        "listening"
    );

    axum::serve(listener, router(Arc::clone(&config)))
        .with_graceful_shutdown(shutdown())
        .await
        .map_err(|e| format!("the server stopped: {e}"))
}

async fn shutdown() {
    match tokio::signal::ctrl_c().await {
        Ok(()) => tracing::info!("shutting down"),
        Err(e) => tracing::error!("could not listen for ctrl-c, shutting down: {e}"),
    }
}
