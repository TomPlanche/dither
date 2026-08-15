//! The dithering endpoints.
//!
//! Both accept an image the same two ways: a raw body (`fetch(url, { method:
//! 'POST', body: file })`) or a `multipart/form-data` field named `image`.
//! Settings ride along in the query string; see [`crate::params`].

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequest, Multipart, Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use reframe_dither::{
    DISPLAY_IMAGE_SIZE, DISPLAY_PANEL_SIZE, IndexedImage, Orientation, PanelPalette, RgbImage, apply_dithering,
    display, io, resize,
};
use serde::Serialize;

use crate::config::Config;
use crate::error::ApiError;
use crate::params::{DitherParams, Format, MAX_DIMENSION, MAX_SCALE, MAX_SOURCE_PIXELS, Method, Params};

/// Size of the returned image, as `WIDTHxHEIGHT`.
pub const X_IMAGE_SIZE: &str = "x-image-size";
/// How the frame buffer had to be rotated: `panel` or `rotated`.
pub const X_PANEL_ORIENTATION: &str = "x-panel-orientation";

/// `POST /api/dither` — a dithered PNG.
pub async fn dither(params: Params, request: Request) -> Result<Response, ApiError> {
    let source = read_image(request).await?;
    let Params(params) = params;

    let png = blocking(move || render_png(&source, params)).await?;

    let content_type = match params.format {
        // Both are PNGs; the distinction is the colour type inside.
        Format::Indexed | Format::Rgb => "image/png",
    };
    Ok((
        [
            (CONTENT_TYPE.as_str(), content_type),
            ("cache-control", "no-store"),
            (X_IMAGE_SIZE, png.size.as_str()),
        ],
        png.bytes,
    )
        .into_response())
}

/// `POST /api/buffer` — the packed e-paper frame buffer.
///
/// `format` and `scale` do not apply here and are ignored: the panel takes one
/// layout only.
pub async fn buffer(params: Params, request: Request) -> Result<Response, ApiError> {
    let source = read_image(request).await?;
    let Params(params) = params;

    let panel = blocking(move || render_buffer(&source, params)).await?;

    Ok((
        [
            (CONTENT_TYPE.as_str(), "application/octet-stream"),
            ("cache-control", "no-store"),
            (X_IMAGE_SIZE, panel.size.as_str()),
            (X_PANEL_ORIENTATION, panel.orientation),
        ],
        panel.bytes,
    )
        .into_response())
}

/// `GET /api/options` — defaults, accepted values and panel facts.
///
/// `defaults` uses the same names as the query string, so a client can feed it
/// straight into `URLSearchParams`.
pub async fn options(State(config): State<Arc<Config>>) -> Json<OptionsBody> {
    let defaults = DitherParams::default();
    let palette = PanelPalette::new(defaults.saturation);

    Json(OptionsBody {
        methods: Method::ALL,
        formats: [Format::Indexed, Format::Rgb],
        bayer_sizes: [2, 4, 8],
        defaults,
        limits: Limits {
            max_upload_bytes: config.max_upload_bytes,
            max_dimension: MAX_DIMENSION,
            max_scale: MAX_SCALE,
            max_source_pixels: MAX_SOURCE_PIXELS,
        },
        panel: Panel {
            image_size: DISPLAY_IMAGE_SIZE,
            panel_size: DISPLAY_PANEL_SIZE,
            palette: palette.colors().iter().map(hex).collect(),
        },
    })
}

#[derive(Serialize)]
pub struct OptionsBody {
    methods: [Method; 6],
    formats: [Format; 2],
    bayer_sizes: [u32; 3],
    defaults: DitherParams,
    limits: Limits,
    panel: Panel,
}

#[derive(Serialize)]
struct Limits {
    max_upload_bytes: usize,
    max_dimension: u32,
    max_scale: u32,
    max_source_pixels: u64,
}

#[derive(Serialize)]
struct Panel {
    image_size: (u32, u32),
    panel_size: (u32, u32),
    palette: Vec<String>,
}

fn hex(rgb: &[u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

struct Rendered {
    bytes: Vec<u8>,
    size: String,
}

struct PanelFrame {
    bytes: Vec<u8>,
    size: String,
    orientation: &'static str,
}

/// Decode, resize, dither and re-encode. Runs on a blocking thread.
fn render_png(source: &[u8], params: DitherParams) -> Result<Rendered, ApiError> {
    let options = params.to_options()?;
    let working = prepare(source, params)?;

    let dithered = apply_dithering(&working, &options);
    let dithered: IndexedImage = if params.scale > 1 {
        dithered.scale_nearest(params.scale)
    } else {
        dithered
    };

    let bytes = match params.format {
        Format::Indexed => io::encode_indexed_png(&dithered),
        Format::Rgb => io::encode_rgb_png(&dithered.to_rgb()),
    }
    .map_err(|e| ApiError::internal(format!("could not encode the result: {e}")))?;

    Ok(Rendered {
        size: format!("{}x{}", dithered.width(), dithered.height()),
        bytes,
    })
}

/// Decode, resize, dither and pack into the panel's frame buffer.
fn render_buffer(source: &[u8], params: DitherParams) -> Result<PanelFrame, ApiError> {
    let options = params.to_options()?;
    let working = prepare(source, params)?;

    let (bytes, dithered, orientation) = display::dither_to_display_buffer(&working, &options);
    let orientation = match orientation {
        Orientation::Panel => "panel",
        Orientation::Rotated => "rotated",
        // The packer would still produce bytes, but the panel could not show them.
        Orientation::Unexpected => {
            return Err(ApiError::bad_request(format!(
                "the panel takes {}x{} or {}x{}, but this request produced {}x{}; \
                 leave `resize` on, or set width and height to one of those",
                DISPLAY_IMAGE_SIZE.0,
                DISPLAY_IMAGE_SIZE.1,
                DISPLAY_PANEL_SIZE.0,
                DISPLAY_PANEL_SIZE.1,
                dithered.width(),
                dithered.height(),
            )));
        },
    };

    Ok(PanelFrame {
        size: format!("{}x{}", dithered.width(), dithered.height()),
        bytes,
        orientation,
    })
}

/// Decodes the upload and resizes it to the working size.
fn prepare(source: &[u8], params: DitherParams) -> Result<RgbImage, ApiError> {
    let photo = io::decode_rgb(source).map_err(|e| ApiError::bad_request(format!("could not read the image: {e}")))?;

    let (width, height) = photo.dimensions();
    if u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS {
        return Err(ApiError::bad_request(format!(
            "the image is {width}x{height}, over the {MAX_SOURCE_PIXELS} pixel limit"
        )));
    }

    Ok(match params.target_size() {
        Some(size) => resize::resize_to_fit(&photo, size, params.fit()),
        None => photo,
    })
}

/// Reads the image out of either a raw body or a multipart `image` field.
async fn read_image(request: Request) -> Result<Bytes, ApiError> {
    let is_multipart = request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("multipart/form-data"));

    let bytes = if is_multipart {
        multipart_image(request).await?
    } else {
        // Keeping the rejection's own status matters: a body over the limit is
        // a 413, not a 400.
        Bytes::from_request(request, &())
            .await
            .map_err(|e| ApiError::new(e.status(), e.body_text()))?
    };

    if bytes.is_empty() {
        return Err(ApiError::bad_request("the request carried no image"));
    }
    Ok(bytes)
}

async fn multipart_image(request: Request) -> Result<Bytes, ApiError> {
    let mut multipart = Multipart::from_request(request, &())
        .await
        .map_err(|e| ApiError::new(e.status(), e.body_text()))?;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::new(e.status(), e.body_text()))?
    {
        if matches!(field.name(), Some("image" | "file")) {
            return field
                .bytes()
                .await
                .map_err(|e| ApiError::new(e.status(), e.body_text()));
        }
    }

    Err(ApiError::bad_request("the multipart body has no `image` field"))
}

/// Runs the pipeline off the async runtime; a 600x400 dither is milliseconds of CPU.
async fn blocking<T, F>(work: F) -> Result<T, ApiError>
where
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| ApiError::internal(format!("the dither task did not finish: {e}")))?
}
