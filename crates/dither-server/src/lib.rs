//! HTTP backend over the reframe dithering pipeline.
//!
//! One stateless job: take an uploaded photo, run it through
//! [`reframe_dither`], and hand back either a dithered PNG or the packed
//! e-paper frame buffer. Nothing is stored between requests.
//!
//! | Route | What it does |
//! | --- | --- |
//! | `GET /health` | Liveness probe. |
//! | `GET /api/options` | Defaults, accepted values, panel palette. |
//! | `POST /api/dither` | Dithered PNG. |
//! | `POST /api/buffer` | Packed 400x600 frame buffer. |
//!
//! The binary is a thin wrapper: read [`Config`](config::Config) from the
//! environment, build [`routes::router`], serve it.

pub mod config;
pub mod error;
pub mod params;
pub mod routes;

pub use config::Config;
pub use routes::router;
