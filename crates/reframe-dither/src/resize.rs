//! Resizing to the panel's working size.

use image::RgbImage;
use image::imageops::{self, FilterType};
use rayon::prelude::*;

/// The landscape size the pipeline dithers at.
pub const DISPLAY_IMAGE_SIZE: (u32, u32) = (600, 400);

/// Scales a photo to the working size.
///
/// A triangle filter widens its kernel when downscaling, so it behaves like an area average rather than point sampling.
/// Run alone on a 45 MP photo that costs more than the rest of the pipeline put together, because the kernel then spans
/// a hundred source pixels per output pixel.
///
/// So the descent happens in two steps. An integer box average takes the photo to within one whole factor of the
/// target, and the triangle filter covers the fractional remainder. Both passes area-average, which is why the result
/// stays within about one 8-bit level of the single-pass version, and Pillow reduces the same way behind
/// `reducing_gap`.
pub fn resize_image(image: &RgbImage, target: (u32, u32)) -> RgbImage {
    if image.dimensions() == target {
        return image.clone();
    }

    match prefilter_factor(image.dimensions(), target) {
        1 => imageops::resize(image, target.0, target.1, FilterType::Triangle),
        factor => {
            let reduced = box_reduce(image, factor);
            imageops::resize(&reduced, target.0, target.1, FilterType::Triangle)
        },
    }
}

/// The largest integer reduction that still leaves both sides at or above the target, so the triangle filter that
/// follows only ever downscales.
fn prefilter_factor(source: (u32, u32), target: (u32, u32)) -> u32 {
    let (w, h) = source;
    let (tw, th) = target;
    if tw == 0 || th == 0 {
        return 1;
    }
    (w / tw).min(h / th).max(1)
}

/// Averages whole `factor` x `factor` blocks of pixels.
///
/// The last few source rows and columns are dropped when the side is not a multiple of `factor`. That is at most
/// `factor - 1` pixels off an edge that is thousands wide, and it keeps every block the same weight.
fn box_reduce(image: &RgbImage, factor: u32) -> RgbImage {
    let f = factor as usize;
    let out_w = (image.width() as usize) / f;
    let out_h = (image.height() as usize) / f;
    let src_stride = (image.width() as usize) * 3;
    let src = image.as_raw();

    // Round to nearest rather than truncating, which halves the drift.
    let count = (f * f) as u32;
    let half = count / 2;

    // Output rows are independent, and each reads a disjoint band of the source.
    let mut out = vec![0u8; out_w * out_h * 3];
    out.par_chunks_mut(out_w * 3).enumerate().for_each(|(y, row)| {
        let block_top = y * f * src_stride;
        for (x, cell) in row.chunks_exact_mut(3).enumerate() {
            let block_left = x * f * 3;
            let mut acc = [0u32; 3];
            for by in 0..f {
                let start = block_top + by * src_stride + block_left;
                for px in src[start..start + f * 3].chunks_exact(3) {
                    acc[0] += px[0] as u32;
                    acc[1] += px[1] as u32;
                    acc[2] += px[2] as u32;
                }
            }
            cell[0] = ((acc[0] + half) / count) as u8;
            cell[1] = ((acc[1] + half) / count) as u8;
            cell[2] = ((acc[2] + half) / count) as u8;
        }
    });

    RgbImage::from_raw(out_w as u32, out_h as u32, out).expect("buffer matches the dimensions")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_to_the_same_size_is_a_no_op() {
        let image = RgbImage::from_fn(3, 2, |x, y| image::Rgb([x as u8, y as u8, 7]));
        assert_eq!(resize_image(&image, (3, 2)), image);
    }

    #[test]
    fn downscaling_averages_rather_than_point_samples() {
        // A black and white checkerboard must reduce to mid grey, not to one colour.
        let image = RgbImage::from_fn(64, 64, |x, y| {
            let v = if (x + y) % 2 == 0 { 0 } else { 255 };
            image::Rgb([v, v, v])
        });
        let small = resize_image(&image, (8, 8));
        assert!(
            small.pixels().all(|p| (100..=155).contains(&p.0[0])),
            "expected mid grey, got {:?}",
            small.get_pixel(4, 4)
        );
    }

    #[test]
    fn the_prefilter_never_undershoots_the_target() {
        // Whatever the factor, both reduced sides must still cover the target, so the triangle pass that follows only
        // ever downscales.
        for (w, h) in [(8256, 5504), (3456, 5184), (1201, 801), (600, 401)] {
            let factor = prefilter_factor((w, h), DISPLAY_IMAGE_SIZE);
            assert!(
                w / factor >= DISPLAY_IMAGE_SIZE.0 && h / factor >= DISPLAY_IMAGE_SIZE.1,
                "{w}x{h} reduced by {factor} falls under the target"
            );
        }
    }

    #[test]
    fn upscaling_skips_the_prefilter() {
        assert_eq!(prefilter_factor((300, 200), DISPLAY_IMAGE_SIZE), 1);
        let image = RgbImage::from_pixel(300, 200, image::Rgb([9, 9, 9]));
        assert_eq!(
            resize_image(&image, DISPLAY_IMAGE_SIZE).dimensions(),
            DISPLAY_IMAGE_SIZE
        );
    }

    #[test]
    fn box_reduce_averages_each_block() {
        // Two 2x2 blocks: one all 0, one all 200.
        let image = RgbImage::from_fn(4, 2, |x, _| {
            let v = if x < 2 { 0 } else { 200 };
            image::Rgb([v, v, v])
        });
        let reduced = box_reduce(&image, 2);
        assert_eq!(reduced.dimensions(), (2, 1));
        assert_eq!(reduced.get_pixel(0, 0).0, [0, 0, 0]);
        assert_eq!(reduced.get_pixel(1, 0).0, [200, 200, 200]);
    }

    #[test]
    fn the_two_step_descent_tracks_the_single_pass_one() {
        // A smooth gradient: the two paths must agree to within a level or two.
        let image = RgbImage::from_fn(2400, 1600, |x, y| {
            image::Rgb([(x / 10) as u8, (y / 10) as u8, ((x + y) / 16) as u8])
        });
        let two_step = resize_image(&image, DISPLAY_IMAGE_SIZE);
        let single = imageops::resize(&image, DISPLAY_IMAGE_SIZE.0, DISPLAY_IMAGE_SIZE.1, FilterType::Triangle);
        let worst = two_step
            .as_raw()
            .iter()
            .zip(single.as_raw())
            .map(|(a, b)| (*a as i32 - *b as i32).abs())
            .max()
            .unwrap();
        assert!(worst <= 2, "two-step resize drifted by {worst} levels");
    }

    #[test]
    fn resizes_to_the_requested_size() {
        let image = RgbImage::new(1200, 800);
        assert_eq!(
            resize_image(&image, DISPLAY_IMAGE_SIZE).dimensions(),
            DISPLAY_IMAGE_SIZE
        );
    }
}
