//! The dithering endpoints.
//!
//! An image arrives one of two ways: a raw body (`fetch(url, { method: 'POST',
//! body: file })`) or a `multipart/form-data` field named `image`. Settings
//! ride along in the query string; see [`crate::params`].

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequest, Multipart, Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use dither_core::{DEFAULT_SIZE, IndexedImage, MAX_CROP_ZOOM, Palette, RgbImage, apply_dithering, io, resize};
use serde::Serialize;

use crate::config::Config;
use crate::error::ApiError;
use crate::params::{
    DitherParams, Format, MAX_DIMENSION, MAX_SCALE, MAX_SOURCE_PIXELS, Method, Params, Preset, Resize,
};

/// Size of the returned image, as `WIDTHxHEIGHT`.
pub const X_IMAGE_SIZE: &str = "x-image-size";
/// The part of the upload that was read, as `X,Y,WIDTH,HEIGHT` in source pixels.
///
/// The whole photo when `crop` is off, so it always says what the source measured, which is what a client needs before
/// it can name a corner in `crop_from`.
pub const X_CROP_RECT: &str = "x-crop-rect";

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
            (X_CROP_RECT, png.crop.as_str()),
        ],
        png.bytes,
    )
        .into_response())
}

/// `GET /api/options` — defaults, accepted values and the palette.
///
/// `defaults` uses the same names as the query string, so a client can feed it
/// straight into `URLSearchParams`.
pub async fn options(State(config): State<Arc<Config>>) -> Json<OptionsBody> {
    let defaults = DitherParams::default();
    let palette = Palette::new(defaults.saturation);

    Json(OptionsBody {
        methods: Method::ALL,
        formats: [Format::Indexed, Format::Rgb],
        bayer_sizes: [2, 4, 8],
        presets: Preset::ALL
            .into_iter()
            .map(|preset| PresetRatio {
                name: preset.name(),
                ratio: preset.ratio(),
            })
            .collect(),
        defaults,
        limits: Limits {
            max_upload_bytes: config.max_upload_bytes,
            max_dimension: MAX_DIMENSION,
            max_scale: MAX_SCALE,
            max_source_pixels: MAX_SOURCE_PIXELS,
            max_crop_zoom: MAX_CROP_ZOOM,
        },
        default_size: DEFAULT_SIZE,
        palette: palette.colors().iter().map(hex).collect(),
    })
}

#[derive(Serialize)]
pub struct OptionsBody {
    methods: [Method; 7],
    formats: [Format; 2],
    bayer_sizes: [u32; 3],
    presets: Vec<PresetRatio>,
    defaults: DitherParams,
    limits: Limits,
    /// The working size a request lands on when it names neither `width` nor `height`.
    default_size: (u32, u32),
    /// The palette at the default saturation, as `#rrggbb`, in slot order.
    palette: Vec<String>,
}

/// A `preset` name and the `width:height` ratio it stands for, so a client can label its own picker.
///
/// A ratio rather than a size, because the size a preset comes out at depends on the `width` and `height` it is fitted
/// inside. `x-image-size` reports what a given request landed on.
#[derive(Serialize)]
struct PresetRatio {
    name: &'static str,
    ratio: (u32, u32),
}

#[derive(Serialize)]
struct Limits {
    max_upload_bytes: usize,
    max_dimension: u32,
    max_scale: u32,
    max_source_pixels: u64,
    max_crop_zoom: f32,
}

fn hex(rgb: &[u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

struct Rendered {
    bytes: Vec<u8>,
    size: String,
    crop: String,
}

/// Decode, resize, dither and re-encode. Runs on a blocking thread.
fn render_png(source: &[u8], params: DitherParams) -> Result<Rendered, ApiError> {
    let options = params.to_options()?;
    let (working, crop) = prepare(source, params)?;

    // `method=none` stops here: there is no palette to index, so the framing comes back as a plain RGB PNG.
    let Some(options) = options else {
        let plain = resize::scale_nearest(&working, params.scale);
        let bytes =
            io::encode_rgb_png(&plain).map_err(|e| ApiError::internal(format!("could not encode the result: {e}")))?;
        return Ok(Rendered {
            size: format!("{}x{}", plain.width(), plain.height()),
            bytes,
            crop,
        });
    };

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
        crop,
    })
}

/// Decodes the upload and resizes it to the working size.
///
/// Returns the region of the source that was read, for the `x-crop-rect` header. It comes from the pipeline's own
/// [`resize::fitted_rect`] rather than being worked out again here, so what the header reports cannot drift from what
/// the resize did.
fn prepare(source: &[u8], params: DitherParams) -> Result<(RgbImage, String), ApiError> {
    let photo = io::decode_rgb(source).map_err(|e| ApiError::bad_request(format!("could not read the image: {e}")))?;

    let (width, height) = photo.dimensions();
    if u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS {
        return Err(ApiError::bad_request(format!(
            "the image is {width}x{height}, over the {MAX_SOURCE_PIXELS} pixel limit"
        )));
    }

    let fit = params.fit();

    // The shape the framing is measured against, which is the working size itself when scaling to it.
    let target = params.working_size().unwrap_or_else(|| params.working_ratio());
    let (x, y, kept_w, kept_h) = resize::fitted_rect((width, height), target, fit);

    let working = match params.resize {
        Resize::Fit => resize::resize_to_fit(&photo, target, fit),
        Resize::Factor(factor) => resize::scale_to_fit(&photo, target, fit, factor),
        // Keeping the source pixels is no reason to stop framing them.
        Resize::Keep if fit.crop => resize::crop_to_fit(&photo, target, fit),
        Resize::Keep => photo,
    };

    Ok((working, format!("{x},{y},{kept_w},{kept_h}")))
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
