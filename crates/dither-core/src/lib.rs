//! A dithering pipeline: it reduces a full-colour photo to a fixed palette, a
//! handful of colours reused for every pixel.
//!
//! The palette is the Spectra 6 one, seven slots blended from six inks, and
//! [`display`] can pack the result into that panel's frame buffer for anyone
//! who has the hardware. Nothing above that stage knows about it.
//!
//! The idea comes from [kaloyaan/reframe](https://github.com/kaloyaan/reframe),
//! a camera that dithers to the same palette in Python, and the blend and the
//! buffer layout follow it. The rest, framing and kernels and the descent to the
//! working size, is this crate's own.
//!
//! # Pipeline
//!
//! 1. Resize to the panel's 600x400 landscape working size ([`resize`]). [`FitOptions`] can keep a portrait photo in
//!    the 400x600 transpose instead, and crop rather than stretch whatever ratio is left over.
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
    CropOrigin, DISPLAY_IMAGE_SIZE, FitOptions, MAX_CROP_ZOOM, RATIO_PRESETS, cover_rect, fitted_rect, fitted_size,
    orient_target, preset_names, preset_ratio, ratio_size, resize_cropped, resize_image, resize_to_fit, scale_nearest,
    scale_to_fit,
};

/// Re-exported so callers can build inputs without depending on `image` directly.
pub use image::RgbImage;
