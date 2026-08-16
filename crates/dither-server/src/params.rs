//! Query parameters accepted by the dithering endpoints.
//!
//! Every field has a default, so `POST /api/dither` with no query string runs
//! the pipeline's own defaults. The names here are also what `GET /api/options`
//! reports, so a client can round-trip its defaults straight back into a query
//! string.

use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use reframe_dither::{
    ATKINSON, BURKES, BayerSize, CropOrigin, DitherMethod, DitherOptions, FLOYD_STEINBERG, FitOptions,
    JARVIS_JUDICE_NINKE, MAX_CROP_ZOOM, STUCKI,
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
    /// No dithering: the photo comes back resized and cropped, and nothing else.
    ///
    /// For checking the framing, where the dither pattern is in the way. The palette settings and `format` do not
    /// apply, since there is no palette: the result is always a plain RGB PNG.
    None,
}

impl Method {
    pub const ALL: [Method; 7] = [
        Method::FloydSteinberg,
        Method::Atkinson,
        Method::Stucki,
        Method::Burkes,
        Method::Jarvis,
        Method::Ordered,
        Method::None,
    ];

    /// The pipeline's own method, or `None` when the request asked for the resize alone.
    pub fn dither(self) -> Option<DitherMethod> {
        Some(match self {
            Method::FloydSteinberg => DitherMethod::ErrorDiffusion(FLOYD_STEINBERG),
            Method::Atkinson => DitherMethod::ErrorDiffusion(ATKINSON),
            Method::Stucki => DitherMethod::ErrorDiffusion(STUCKI),
            Method::Burkes => DitherMethod::ErrorDiffusion(BURKES),
            Method::Jarvis => DitherMethod::ErrorDiffusion(JARVIS_JUDICE_NINKE),
            Method::Ordered => DitherMethod::Ordered,
            Method::None => return Option::None,
        })
    }
}

/// An aspect ratio that goes by name.
///
/// The variants mirror [`reframe_dither::RATIO_PRESETS`], which is where the ratios come from. Serde does the
/// validating: an unknown name is refused with the list of the ones that work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    Panel,
    PanelPortrait,
    InstagramPost,
    InstagramPortrait,
    InstagramLandscape,
    InstagramStory,
    Iphone,
}

impl Preset {
    pub const ALL: [Preset; 7] = [
        Preset::Panel,
        Preset::PanelPortrait,
        Preset::InstagramPost,
        Preset::InstagramPortrait,
        Preset::InstagramLandscape,
        Preset::InstagramStory,
        Preset::Iphone,
    ];

    /// The name this goes by, in both the query string and the pipeline's own table.
    pub fn name(self) -> &'static str {
        match self {
            Preset::Panel => "panel",
            Preset::PanelPortrait => "panel-portrait",
            Preset::InstagramPost => "instagram-post",
            Preset::InstagramPortrait => "instagram-portrait",
            Preset::InstagramLandscape => "instagram-landscape",
            Preset::InstagramStory => "instagram-story",
            Preset::Iphone => "iphone",
        }
    }

    /// The aspect ratio it names, as `width:height`.
    pub fn ratio(self) -> (u32, u32) {
        reframe_dither::preset_ratio(self.name()).expect("every preset names a ratio the pipeline knows")
    }
}

/// What `resize` asks for.
///
/// Three answers to one question, which is how much smaller the photo should come back: the working size, nothing at
/// all, or a fraction of what the framing kept. Whichever it is, `crop` still decides the shape, so `resize` governs
/// the scaling alone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resize {
    /// `true`: scale to `width`x`height`, reshaped by any `preset`.
    Fit,
    /// `false`: keep the source resolution.
    Keep,
    /// `0.75`: three quarters of each side of what the framing kept, so a quarter off the photo.
    Factor(f64),
}

impl Resize {
    /// The fraction it asks for, or `None` when it names a size instead.
    pub fn factor(self) -> Option<f64> {
        match self {
            Resize::Factor(factor) => Some(factor),
            _ => None,
        }
    }
}

impl Serialize for Resize {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Resize::Fit => serializer.serialize_bool(true),
            Resize::Keep => serializer.serialize_bool(false),
            Resize::Factor(factor) => serializer.serialize_f64(*factor),
        }
    }
}

impl<'de> Deserialize<'de> for Resize {
    /// Reads `true`, `false` or a number, and the same three spelled as the strings a query string carries.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct AnyResize;

        impl serde::de::Visitor<'_> for AnyResize {
            type Value = Resize;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("true, false, or a fraction between 0 and 1")
            }

            fn visit_bool<E>(self, yes: bool) -> Result<Resize, E> {
                Ok(if yes { Resize::Fit } else { Resize::Keep })
            }

            fn visit_f64<E>(self, factor: f64) -> Result<Resize, E> {
                Ok(Resize::Factor(factor))
            }

            fn visit_u64<E>(self, factor: u64) -> Result<Resize, E> {
                Ok(Resize::Factor(factor as f64))
            }

            fn visit_i64<E>(self, factor: i64) -> Result<Resize, E> {
                Ok(Resize::Factor(factor as f64))
            }

            fn visit_str<E: serde::de::Error>(self, raw: &str) -> Result<Resize, E> {
                match raw.trim() {
                    "true" => Ok(Resize::Fit),
                    "false" => Ok(Resize::Keep),
                    number => number
                        .parse()
                        .map(Resize::Factor)
                        .map_err(|_| E::custom(format!("resize must be true, false or a number, got `{raw}`"))),
                }
            }
        }

        deserializer.deserialize_any(AnyResize)
    }
}

/// How to encode the PNG that comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// Indexed PNG carrying the palette, which is what the panel wants.
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
    /// Working width, used unless `resize` is false. A `preset` reshapes it rather than replacing it.
    pub width: u32,
    /// Working height, used unless `resize` is false. A `preset` reshapes it rather than replacing it.
    pub height: u32,
    /// A named aspect ratio, fitted inside `width`x`height`.
    ///
    /// It picks the shape and the pair still picks the scale, so a request says how many pixels it wants dithered
    /// whichever preset it names.
    ///
    /// Left out of `GET /api/options` when unset, where the `presets` list says what the names are instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<Preset>,
    /// What to scale the photo to: `true` for the working size, `false` for none, or a fraction of its own size.
    pub resize: Resize,
    /// Keep the photo's orientation: a portrait photo resizes to `height`x`width`.
    pub keep_orientation: bool,
    /// Crop to the working size's aspect ratio instead of stretching the photo into it.
    pub crop: bool,
    /// Which part the crop keeps: `center`, `top`, `bottom`, `left`, `right`, or a corner as `X,Y`.
    ///
    /// Refused without `crop`, since there would be nothing for it to place. Left out of `GET /api/options` when
    /// unset, for the same reason: sending the defaults back unchanged has to stay a valid request.
    #[serde(default, with = "crop_from", skip_serializing_if = "Option::is_none")]
    pub crop_from: Option<CropOrigin>,
    /// How far into the photo the crop moves, `1.0` to `10.0`.
    ///
    /// At `1.0` the kept rectangle touches two opposite edges, so `crop_from` can only slide it along one axis. Above
    /// that it keeps a proportionally smaller rectangle, which frees the other axis too. Refused without `crop`, and
    /// left out of `GET /api/options` when unset, like `crop_from`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop_zoom: Option<f64>,
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
            preset: None,
            resize: Resize::Fit,
            keep_orientation: false,
            crop: false,
            crop_from: None,
            crop_zoom: None,
            scale: 1,
            format: Format::Indexed,
        }
    }
}

impl DitherParams {
    /// Checks the ranges and builds the pipeline's own options struct.
    ///
    /// `None` when `method=none` asked for the resize alone, in which case the palette settings are checked but go
    /// unused, the same way `bayer_size` does under error diffusion.
    pub fn to_options(self) -> Result<Option<DitherOptions>, ApiError> {
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

        // A setting that cannot take effect is a mistake worth naming, the way an unknown parameter is.
        let unusable = [
            ("crop_from", self.crop_from.is_some()),
            ("crop_zoom", self.crop_zoom.is_some()),
        ]
        .into_iter()
        .find_map(|(name, given)| (given && !self.crop).then_some(name));
        if let Some(name) = unusable {
            return Err(ApiError::bad_request(format!(
                "{name} needs crop=true, which is the crop it places"
            )));
        }

        if let Some(factor) = self.resize.factor()
            && (!factor.is_finite() || !(0.0..=1.0).contains(&factor) || factor == 0.0)
        {
            return Err(ApiError::bad_request(format!(
                "resize must be true, false, or a fraction between 0 and 1, got {factor}"
            )));
        }

        if let Some(zoom) = self.crop_zoom
            && (!zoom.is_finite() || !(1.0..=f64::from(MAX_CROP_ZOOM)).contains(&zoom))
        {
            return Err(ApiError::bad_request(format!(
                "crop_zoom must be between 1.0 and {MAX_CROP_ZOOM}, got {zoom}"
            )));
        }

        if self.scale == 0 || self.scale > MAX_SCALE {
            return Err(ApiError::bad_request(format!(
                "scale must be between 1 and {MAX_SCALE}, got {}",
                self.scale
            )));
        }

        Ok(self.method.dither().map(|method| DitherOptions {
            saturation: self.saturation,
            brightness_factor: self.brightness,
            color_factor: self.color,
            method,
            bayer_size: BayerSize::from_side_or_default(self.bayer_size),
            threshold_scale: self.threshold_scale,
        }))
    }

    /// The size to dither at, or `None` when the source resolution is kept.
    ///
    /// A preset reshapes `width`x`height` rather than replacing it: the largest rectangle of the preset's ratio that
    /// fits inside the pair, which is turned over first when the ratio disagrees with it. So `preset=panel-portrait`
    /// against the default 600x400 is the panel's own 400x600, and either way the result is never larger than the
    /// dimensions the request already had checked.
    pub fn working_size(self) -> Option<(u32, u32)> {
        matches!(self.resize, Resize::Fit)
            .then(|| reframe_dither::ratio_size((self.width, self.height), self.working_ratio()))
    }

    /// The shape the geometry is measured against, whatever `resize` says.
    ///
    /// `resize=false` keeps the source resolution, but a crop still needs a shape to aim at, and this is it: the
    /// preset's ratio, or `width`x`height` when no preset was named. So `resize=false` means no scaling rather than no
    /// framing, and `crop` keeps working underneath it.
    pub fn working_ratio(self) -> (u32, u32) {
        match self.preset {
            Some(preset) => preset.ratio(),
            None => (self.width, self.height),
        }
    }

    /// How a photo that does not share the working size's shape is fitted to it.
    pub fn fit(self) -> FitOptions {
        FitOptions {
            keep_orientation: self.keep_orientation,
            crop: self.crop,
            crop_from: self.crop_from.unwrap_or_default(),
            crop_zoom: self.crop_zoom.map_or(1.0, |zoom| zoom as f32),
        }
    }
}

/// `crop_from` as the one string the pipeline already parses.
///
/// The pipeline owns the syntax, so the query string, the CLI and `GET /api/options` all read and write the same
/// spelling, and an unusable one comes back as a 400 carrying the parser's own message.
mod crop_from {
    use std::str::FromStr;

    use reframe_dither::CropOrigin;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(origin: &Option<CropOrigin>, serializer: S) -> Result<S::Ok, S::Error> {
        match origin {
            Some(origin) => serializer.collect_str(origin),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<CropOrigin>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|raw| CropOrigin::from_str(&raw).map_err(serde::de::Error::custom))
            .transpose()
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
        assert_eq!(options, Some(DitherOptions::default()));
    }

    #[test]
    fn method_none_asks_for_the_resize_alone() {
        let params = DitherParams {
            method: Method::None,
            ..Default::default()
        };
        assert_eq!(params.to_options().expect("still valid"), None);
        assert_eq!(Method::None.dither(), None);

        // The palette settings are still checked, so a typo is caught whichever method is running.
        let bad = DitherParams {
            saturation: 9.0,
            ..params
        };
        assert!(bad.to_options().is_err());

        // Every other method has one.
        for method in Method::ALL.iter().filter(|m| **m != Method::None) {
            assert!(method.dither().is_some(), "{method:?} should name a dither");
        }
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
    fn every_preset_names_a_ratio_the_api_can_serve() {
        for preset in Preset::ALL {
            let (width, height) = preset.ratio();
            assert!(width > 0 && height > 0, "{} has a zero side", preset.name());

            // Whatever the ratio, what it resolves to is inside what `width` and `height` already had checked, so a
            // preset can never carry a request past the dimension limit.
            let params = DitherParams {
                preset: Some(preset),
                width: MAX_DIMENSION,
                height: MAX_DIMENSION,
                ..Default::default()
            };
            let (width, height) = params.working_size().expect("resize is on");
            assert!(
                (1..=MAX_DIMENSION).contains(&width) && (1..=MAX_DIMENSION).contains(&height),
                "{} is {width}x{height}, outside what the API accepts",
                preset.name()
            );

            // The name the query string uses is the one the pipeline's table is keyed by, and it parses back, so the
            // `presets` listing can never advertise a name the endpoints refuse.
            let quoted = serde_json::to_string(&preset).expect("preset serialises");
            assert_eq!(quoted.trim_matches('"'), preset.name());
            assert_eq!(
                serde_json::from_str::<Preset>(&quoted).expect("preset parses back"),
                preset
            );
        }

        assert_eq!(Preset::InstagramStory.ratio(), (9, 16));
        assert_eq!(Preset::Panel.ratio(), (3, 2));
    }

    #[test]
    fn a_preset_reshapes_width_and_height_rather_than_replacing_them() {
        let params = DitherParams {
            preset: Some(Preset::InstagramPortrait),
            width: 320,
            height: 240,
            ..Default::default()
        };
        // 4:5 turns the pair over and then fits inside it, so the scale is still the one that was asked for.
        assert_eq!(params.working_size(), Some((240, 300)));

        // Without one, the pair is used as it was sent.
        assert_eq!(DitherParams { preset: None, ..params }.working_size(), Some((320, 240)));

        // The panel entries against the default pair land on the layouts `/api/buffer` packs.
        let panel = |preset| {
            DitherParams {
                preset: Some(preset),
                ..Default::default()
            }
            .working_size()
        };
        assert_eq!(panel(Preset::Panel), Some(reframe_dither::DISPLAY_IMAGE_SIZE));
        assert_eq!(panel(Preset::PanelPortrait), Some(reframe_dither::DISPLAY_PANEL_SIZE));

        // And a bigger pair buys more pixels of the same shape.
        let bigger = DitherParams {
            preset: Some(Preset::InstagramStory),
            width: 1080,
            height: 1080,
            ..Default::default()
        };
        assert_eq!(bigger.working_size(), Some((607, 1080)));
    }

    #[test]
    fn the_fitting_flags_reach_the_pipeline() {
        let params = DitherParams::default();
        assert_eq!(params.working_size(), Some((600, 400)));
        assert_eq!(params.fit(), FitOptions::default());

        let params = DitherParams {
            keep_orientation: true,
            crop: true,
            crop_from: Some(CropOrigin::At { x: 40, y: 90 }),
            crop_zoom: Some(2.5),
            ..Default::default()
        };
        assert_eq!(
            params.fit(),
            FitOptions {
                keep_orientation: true,
                crop: true,
                crop_from: CropOrigin::At { x: 40, y: 90 },
                crop_zoom: 2.5,
            }
        );

        // Unset, the crop falls back to the middle at full size rather than to nothing.
        let params = DitherParams {
            crop_from: None,
            crop_zoom: None,
            ..params
        };
        assert_eq!(params.fit().crop_from, CropOrigin::Center);
        assert_eq!(params.fit().crop_zoom, 1.0);

        // Anything but `resize=true` drops the working size, and the framing then works off the shape alone.
        for resize in [Resize::Keep, Resize::Factor(0.75)] {
            let params = DitherParams { resize, ..params };
            assert_eq!(params.working_size(), None);
            assert_eq!(params.working_ratio(), (600, 400));
        }
    }

    #[test]
    fn resize_reads_a_flag_or_a_fraction() {
        // What a query string carries, and what `GET /api/options` reports back.
        for (raw, expected) in [
            ("true", Resize::Fit),
            ("false", Resize::Keep),
            ("0.75", Resize::Factor(0.75)),
            ("1", Resize::Factor(1.0)),
        ] {
            let params: DitherParams =
                serde_json::from_value(serde_json::json!({ "resize": raw })).expect("the query string form parses");
            assert_eq!(params.resize, expected, "`{raw}`");
            assert!(params.to_options().is_ok(), "`{raw}` should be accepted");
        }

        // And the JSON forms, for a client posting the defaults back as they came.
        assert_eq!(
            serde_json::from_value::<DitherParams>(serde_json::json!({ "resize": true }))
                .expect("a bool parses")
                .resize,
            Resize::Fit
        );
        assert_eq!(
            serde_json::to_value(DitherParams::default()).expect("defaults serialise")["resize"],
            serde_json::json!(true)
        );
        assert_eq!(
            serde_json::to_value(DitherParams {
                resize: Resize::Factor(0.75),
                ..Default::default()
            })
            .expect("a factor serialises")["resize"],
            serde_json::json!(0.75)
        );

        // A fraction outside what it can mean is refused, and so is a spelling that is neither.
        for bad in [0.0, -0.5, 1.5, f64::NAN] {
            let params = DitherParams {
                resize: Resize::Factor(bad),
                ..Default::default()
            };
            assert!(params.to_options().is_err(), "resize {bad} should be refused");
        }
        assert!(serde_json::from_value::<DitherParams>(serde_json::json!({ "resize": "half" })).is_err());
    }

    #[test]
    fn crop_from_survives_the_query_string_in_both_forms() {
        for origin in [
            CropOrigin::Center,
            CropOrigin::Top,
            CropOrigin::Bottom,
            CropOrigin::Left,
            CropOrigin::Right,
            CropOrigin::At { x: 120, y: 340 },
        ] {
            let params = DitherParams {
                crop: true,
                crop_from: Some(origin),
                ..Default::default()
            };

            // What `GET /api/options` reports is what the endpoints read back.
            let json = serde_json::to_value(params).expect("params serialise");
            assert_eq!(json["crop_from"], origin.to_string());
            let parsed: DitherParams = serde_json::from_value(json).expect("params parse back");
            assert_eq!(parsed.crop_from, Some(origin));
            assert!(parsed.to_options().is_ok(), "{origin} should be accepted with crop");
        }

        // Unset, it stays out of the reported defaults, so posting them back unchanged is still a valid request.
        let defaults = serde_json::to_value(DitherParams::default()).expect("defaults serialise");
        assert!(defaults.get("crop_from").is_none(), "{defaults}");

        // An unusable spelling is refused rather than falling back to the centre.
        let bad = serde_json::json!({ "crop_from": "middle" });
        let error = serde_json::from_value::<DitherParams>(bad).expect_err("`middle` should be refused");
        assert!(error.to_string().contains("center"), "{error}");
    }

    #[test]
    fn a_crop_setting_without_the_crop_is_refused_rather_than_ignored() {
        let alone = [
            DitherParams {
                crop_from: Some(CropOrigin::Top),
                ..Default::default()
            },
            DitherParams {
                crop_zoom: Some(2.0),
                ..Default::default()
            },
        ];

        for params in alone {
            // What the message says is checked over HTTP, where a client would read it.
            assert!(params.to_options().is_err(), "{params:?} should be refused");
            // The same setting with the crop on is fine.
            assert!(DitherParams { crop: true, ..params }.to_options().is_ok());
        }

        // And neither is fine too.
        assert!(DitherParams::default().to_options().is_ok());
    }

    #[test]
    fn the_crop_zoom_range_is_checked() {
        let zoomed = |zoom: f64| {
            DitherParams {
                crop: true,
                crop_zoom: Some(zoom),
                ..Default::default()
            }
            .to_options()
        };

        assert!(zoomed(1.0).is_ok());
        assert!(zoomed(f64::from(MAX_CROP_ZOOM)).is_ok());
        // Under 1.0 there is nothing left inside the photo to keep, and past the cap it is a stamp being blown up.
        for bad in [0.9, 0.0, -1.0, f64::from(MAX_CROP_ZOOM) + 0.1, f64::NAN] {
            assert!(zoomed(bad).is_err(), "crop_zoom {bad} should be refused");
        }
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
