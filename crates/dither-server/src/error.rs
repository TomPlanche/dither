//! The single error type every handler returns.
//!
//! Failures reach the browser as JSON (`{"error": "...", "status": 400}`) so the
//! Svelte side can show the message without sniffing at response bodies.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    /// The request was malformed: bad parameters, missing field, undecodable image.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    /// The pipeline itself failed. Nothing the caller can fix.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: &'a str,
    status: u16,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if self.status.is_server_error() {
            tracing::error!(error = self.message, "request failed");
        }
        let body = ErrorBody {
            error: &self.message,
            status: self.status.as_u16(),
        };
        (self.status, Json(body)).into_response()
    }
}
