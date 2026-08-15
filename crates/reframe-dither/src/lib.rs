//! A Rust port of the reframe camera's dithering pipeline.
//!
//! reframe photographs onto a Waveshare 4" Spectra 6 e-paper panel, which can
//! only show six colours. This crate reduces a full-colour photo to that palette
//! and packs the result into the panel's frame buffer.
//!
//! # Pipeline
//!
//! 1. Resize to the panel's 600x400 landscape working size ([`resize`]).
//!    [`FitOptions`] can keep a portrait photo in the 400x600 transpose
//!    instead, and crop rather than stretch whatever ratio is left over.
//! 2. Boost brightness, then colour ([`enhance`]).
//! 3. Dither to the 7-slot panel palette ([`dither`]).
//! 4. Rotate to portrait and pack two 4-bit codes per byte ([`display`]).
//!
//! The heavy lifting is delegated: [`image`] provides the buffers, decoding,
//! resampling and rotation, and [`palette`] provides the CIELAB colour science
//! behind the nearest-colour search. What is left here is specific to this
//! panel: its palette, the saturation blend, the Bayer thresholds, the frame
//! buffer layout, and the error-diffusion loop in [`diffusion`].
//!
//! # Example
//!
//! ```no_run
//! use reframe_dither::{DitherOptions, apply_dithering, io, resize};
//!
//! let photo = io::load_rgb("photo.jpg")?;
//! let sized = resize::resize_image(&photo, resize::DISPLAY_IMAGE_SIZE);
//! let dithered = apply_dithering(&sized, &DitherOptions::default());
//! io::save_indexed_png(&dithered, "photo_dithered.png")?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod bayer;
pub mod buffer;
pub mod diffusion;
pub mod display;
pub mod dither;
pub mod enhance;
pub mod panel;
pub mod resize;

#[cfg(feature = "image-io")]
pub mod io;

pub use bayer::BayerSize;
pub use buffer::IndexedImage;
pub use diffusion::{ATKINSON, BURKES, FLOYD_STEINBERG, JARVIS_JUDICE_NINKE, KERNELS, Kernel, STUCKI};
pub use display::{DISPLAY_PANEL_SIZE, Orientation, dither_to_display_buffer, img2buffer};
pub use dither::{DitherMethod, DitherOptions, OrderedLut, apply_dithering};
pub use panel::PanelPalette;
pub use resize::{
    DISPLAY_IMAGE_SIZE, FitOptions, SIZE_PRESETS, cover_rect, orient_target, preset_names, preset_size, resize_cropped,
    resize_image, resize_to_fit,
};

/// Re-exported so callers can build inputs without depending on `image` directly.
pub use image::RgbImage;
