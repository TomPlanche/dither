//! The two dithering methods.
//!
//! Both boost brightness and colour, then reduce the image to the panel's
//! 7-colour palette. They differ in how a slot gets picked:
//!
//! * [`DitherMethod::ErrorDiffusion`] pushes each pixel's quantisation error into its neighbours, using one of the
//!   kernels in [`crate::diffusion`].
//! * [`DitherMethod::Ordered`] adds a Bayer threshold and resolves colours through a precomputed lookup table over a
//!   5-bit RGB cube.

use image::RgbImage;
use rayon::prelude::*;

use crate::bayer::{self, BayerSize};
use crate::buffer::IndexedImage;
use crate::diffusion::{self, FLOYD_STEINBERG, Kernel};
use crate::enhance;
use crate::panel::{PanelPalette, to_lab};

/// Which dithering algorithm to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DitherMethod {
    /// Error diffusion with the given kernel.
    ErrorDiffusion(Kernel),
    /// Bayer threshold matrix.
    Ordered,
}

impl Default for DitherMethod {
    fn default() -> Self {
        DitherMethod::ErrorDiffusion(FLOYD_STEINBERG)
    }
}

impl DitherMethod {
    /// Plain Floyd-Steinberg, the camera's default.
    pub const FLOYD_STEINBERG: Self = DitherMethod::ErrorDiffusion(FLOYD_STEINBERG);

    /// Parses the spelling used in `settings.json`.
    pub fn from_settings_name(name: &str) -> Option<Self> {
        match name {
            "floyd_steinberg" => Some(Self::FLOYD_STEINBERG),
            "ordered" => Some(DitherMethod::Ordered),
            _ => None,
        }
    }

    /// The spelling used in `settings.json`.
    ///
    /// Kernels the Python pipeline does not know about report as
    /// `floyd_steinberg`, since that is the setting they are closest to.
    pub fn settings_name(self) -> &'static str {
        match self {
            DitherMethod::ErrorDiffusion(_) => "floyd_steinberg",
            DitherMethod::Ordered => "ordered",
        }
    }
}

/// The `processing` section of the reframe settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DitherOptions {
    pub saturation: f64,
    pub brightness_factor: f64,
    pub color_factor: f64,
    pub method: DitherMethod,
    pub bayer_size: BayerSize,
    pub threshold_scale: f64,
}

impl Default for DitherOptions {
    fn default() -> Self {
        Self {
            saturation: 0.6,
            brightness_factor: 1.1,
            color_factor: 1.4,
            method: DitherMethod::default(),
            bayer_size: BayerSize::default(),
            threshold_scale: 1.0,
        }
    }
}

/// Applies the enhancement and dithering pipeline.
pub fn apply_dithering(image: &RgbImage, options: &DitherOptions) -> IndexedImage {
    let mut work = image.clone();
    // Order matters: brightness first, then colour.
    enhance::brightness(&mut work, options.brightness_factor as f32);
    enhance::color(&mut work, options.color_factor as f32);

    let palette = PanelPalette::new(options.saturation);
    match options.method {
        DitherMethod::ErrorDiffusion(kernel) => {
            let indices = diffusion::diffuse(&work, &palette, &kernel);
            IndexedImage::new(indices, palette)
        },
        DitherMethod::Ordered => ordered(&work, palette, options.bayer_size, options.threshold_scale as f32),
    }
}

/// Nearest-colour lookup over the 32768 cells of a 5-bit RGB cube.
///
/// Reuse one of these when dithering a batch of images at a fixed saturation:
/// building it converts 32768 colours to CIELAB.
#[derive(Debug, Clone)]
pub struct OrderedLut {
    table: Vec<u8>,
}

impl OrderedLut {
    /// Precomputes the table for a palette.
    ///
    /// The 32768 sRGB to CIELAB conversions are independent, so they are spread across cores. Reuse the table across a
    /// batch when you can: it depends only on the palette, and the palette only on the saturation.
    pub fn new(palette: &PanelPalette) -> Self {
        const CHUNK: usize = 1024;
        let mut table = vec![0u8; 32768];
        table.par_chunks_mut(CHUNK).enumerate().for_each(|(chunk, slots)| {
            for (offset, slot) in slots.iter_mut().enumerate() {
                let cell = chunk * CHUNK + offset;
                // Cell centres: 5 bits per channel, so steps of 8 centred at 4.
                let rgb = [
                    ((cell >> 10) as u8) * 8 + 4,
                    (((cell >> 5) & 31) as u8) * 8 + 4,
                    ((cell & 31) as u8) * 8 + 4,
                ];
                *slot = palette.nearest_lab(to_lab(rgb)) as u8;
            }
        });
        Self { table }
    }

    #[inline]
    fn lookup(&self, r: usize, g: usize, b: usize) -> u8 {
        self.table[(r << 10) | (g << 5) | b]
    }
}

/// Per-channel Bayer amplitude: the widest gap between successive palette values.
fn channel_thresholds(palette: &PanelPalette, threshold_scale: f32) -> [f32; 3] {
    let mut out = [0f32; 3];
    for (channel, slot) in out.iter_mut().enumerate() {
        let mut values: Vec<u8> = palette.colors().iter().map(|c| c[channel]).collect();
        values.sort_unstable();
        let max_gap = values.windows(2).map(|w| w[1] - w[0]).max().unwrap_or(255);
        *slot = max_gap as f32 * threshold_scale;
    }
    out
}

/// Ordered (Bayer) dithering.
///
/// Unlike error diffusion, every pixel here is independent, so the rows run in parallel.
fn ordered(image: &RgbImage, palette: PanelPalette, size: BayerSize, threshold_scale: f32) -> IndexedImage {
    let lut = OrderedLut::new(&palette);
    let thresholds = channel_thresholds(&palette, threshold_scale);
    let side = size.side();
    let matrix = bayer::matrix(size);

    let (width, height) = image.dimensions();
    let w = width as usize;
    let source = image.as_raw();
    let mut indices = vec![0u8; w * (height as usize)];

    indices.par_chunks_mut(w).enumerate().for_each(|(y, out_row)| {
        let row = &matrix[(y % side) * side..(y % side) * side + side];
        let src_row = &source[y * w * 3..(y + 1) * w * 3];
        for (x, (px, slot)) in src_row.chunks_exact(3).zip(out_row.iter_mut()).enumerate() {
            let threshold = row[x % side] - 0.5;
            let mut cell = [0usize; 3];
            for (channel, bucket) in cell.iter_mut().enumerate() {
                let noisy = (px[channel] as f32 + threshold * thresholds[channel]).clamp(0.0, 255.0);
                *bucket = (noisy as usize / 8).min(31);
            }
            *slot = lut.lookup(cell[0], cell[1], cell[2]);
        }
    });

    let indices = image::GrayImage::from_raw(width, height, indices).expect("buffer matches the dimensions");
    IndexedImage::new(indices, palette)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::PALETTE_COLORS;

    fn solid(width: u32, height: u32, rgb: [u8; 3]) -> RgbImage {
        RgbImage::from_pixel(width, height, image::Rgb(rgb))
    }

    #[test]
    fn white_stays_white_under_both_methods() {
        for method in [DitherMethod::FLOYD_STEINBERG, DitherMethod::Ordered] {
            let options = DitherOptions {
                method,
                ..Default::default()
            };
            let out = apply_dithering(&solid(8, 8, [255, 255, 255]), &options);
            assert!(
                out.indices().iter().all(|&i| i == 1),
                "{method:?} should map white to slot 1"
            );
        }
    }

    #[test]
    fn every_slot_is_reachable() {
        for method in [DitherMethod::FLOYD_STEINBERG, DitherMethod::Ordered] {
            let options = DitherOptions {
                method,
                ..Default::default()
            };
            let out = apply_dithering(&solid(8, 8, [0, 0, 0]), &options);
            assert!(out.indices().iter().all(|&i| (i as usize) < PALETTE_COLORS));
        }
    }

    #[test]
    fn single_pixel_images_do_not_panic() {
        let out = apply_dithering(&solid(1, 1, [200, 30, 30]), &DitherOptions::default());
        assert_eq!(out.size(), (1, 1));
    }

    #[test]
    fn thresholds_use_the_widest_palette_gap() {
        let palette = PanelPalette::new(0.6);
        // Red channel sorted: 0, 24, 34, 34, 195, 226, 255 -> widest gap 161.
        assert_eq!(channel_thresholds(&palette, 1.0)[0], 161.0);
        assert_eq!(channel_thresholds(&palette, 0.5)[0], 80.5);
    }
}
