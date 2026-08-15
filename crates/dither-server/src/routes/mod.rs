//! Route table and the middleware every request passes through.

pub mod dither;
pub mod health;

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderName, HeaderValue, Method};
use axum::routing::{get, post};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::config::{Config, Origins};

/// Builds the application.
pub fn router(config: Arc<Config>) -> Router {
    let api = Router::new()
        .route("/dither", post(dither::dither))
        .route("/buffer", post(dither::buffer))
        .route("/options", get(dither::options));

    Router::new()
        .route("/health", get(health::health))
        .nest("/api", api)
        // Innermost first: the limit rejects oversized bodies, and CORS wraps it
        // so even that rejection reaches the browser with the right headers.
        .layer(DefaultBodyLimit::max(config.max_upload_bytes))
        .layer(cors(&config))
        .layer(TraceLayer::new_for_http())
        .with_state(config)
}

fn cors(config: &Config) -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any)
        // Custom headers stay invisible to `fetch` unless they are exposed.
        .expose_headers([
            HeaderName::from_static(dither::X_IMAGE_SIZE),
            HeaderName::from_static(dither::X_PANEL_ORIENTATION),
        ]);

    match &config.origins {
        Origins::Any => layer.allow_origin(Any),
        // Every entry parsed cleanly at startup, so nothing is dropped here.
        Origins::List(list) => layer.allow_origin(
            list.iter()
                .filter_map(|origin| HeaderValue::from_str(origin).ok())
                .collect::<Vec<_>>(),
        ),
    }
}
