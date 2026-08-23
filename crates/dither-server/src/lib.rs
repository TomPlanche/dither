//! HTTP backend over the core dithering pipeline.
//!
//! One stateless job: take an uploaded photo, run it through
//! [`dither_core`], and hand back a dithered PNG. Nothing is stored between
//! requests.
//!
//! | Route | What it does |
//! | --- | --- |
//! | `GET /health` | Liveness probe. |
//! | `GET /api/options` | Defaults, accepted values, the palette. |
//! | `POST /api/dither` | Dithered PNG. |
//!
//! The binary is a thin wrapper: read [`Config`](config::Config) from the
//! environment, build [`routes::router`], serve it.

pub mod config;
pub mod error;
pub mod params;
pub mod routes;

pub use config::Config;
pub use routes::router;
