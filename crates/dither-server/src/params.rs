//! Query parameters accepted by the dithering endpoints.
//!
//! Every field has a default, so `POST /api/dither` with no query string runs
//! the camera's own settings. The names here are also what `GET /api/options`
//! reports, so a client can round-trip its defaults straight back into a query
//! string.

use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use reframe_dither::{
    ATKINSON, BURKES, BayerSize, DitherMethod, DitherOptions, FLOYD_STEINBERG, FitOptions, JARVIS_JUDICE_NINKE, STUCKI,
};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;

/// Largest working width or height a request may ask for.
pub const MAX_DIMENSION: u32 = 4096;
/// Largest nearest-neighbour upscale factor.
pub const MAX_SCALE: u32 = 4;
/// Largest source image accepted, as a guard against decompression bombs.
pub const MAX_SOURCE_PIXELS: u64 = 50_000_000;

/// Which dithering algorithm to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Method {
    #[serde(alias = "floyd_steinberg")]
    FloydSteinberg,
    Atkinson,
    Stucki,
    Burkes,
    Jarvis,
    Ordered,
}

impl Method {
    pub const ALL: [Method; 6] = [
        Method::FloydSteinberg,
        Method::Atkinson,
        Method::Stucki,
        Method::Burkes,
        Method::Jarvis,
        Method::Ordered,
    ];
}

impl From<Method> for DitherMethod {
    fn from(value: Method) -> Self {
        match value {
            Method::FloydSteinberg => DitherMethod::ErrorDiffusion(FLOYD_STEINBERG),
            Method::Atkinson => DitherMethod::ErrorDiffusion(ATKINSON),
            Method::Stucki => DitherMethod::ErrorDiffusion(STUCKI),
            Method::Burkes => DitherMethod::ErrorDiffusion(BURKES),
            Method::Jarvis => DitherMethod::ErrorDiffusion(JARVIS_JUDICE_NINKE),
            Method::Ordered => DitherMethod::Ordered,
        }
    }
}

/// How to encode the PNG that comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// Indexed PNG carrying the 6-colour palette, as the camera saves it.
    Indexed,
    /// Plain RGB PNG, for viewers that dislike palette images.
    Rgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DitherParams {
    pub method: Method,
    /// Blend between the pure and the muted panel palettes, 0.0 to 1.0.
    pub saturation: f64,
    /// Brightness multiplier applied before dithering.
    pub brightness: f64,
    /// Colour intensity multiplier applied before dithering.
    pub color: f64,
    /// Bayer matrix side. Ordered dithering only.
    pub bayer_size: u32,
    /// Scales the Bayer threshold amplitude. Ordered dithering only.
    pub threshold_scale: f64,
    /// Working width, used unless `resize` is false.
    pub width: u32,
    /// Working height, used unless `resize` is false.
    pub height: u32,
    /// Resize to `width`x`height` first. False dithers at the source resolution.
    pub resize: bool,
    /// Keep the photo's orientation: a portrait photo resizes to `height`x`width`.
    pub keep_orientation: bool,
    /// Crop to the working size's aspect ratio instead of stretching the photo into it.
    pub crop: bool,
    /// Nearest-neighbour upscale applied to the result, 1 to 4.
    pub scale: u32,
    pub format: Format,
}

impl Default for DitherParams {
    fn default() -> Self {
        let defaults = DitherOptions::default();
        Self {
            method: Method::FloydSteinberg,
            saturation: defaults.saturation,
            brightness: defaults.brightness_factor,
            color: defaults.color_factor,
            bayer_size: defaults.bayer_size.side() as u32,
            threshold_scale: defaults.threshold_scale,
            width: reframe_dither::DISPLAY_IMAGE_SIZE.0,
            height: reframe_dither::DISPLAY_IMAGE_SIZE.1,
            resize: true,
            keep_orientation: false,
            crop: false,
            scale: 1,
            format: Format::Indexed,
        }
    }
}

impl DitherParams {
    /// Checks the ranges and builds the pipeline's own options struct.
    pub fn to_options(self) -> Result<DitherOptions, ApiError> {
        ratio("saturation", self.saturation)?;
        factor("brightness", self.brightness)?;
        factor("color", self.color)?;
        factor("threshold_scale", self.threshold_scale)?;

        if !matches!(self.bayer_size, 2 | 4 | 8) {
            return Err(ApiError::bad_request(format!(
                "bayer_size must be 2, 4 or 8, got {}",
                self.bayer_size
            )));
        }

        dimension("width", self.width)?;
        dimension("height", self.height)?;

        if self.scale == 0 || self.scale > MAX_SCALE {
            return Err(ApiError::bad_request(format!(
                "scale must be between 1 and {MAX_SCALE}, got {}",
                self.scale
            )));
        }

        Ok(DitherOptions {
            saturation: self.saturation,
            brightness_factor: self.brightness,
            color_factor: self.color,
            method: self.method.into(),
            bayer_size: BayerSize::from_side_or_default(self.bayer_size),
            threshold_scale: self.threshold_scale,
        })
    }

    /// The working size, or `None` when the source resolution is kept.
    pub fn target_size(self) -> Option<(u32, u32)> {
        self.resize.then_some((self.width, self.height))
    }

    /// How a photo that does not share the working size's shape is fitted to it.
    pub fn fit(self) -> FitOptions {
        FitOptions {
            keep_orientation: self.keep_orientation,
            crop: self.crop,
        }
    }
}

/// [`DitherParams`] pulled from the query string.
///
/// A thin wrapper over [`Query`] so a bad query string fails with the API's own
/// JSON error shape instead of axum's plain text.
#[derive(Debug)]
pub struct Params(pub DitherParams);

impl<S: Send + Sync> FromRequestParts<S> for Params {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(params) = Query::<DitherParams>::from_request_parts(parts, state)
            .await
            .map_err(|e| ApiError::new(e.status(), e.body_text()))?;
        Ok(Params(params))
    }
}

fn ratio(name: &str, value: f64) -> Result<(), ApiError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(ApiError::bad_request(format!(
            "{name} must be between 0.0 and 1.0, got {value}"
        )));
    }
    Ok(())
}

fn factor(name: &str, value: f64) -> Result<(), ApiError> {
    if !value.is_finite() || !(0.0..=5.0).contains(&value) {
        return Err(ApiError::bad_request(format!(
            "{name} must be between 0.0 and 5.0, got {value}"
        )));
    }
    Ok(())
}

fn dimension(name: &str, value: u32) -> Result<(), ApiError> {
    if value == 0 || value > MAX_DIMENSION {
        return Err(ApiError::bad_request(format!(
            "{name} must be between 1 and {MAX_DIMENSION}, got {value}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_pipeline() {
        let options = DitherParams::default().to_options().expect("defaults are valid");
        assert_eq!(options, DitherOptions::default());
    }

    #[test]
    fn out_of_range_values_are_rejected() {
        let bad = [
            DitherParams {
                saturation: 1.5,
                ..Default::default()
            },
            DitherParams {
                bayer_size: 3,
                ..Default::default()
            },
            DitherParams {
                width: 0,
                ..Default::default()
            },
            DitherParams {
                scale: 9,
                ..Default::default()
            },
            DitherParams {
                brightness: f64::NAN,
                ..Default::default()
            },
        ];
        for params in bad {
            assert!(params.to_options().is_err(), "{params:?} should be rejected");
        }
    }

    #[test]
    fn the_fitting_flags_reach_the_pipeline() {
        let params = DitherParams::default();
        assert_eq!(params.target_size(), Some((600, 400)));
        assert_eq!(params.fit(), FitOptions::default());

        let params = DitherParams {
            keep_orientation: true,
            crop: true,
            ..Default::default()
        };
        assert_eq!(
            params.fit(),
            FitOptions {
                keep_orientation: true,
                crop: true
            }
        );

        // `resize=false` drops the working size, and the flags then have nothing to act on.
        let params = DitherParams {
            resize: false,
            ..params
        };
        assert_eq!(params.target_size(), None);
    }

    #[test]
    fn method_names_parse_the_way_the_api_reports_them() {
        for method in Method::ALL {
            let name = serde_json::to_string(&method).expect("method serialises");
            let parsed: Method = serde_json::from_str(&name).expect("method parses back");
            assert_eq!(method, parsed);
        }
    }
}
