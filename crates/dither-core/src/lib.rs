//! A dithering pipeline: it reduces a full-colour photo to a fixed palette, a
//! handful of colours reused for every pixel.
//!
//! The palette is six colours ([`palette`]), each blended between a pure
//! primary and a muted version of it. How far the blend goes is a setting, so
//! the same pipeline covers everything from poster-flat primaries to something
//! closer to ink on paper.
//!
//! # Pipeline
//!
//! 1. Resize to the working size ([`resize`]), 600x400 by default. [`FitOptions`] can keep a portrait photo in the
//!    transposed size instead, and crop rather than stretch whatever ratio is left over.
//! 2. Boost brightness, then colour ([`enhance`]).
//! 3. Dither to the 6-colour palette ([`dither`]), which yields an [`IndexedImage`].
//!
//! The heavy lifting is delegated: [`image`] provides the buffers, decoding,
//! resampling and rotation, and the `palette` crate provides the CIELAB colour
//! science behind the nearest-colour search. What is left here is the palette
//! and its saturation blend, the Bayer thresholds, and the error-diffusion loop
//! in [`diffusion`].
//!
//! # Example
//!
//! ```no_run
//! use dither_core::{DitherOptions, apply_dithering, io, resize};
//!
//! let photo = io::load_rgb("photo.jpg")?;
//! let sized = resize::resize_image(&photo, resize::DEFAULT_SIZE);
//! let dithered = apply_dithering(&sized, &DitherOptions::default());
//! io::save_indexed_png(&dithered, "photo_dithered.png")?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod bayer;
pub mod diffusion;
pub mod dither;
pub mod enhance;
pub mod indexed;
pub mod palette;
mod parallel;
pub mod resize;

#[cfg(feature = "image-io")]
pub mod io;

pub use bayer::BayerSize;
pub use diffusion::{ATKINSON, BURKES, FLOYD_STEINBERG, JARVIS_JUDICE_NINKE, KERNELS, Kernel, STUCKI};
pub use dither::{DitherMethod, DitherOptions, OrderedLut, apply_dithering};
pub use indexed::IndexedImage;
pub use palette::Palette;
pub use resize::{
    CropOrigin, DEFAULT_SIZE, FitOptions, MAX_CROP_ZOOM, RATIO_PRESETS, cover_rect, fitted_rect, fitted_size,
    orient_target, preset_names, preset_ratio, ratio_size, resize_cropped, resize_image, resize_to_fit, scale_nearest,
    scale_to_fit,
};

/// Re-exported so callers can build inputs without depending on `image` directly.
pub use image::RgbImage;
