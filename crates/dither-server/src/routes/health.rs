//! Liveness probe.

use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct Health {
    status: &'static str,
    service: &'static str,
    version: &'static str,
}

pub async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        service: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
    })
}
