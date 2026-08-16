//! Brightness and colour boosts, reproducing `PIL.ImageEnhance`.
//!
//! Both are `Image.blend(degenerate, image, factor)`: a lerp away from a
//! reference image, extrapolating past it when the factor exceeds 1.

use image::RgbImage;

/// One step of PIL's `ImagingBlend`, clamped and truncated back to `u8`.
#[inline]
fn blend(from: u8, to: u8, factor: f32) -> u8 {
    let t = from as f32 + factor * (to as f32 - from as f32);
    t.clamp(0.0, 255.0) as u8
}

/// Rec. 601 luma on gamma-encoded values, as PIL's `RGB -> L` computes it.
///
/// Deliberately not `palette`'s luminance, which is linear-light and lands far
/// brighter (172 against 124 for the same pixel). The colour enhancer blends
/// toward this grey, so the choice sets how desaturation looks.
#[inline]
pub fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 19595 + g as u32 * 38470 + b as u32 * 7471 + 0x8000) >> 16) as u8
}

/// `ImageEnhance.Brightness`: blends away from black.
///
/// Precomputing all 256 answers into a lookup table was tried and measured 8% slower: the blend is one multiply and a
/// clamp, which the compiler vectorises across the buffer, and a table turns that into a gather it cannot. See
/// BENCHMARKS.md, entry 7.
pub fn brightness(image: &mut RgbImage, factor: f32) {
    if factor == 1.0 {
        return;
    }
    for channel in image.iter_mut() {
        *channel = blend(0, *channel, factor);
    }
}

/// `ImageEnhance.Color`: blends away from the greyscale version of the image.
pub fn color(image: &mut RgbImage, factor: f32) {
    if factor == 1.0 {
        return;
    }
    for px in image.pixels_mut() {
        let grey = luma(px.0[0], px.0[1], px.0[2]);
        for channel in &mut px.0 {
            *channel = blend(grey, *channel, factor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brightness_matches_pillow() {
        // Captured from ImageEnhance.Brightness(...).enhance(1.1).
        let input = [0u8, 1, 100, 200, 231, 232, 255];
        let expected = [0u8, 1, 110, 220, 254, 255, 255];
        for (i, e) in input.iter().zip(expected) {
            assert_eq!(blend(0, *i, 1.1), e, "input {i}");
        }
    }

    #[test]
    fn luma_matches_pillow() {
        // Captured from Image.convert("L").
        let cases = [
            ([10u8, 200, 30], 124u8),
            ([255, 0, 0], 76),
            ([0, 255, 0], 150),
            ([0, 0, 255], 29),
            ([123, 45, 67], 71),
        ];
        for ([r, g, b], want) in cases {
            assert_eq!(luma(r, g, b), want, "rgb {r},{g},{b}");
        }
    }

    #[test]
    fn a_factor_of_one_is_a_no_op() {
        let original = RgbImage::from_fn(4, 4, |x, y| image::Rgb([x as u8 * 9, y as u8 * 7, 33]));
        let mut image = original.clone();
        brightness(&mut image, 1.0);
        color(&mut image, 1.0);
        assert_eq!(image, original);
    }
}
