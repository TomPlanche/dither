//! Runtime configuration, read from the environment.
//!
//! Everything has a working default, so `cargo run -p dither-server` is enough
//! to start serving a Vite dev server on port 5173.

use std::env;
use std::net::{Ipv4Addr, SocketAddr};

use axum::http::HeaderValue;

/// Where the Svelte dev server usually lives.
const DEFAULT_ORIGINS: &str = "http://localhost:5173,http://127.0.0.1:5173";
const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 3000;
const DEFAULT_MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Config {
    pub addr: SocketAddr,
    pub origins: Origins,
    pub max_upload_bytes: usize,
}

/// Which browser origins may call the API.
#[derive(Debug, Clone)]
pub enum Origins {
    /// `CORS_ORIGINS=*`. Fine for a local backend, not for a public one.
    Any,
    List(Vec<String>),
}

impl Default for Config {
    fn default() -> Self {
        Self {
            addr: SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_PORT)),
            origins: Origins::List(DEFAULT_ORIGINS.split(',').map(str::to_string).collect()),
            max_upload_bytes: DEFAULT_MAX_UPLOAD_BYTES,
        }
    }
}

impl Config {
    /// Reads the configuration, returning a message the caller can print as-is.
    ///
    /// * `HOST` / `PORT` — where to listen. Defaults to `127.0.0.1:3000`.
    /// * `CORS_ORIGINS` — comma separated, or `*`. Defaults to the Vite dev server.
    /// * `MAX_UPLOAD_BYTES` — largest accepted request body. Defaults to 25 MiB.
    pub fn from_env() -> Result<Self, String> {
        let host = var("HOST").unwrap_or_else(|| DEFAULT_HOST.to_string());
        let port = match var("PORT") {
            Some(raw) => raw
                .parse::<u16>()
                .map_err(|_| format!("PORT must be a port number, got `{raw}`"))?,
            None => DEFAULT_PORT,
        };
        let addr = format!("{host}:{port}")
            .parse::<SocketAddr>()
            .map_err(|_| format!("HOST and PORT do not form an address: `{host}:{port}`"))?;

        let raw_origins = var("CORS_ORIGINS").unwrap_or_else(|| DEFAULT_ORIGINS.to_string());
        let origins = if raw_origins.trim() == "*" {
            Origins::Any
        } else {
            let list: Vec<String> = raw_origins
                .split(',')
                .map(|o| o.trim().to_string())
                .filter(|o| !o.is_empty())
                .collect();
            if list.is_empty() {
                return Err("CORS_ORIGINS is set but lists no origin".into());
            }
            // Checked once here so the CORS layer can assume every entry is usable.
            for origin in &list {
                HeaderValue::from_str(origin)
                    .map_err(|_| format!("CORS_ORIGINS holds `{origin}`, which is not a valid origin"))?;
            }
            Origins::List(list)
        };

        let max_upload_bytes = match var("MAX_UPLOAD_BYTES") {
            Some(raw) => raw
                .parse::<usize>()
                .map_err(|_| format!("MAX_UPLOAD_BYTES must be a byte count, got `{raw}`"))?,
            None => DEFAULT_MAX_UPLOAD_BYTES,
        };
        if max_upload_bytes == 0 {
            return Err("MAX_UPLOAD_BYTES must be greater than zero".into());
        }

        Ok(Self {
            addr,
            origins,
            max_upload_bytes,
        })
    }
}

/// An environment variable, treating an empty value as unset.
fn var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.trim().is_empty())
}
