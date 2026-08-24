//! What the controls hold, and how it turns into the pipeline's own options.
//!
//! The settings are split in two because the pipeline is: [`Geometry`] decides
//! which pixels are kept and at what size, [`Look`] decides what colour each
//! one ends up. Only the first stage is expensive, so keeping the halves apart
//! lets the app re-run the cheap one on its own while a colour slider moves.

use dither_core::{
    ATKINSON, BURKES, BayerSize, CropOrigin, DitherMethod, DitherOptions, FLOYD_STEINBERG, FitOptions,
    JARVIS_JUDICE_NINKE, STUCKI, working_size,
};

/// The largest scale factor the UI offers, matching the server's own ceiling.
pub const MAX_SCALE: u32 = 4;
/// The largest working dimension, matching the server's own ceiling.
pub const MAX_DIMENSION: u32 = 4096;

/// Which dithering algorithm runs, named as the HTTP API names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    FloydSteinberg,
    Atkinson,
    Stucki,
    Burkes,
    Jarvis,
    Ordered,
    /// No dithering: the photo comes back resized and cropped, and nothing else.
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

    /// The wire name, which is also the `<option>` value.
    pub fn slug(self) -> &'static str {
        match self {
            Method::FloydSteinberg => "floyd-steinberg",
            Method::Atkinson => "atkinson",
            Method::Stucki => "stucki",
            Method::Burkes => "burkes",
            Method::Jarvis => "jarvis",
            Method::Ordered => "ordered",
            Method::None => "none",
        }
    }

    /// What the dropdown shows.
    pub fn label(self) -> &'static str {
        match self {
            Method::FloydSteinberg => "Floyd-Steinberg",
            Method::Atkinson => "Atkinson",
            Method::Stucki => "Stucki",
            Method::Burkes => "Burkes",
            Method::Jarvis => "Jarvis-Judice-Ninke",
            Method::Ordered => "Ordered (Bayer)",
            Method::None => "None (resize only)",
        }
    }

    /// Reads back what [`Method::slug`] wrote, falling back to the default.
    pub fn from_slug(slug: &str) -> Self {
        Method::ALL
            .into_iter()
            .find(|method| method.slug() == slug)
            .unwrap_or(Method::FloydSteinberg)
    }

    /// The pipeline's own method, or `None` when the settings asked for the resize alone.
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

    /// Whether the Bayer controls have anything to act on.
    pub fn is_ordered(self) -> bool {
        self == Method::Ordered
    }
}

/// Where a crop is anchored. The UI offers the named anchors only.
///
/// [`CropOrigin::At`] is the sixth thing the pipeline accepts, and it needs a
/// pair of source-pixel coordinates rather than a menu, so it is left to the
/// HTTP API where a caller can name the corner it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor(pub CropOrigin);

impl Anchor {
    pub const ALL: [Anchor; 5] = [
        Anchor(CropOrigin::Center),
        Anchor(CropOrigin::Top),
        Anchor(CropOrigin::Bottom),
        Anchor(CropOrigin::Left),
        Anchor(CropOrigin::Right),
    ];

    pub fn slug(self) -> &'static str {
        match self.0 {
            CropOrigin::Center => "center",
            CropOrigin::Top => "top",
            CropOrigin::Bottom => "bottom",
            CropOrigin::Left => "left",
            CropOrigin::Right => "right",
            // Not offered, so never written.
            CropOrigin::At { .. } => "center",
        }
    }

    pub fn label(self) -> &'static str {
        match self.0 {
            CropOrigin::Center => "Center",
            CropOrigin::Top => "Top",
            CropOrigin::Bottom => "Bottom",
            CropOrigin::Left => "Left",
            CropOrigin::Right => "Right",
            CropOrigin::At { .. } => "Center",
        }
    }

    pub fn from_slug(slug: &str) -> Self {
        Anchor::ALL
            .into_iter()
            .find(|anchor| anchor.slug() == slug)
            .unwrap_or(Anchor(CropOrigin::Center))
    }
}

/// How much smaller the photo comes back, spelled as the API spells it.
///
/// Three answers to one question: scale to the working size, keep the source
/// resolution, or take a fraction of what the framing kept. The framing itself
/// is a separate question, so `crop` still decides the shape underneath any of
/// them and this governs the scaling alone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resize {
    /// Scale to `width` x `height`.
    Fit,
    /// Keep the source resolution: framed, but not scaled.
    Keep,
    /// A fraction of each side of what the framing kept.
    Factor(f64),
}

impl Resize {
    /// The two named modes, then a halving ladder down to an eighth. `Keep` leads, since it is the default.
    pub const ALL: [Resize; 5] = [
        Resize::Keep,
        Resize::Fit,
        Resize::Factor(0.5),
        Resize::Factor(0.25),
        Resize::Factor(0.125),
    ];

    /// The wire spelling, which is what the query string carries too.
    pub fn slug(self) -> String {
        match self {
            Resize::Fit => "true".to_string(),
            Resize::Keep => "false".to_string(),
            Resize::Factor(factor) => factor.to_string(),
        }
    }

    /// What the dropdown shows. Matched on the spelling rather than the float,
    /// so no two of these ever compare equal by accident.
    pub fn label(self) -> &'static str {
        match self.slug().as_str() {
            "true" => "Scale to size",
            "false" => "Photo's own size",
            "0.5" => "Half (0.5)",
            "0.25" => "Quarter (0.25)",
            "0.125" => "Eighth (0.125)",
            _ => "Fraction",
        }
    }

    /// Reads back what [`Resize::slug`] wrote, the way the API's deserialiser does.
    pub fn from_slug(slug: &str) -> Self {
        match slug.trim() {
            "true" => Resize::Fit,
            "false" => Resize::Keep,
            number => number.parse().map_or(Resize::Fit, Resize::Factor),
        }
    }

    /// Whether `width` x `height` names a size rather than just a shape.
    pub fn names_a_size(self) -> bool {
        self == Resize::Fit
    }
}

/// Which pixels survive, and at what size.
///
/// Changing any of this re-runs the resample, which is the expensive stage. The
/// scale factor is deliberately not here: it is applied after the dither, so it
/// belongs to the cheap stage instead.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    /// The working size, once a photo has been loaded to seed it. `None` only before that.
    pub size: Option<(u32, u32)>,
    pub resize: Resize,
    pub keep_orientation: bool,
    pub crop: bool,
    pub crop_from: Anchor,
    pub crop_zoom: f32,
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            size: None,
            resize: Resize::Keep,
            keep_orientation: false,
            crop: false,
            crop_from: Anchor(CropOrigin::Center),
            crop_zoom: 1.0,
        }
    }
}

impl Geometry {
    /// The working size when `resize` scales to it, and otherwise the shape the
    /// crop is measured against. The pipeline reads it as one or the other
    /// depending on which resize function it is handed to.
    ///
    /// The rule itself is [`working_size`], which the CLI and the HTTP backend
    /// call too, so what this app does with a size cannot drift from what they
    /// do with the same one.
    pub fn target(self, source: (u32, u32)) -> (u32, u32) {
        working_size(source, self.size, None)
    }

    /// The pair the number fields show: the working size, or the photo's own
    /// until one has been picked.
    pub fn sides(self, source: (u32, u32)) -> (u32, u32) {
        self.size.unwrap_or(source)
    }

    pub fn fit(self) -> FitOptions {
        FitOptions {
            keep_orientation: self.keep_orientation,
            crop: self.crop,
            crop_from: self.crop_from.0,
            crop_zoom: self.crop_zoom,
        }
    }
}

/// What colour each surviving pixel becomes.
///
/// Changing any of this re-runs the dither alone, on an image already cut down
/// to the working size, which is fast enough to keep up with a dragged slider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub method: Method,
    pub saturation: f64,
    pub brightness: f64,
    pub color: f64,
    pub bayer_size: BayerSize,
    pub threshold_scale: f64,
}

impl Default for Look {
    fn default() -> Self {
        let defaults = DitherOptions::default();
        Self {
            method: Method::FloydSteinberg,
            saturation: defaults.saturation,
            brightness: defaults.brightness_factor,
            color: defaults.color_factor,
            bayer_size: defaults.bayer_size,
            threshold_scale: defaults.threshold_scale,
        }
    }
}

impl Look {
    /// The pipeline's options, or `None` when the method asked for no dithering.
    pub fn options(self) -> Option<DitherOptions> {
        Some(DitherOptions {
            saturation: self.saturation,
            brightness_factor: self.brightness,
            color_factor: self.color,
            method: self.method.dither()?,
            bayer_size: self.bayer_size,
            threshold_scale: self.threshold_scale,
        })
    }
}

/// Reads a Bayer side length back into the pipeline's enum.
pub fn bayer_from_side(side: u32) -> BayerSize {
    match side {
        2 => BayerSize::Two,
        8 => BayerSize::Eight,
        _ => BayerSize::Four,
    }
}
