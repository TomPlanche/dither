//! Resizing to the panel's working size.

use image::RgbImage;
use image::imageops::{self, FilterType};

/// The landscape size the pipeline dithers at.
pub const DISPLAY_IMAGE_SIZE: (u32, u32) = (600, 400);

/// Scales a photo to the working size.
///
/// Uses a triangle filter, which widens its kernel when downscaling and so
/// behaves like an area average rather than point sampling.
pub fn resize_image(image: &RgbImage, target: (u32, u32)) -> RgbImage {
    if image.dimensions() == target {
        return image.clone();
    }
    imageops::resize(image, target.0, target.1, FilterType::Triangle)
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
    fn resizes_to_the_requested_size() {
        let image = RgbImage::new(1200, 800);
        assert_eq!(
            resize_image(&image, DISPLAY_IMAGE_SIZE).dimensions(),
            DISPLAY_IMAGE_SIZE
        );
    }
}
