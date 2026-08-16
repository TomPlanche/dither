//! Error-diffusion dithering.
//!
//! This is the one piece not delegated to a crate. `image::imageops::dither`
//! clamps the accumulated error back into a `u8` after every step, which throws
//! away most of it on a palette this small and visibly washes the output out.
//! The `dither` crate gets the arithmetic right but hard-depends on a beta
//! `clap` and an old `image`, so it cannot be used alongside `image` 0.25.
//!
//! The loop below keeps the running error in `f32`, as Pillow keeps it in `i32`.
//! Quantisation itself goes through [`PanelPalette`], which is an
//! `image::imageops::ColorMap`.

use image::{GrayImage, Luma, RgbImage};

use crate::panel::PanelPalette;

/// An error-diffusion kernel: where to push the error, and by how much.
///
/// Offsets are `(dx, dy, weight)` and are scaled by `divisor`. Only forward
/// neighbours appear, since the image is traversed left to right, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kernel {
    pub name: &'static str,
    pub offsets: &'static [(i32, i32, i32)],
    pub divisor: i32,
}

/// The classic. Sharp, and the default here.
pub const FLOYD_STEINBERG: Kernel = Kernel {
    name: "floyd-steinberg",
    offsets: &[(1, 0, 7), (-1, 1, 3), (0, 1, 5), (1, 1, 1)],
    divisor: 16,
};

/// Discards some error on purpose, giving cleaner highlights and more contrast.
pub const ATKINSON: Kernel = Kernel {
    name: "atkinson",
    offsets: &[(1, 0, 1), (2, 0, 1), (-1, 1, 1), (0, 1, 1), (1, 1, 1), (0, 2, 1)],
    divisor: 8,
};

/// Wider spread than Floyd-Steinberg, so smoother gradients.
pub const STUCKI: Kernel = Kernel {
    name: "stucki",
    offsets: &[
        (1, 0, 8),
        (2, 0, 4),
        (-2, 1, 2),
        (-1, 1, 4),
        (0, 1, 8),
        (1, 1, 4),
        (2, 1, 2),
        (-2, 2, 1),
        (-1, 2, 2),
        (0, 2, 4),
        (1, 2, 2),
        (2, 2, 1),
    ],
    divisor: 42,
};

/// Like Stucki but cheaper, with a slightly grainier look.
pub const BURKES: Kernel = Kernel {
    name: "burkes",
    offsets: &[
        (1, 0, 8),
        (2, 0, 4),
        (-2, 1, 2),
        (-1, 1, 4),
        (0, 1, 8),
        (1, 1, 4),
        (2, 1, 2),
    ],
    divisor: 32,
};

/// The widest of the classic kernels, and the smoothest.
pub const JARVIS_JUDICE_NINKE: Kernel = Kernel {
    name: "jarvis",
    offsets: &[
        (1, 0, 7),
        (2, 0, 5),
        (-2, 1, 3),
        (-1, 1, 5),
        (0, 1, 7),
        (1, 1, 5),
        (2, 1, 3),
        (-2, 2, 1),
        (-1, 2, 3),
        (0, 2, 5),
        (1, 2, 3),
        (2, 2, 1),
    ],
    divisor: 48,
};

/// Every kernel, in the order the CLI lists them.
pub const KERNELS: [Kernel; 5] = [FLOYD_STEINBERG, ATKINSON, STUCKI, BURKES, JARVIS_JUDICE_NINKE];

/// Quantises to `palette`, diffusing the error with `kernel`.
///
/// Returns one palette slot per pixel.
///
/// The error buffer covers the whole image rather than just the two or three rows a kernel can still reach. That looks
/// wasteful and is not: keeping only the live rows was tried three ways and every one measured slower, because the
/// window has to be addressed as a ring and the per-tap cost of that outweighs the locality it buys. See BENCHMARKS.md,
/// entry 5.
pub fn diffuse(image: &RgbImage, palette: &PanelPalette, kernel: &Kernel) -> GrayImage {
    let (width, height) = image.dimensions();
    let mut spill = vec![[0f32; 3]; (width as usize) * (height as usize)];
    let mut indices = GrayImage::new(width, height);
    let divisor = kernel.divisor as f32;

    for y in 0..height {
        for x in 0..width {
            let here = (y as usize) * (width as usize) + (x as usize);
            let source = image.get_pixel(x, y).0;

            // Clamp before matching, so a large running error cannot chase the
            // search outside the palette's range.
            let mut wanted = [0u8; 3];
            for channel in 0..3 {
                let v = source[channel] as f32 + spill[here][channel];
                wanted[channel] = v.clamp(0.0, 255.0) as u8;
            }

            let slot = palette.nearest_rgb(wanted);
            indices.put_pixel(x, y, Luma([slot as u8]));

            let chosen = palette.colors()[slot];
            let error = [
                wanted[0] as f32 - chosen[0] as f32,
                wanted[1] as f32 - chosen[1] as f32,
                wanted[2] as f32 - chosen[2] as f32,
            ];

            for &(dx, dy, weight) in kernel.offsets {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || nx >= width as i32 || ny >= height as i32 {
                    continue;
                }
                let there = (ny as usize) * (width as usize) + (nx as usize);
                let share = weight as f32 / divisor;
                for channel in 0..3 {
                    spill[there][channel] += error[channel] * share;
                }
            }
        }
    }

    indices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernels_only_push_error_forward() {
        for kernel in KERNELS {
            for &(dx, dy, weight) in kernel.offsets {
                assert!(dy > 0 || (dy == 0 && dx > 0), "{} looks backwards", kernel.name);
                assert!(weight > 0, "{} has a non-positive weight", kernel.name);
            }
        }
    }

    #[test]
    fn floyd_steinberg_weights_sum_to_its_divisor() {
        // Atkinson deliberately discards error, so it is excluded here.
        for kernel in [FLOYD_STEINBERG, STUCKI, BURKES, JARVIS_JUDICE_NINKE] {
            let total: i32 = kernel.offsets.iter().map(|(_, _, w)| w).sum();
            assert_eq!(total, kernel.divisor, "{} does not conserve error", kernel.name);
        }
        let atkinson: i32 = ATKINSON.offsets.iter().map(|(_, _, w)| w).sum();
        assert!(atkinson < ATKINSON.divisor, "atkinson should shed error");
    }

    #[test]
    fn a_flat_midtone_uses_more_than_one_slot() {
        let image = RgbImage::from_pixel(32, 32, image::Rgb([128, 128, 128]));
        let palette = PanelPalette::new(0.6);
        let indices = diffuse(&image, &palette, &FLOYD_STEINBERG);
        let mut slots: Vec<u8> = indices.as_raw().to_vec();
        slots.sort_unstable();
        slots.dedup();
        assert!(slots.len() > 1, "a midtone should dither, got {slots:?}");
    }
}
