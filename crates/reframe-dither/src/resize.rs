//! Resizing to the panel's working size.

use std::fmt;
use std::str::FromStr;

use image::RgbImage;
use image::imageops::{self, FilterType};
use rayon::prelude::*;

use crate::display::DISPLAY_PANEL_SIZE;

/// The landscape size the pipeline dithers at.
pub const DISPLAY_IMAGE_SIZE: (u32, u32) = (600, 400);

/// Working sizes that go by a name, for a caller that would rather not carry the pixel counts around.
///
/// The two panel entries are the layouts the frame buffer takes, and are the only ones [`crate::display`] can pack. The
/// rest are what the platforms serve their images at, so a dithered photo can be posted without something else
/// resampling it and smearing the dither pattern on the way.
///
/// A preset names one orientation. The other one is [`FitOptions::keep_orientation`], which transposes whichever of
/// these is asked for: `iphone` on a portrait photo dithers at 3024x4032.
pub const SIZE_PRESETS: [(&str, (u32, u32)); 7] = [
    // The pipeline's own default, and the panel's portrait layout.
    ("panel", DISPLAY_IMAGE_SIZE),
    ("panel-portrait", DISPLAY_PANEL_SIZE),
    // Instagram serves 1080 wide, whatever the shape.
    ("instagram-post", (1080, 1080)),
    ("instagram-portrait", (1080, 1350)),
    ("instagram-landscape", (1080, 566)),
    ("instagram-story", (1080, 1920)),
    // The iPhone's default 4:3 photo.
    ("iphone", (4032, 3024)),
];

/// The size a preset names, or `None` when nothing goes by that name.
pub fn preset_size(name: &str) -> Option<(u32, u32)> {
    SIZE_PRESETS
        .iter()
        .find(|(preset, _)| *preset == name)
        .map(|(_, size)| *size)
}

/// The preset names, in the order [`SIZE_PRESETS`] lists them.
pub const fn preset_names() -> [&'static str; SIZE_PRESETS.len()] {
    let mut names = [""; SIZE_PRESETS.len()];
    let mut i = 0;
    while i < SIZE_PRESETS.len() {
        names[i] = SIZE_PRESETS[i].0;
        i += 1;
    }
    names
}

/// How a photo that does not share the working size's shape is made to fit it.
///
/// The flags are off by default, which is the camera's own behaviour: the photo is stretched into the landscape working
/// size whatever shape it arrived in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FitOptions {
    /// Transpose the working size for a photo of the other orientation, so a portrait photo stays portrait.
    pub keep_orientation: bool,
    /// Crop to the working size's aspect ratio rather than stretching the photo into it.
    pub crop: bool,
    /// Which part of the photo the crop keeps. Ignored unless `crop`.
    pub crop_from: CropOrigin,
}

/// Which part of a photo a crop keeps.
///
/// The kept rectangle's size is fixed by the target's aspect ratio, since anything else would distort the result. What
/// is left to choose is where it sits, and only along one axis: a crop either spans the photo's full width or its full
/// height, never neither. So an anchor moves the rectangle along the axis that has any slack and centres it on the
/// other, which is why `Top` on a photo that is losing its sides behaves like `Center`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CropOrigin {
    /// The middle, which is what a photographer usually framed for.
    #[default]
    Center,
    Top,
    Bottom,
    Left,
    Right,
    /// The rectangle's top-left corner, in source pixels.
    ///
    /// Clamped to what the photo can offer, because how much slack there is depends on the photo's own size, which the
    /// caller asking for a corner does not always know.
    At {
        x: u32,
        y: u32,
    },
}

impl fmt::Display for CropOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CropOrigin::Center => f.write_str("center"),
            CropOrigin::Top => f.write_str("top"),
            CropOrigin::Bottom => f.write_str("bottom"),
            CropOrigin::Left => f.write_str("left"),
            CropOrigin::Right => f.write_str("right"),
            CropOrigin::At { x, y } => write!(f, "{x},{y}"),
        }
    }
}

impl FromStr for CropOrigin {
    type Err = String;

    /// Parses an anchor name, or `X,Y` for a corner. [`fmt::Display`] writes back what this reads.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim() {
            "center" => Ok(CropOrigin::Center),
            "top" => Ok(CropOrigin::Top),
            "bottom" => Ok(CropOrigin::Bottom),
            "left" => Ok(CropOrigin::Left),
            "right" => Ok(CropOrigin::Right),
            corner => {
                let (x, y) = corner
                    .split_once(',')
                    .ok_or_else(|| format!("expected center, top, bottom, left, right or X,Y, got `{raw}`"))?;
                Ok(CropOrigin::At {
                    x: x.trim().parse().map_err(|_| format!("bad crop x `{x}`"))?,
                    y: y.trim().parse().map_err(|_| format!("bad crop y `{y}`"))?,
                })
            },
        }
    }
}

/// Scales a photo to the working size, fitted the way `fit` asks for.
///
/// Turning both flags on is what keeps a photo of any shape undistorted: the target follows the photo's orientation,
/// and whatever ratio is left over comes off the long side as a crop.
pub fn resize_to_fit(image: &RgbImage, target: (u32, u32), fit: FitOptions) -> RgbImage {
    let target = if fit.keep_orientation {
        orient_target(image.dimensions(), target)
    } else {
        target
    };

    if fit.crop {
        resize_cropped(image, target, fit.crop_from)
    } else {
        resize_image(image, target)
    }
}

/// Scales a photo to the working size, stretching it when the two shapes disagree.
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
    resize_region(image, (0, 0, image.width(), image.height()), target)
}

/// Scales the largest part of a photo that already has the target's aspect ratio, taken from `origin`.
///
/// Nothing is distorted, and what it costs instead is the edges: a 3:2 photo against the 2:3 panel keeps the middle
/// third or so of its width. The crop is never materialised, so this reads the source once, the same as a plain resize.
pub fn resize_cropped(image: &RgbImage, target: (u32, u32), origin: CropOrigin) -> RgbImage {
    resize_region(image, cover_rect(image.dimensions(), target, origin), target)
}

/// `target`, transposed when it and `source` disagree on orientation.
///
/// A square source counts as landscape, so a square photo against a landscape target is left alone.
pub fn orient_target(source: (u32, u32), target: (u32, u32)) -> (u32, u32) {
    if (source.1 > source.0) == (target.1 > target.0) {
        target
    } else {
        (target.1, target.0)
    }
}

/// The largest rectangle of `source` that has `target`'s aspect ratio, placed at `origin`, as `(x, y, width, height)`.
///
/// The comparison is `sw * th` against `tw * sh` rather than a pair of divisions, so the ratios are exact and a photo
/// that already matches the target keeps every pixel, wherever `origin` points.
pub fn cover_rect(source: (u32, u32), target: (u32, u32), origin: CropOrigin) -> (u32, u32, u32, u32) {
    let (sw, sh) = source;
    let (tw, th) = target;
    if tw == 0 || th == 0 {
        return (0, 0, sw, sh);
    }

    let (sw64, sh64) = (u64::from(sw), u64::from(sh));
    let (tw64, th64) = (u64::from(tw), u64::from(th));

    // Rounding down keeps the rectangle inside the photo.
    let (width, height) = if sw64 * th64 > tw64 * sh64 {
        // The photo is the wider of the two, so the sides come off.
        (((sh64 * tw64 / th64) as u32).clamp(1, sw), sh)
    } else {
        (sw, ((sw64 * th64 / tw64) as u32).clamp(1, sh))
    };

    // What the rectangle leaves over, which is only ever on one axis.
    let (free_x, free_y) = (sw - width, sh - height);
    let (x, y) = match origin {
        CropOrigin::Center => (free_x / 2, free_y / 2),
        CropOrigin::Top => (free_x / 2, 0),
        CropOrigin::Bottom => (free_x / 2, free_y),
        CropOrigin::Left => (0, free_y / 2),
        CropOrigin::Right => (free_x, free_y / 2),
        CropOrigin::At { x, y } => (x.min(free_x), y.min(free_y)),
    };

    (x, y, width, height)
}

/// Scales the `(x, y, width, height)` region of a photo to `target`.
fn resize_region(image: &RgbImage, rect: (u32, u32, u32, u32), target: (u32, u32)) -> RgbImage {
    let (x, y, width, height) = rect;
    if (width, height) == target && rect == (0, 0, image.width(), image.height()) {
        return image.clone();
    }

    match prefilter_factor((width, height), target) {
        // A crop view costs an offset per pixel, and this arm only ever runs on a photo under twice the target.
        1 => imageops::resize(
            &*imageops::crop_imm(image, x, y, width, height),
            target.0,
            target.1,
            FilterType::Triangle,
        ),
        factor => {
            let reduced = box_reduce(image, rect, factor);
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

/// Averages whole `factor` x `factor` blocks of the `(x, y, width, height)` region.
///
/// The last few source rows and columns are dropped when the side is not a multiple of `factor`. That is at most
/// `factor - 1` pixels off an edge that is thousands wide, and it keeps every block the same weight.
///
/// The region is read in place. A crop is only ever an offset and a shorter row here, so it never costs the copy that
/// materialising the sub-image would.
fn box_reduce(image: &RgbImage, rect: (u32, u32, u32, u32), factor: u32) -> RgbImage {
    let (x0, y0, width, height) = rect;
    let f = factor as usize;
    let out_w = (width as usize) / f;
    let out_h = (height as usize) / f;
    let src_stride = (image.width() as usize) * 3;
    let origin = (y0 as usize) * src_stride + (x0 as usize) * 3;
    let src = image.as_raw();

    // Round to nearest rather than truncating, which halves the drift.
    let count = (f * f) as u32;
    let half = count / 2;

    // Output rows are independent, and each reads a disjoint band of the source.
    let mut out = vec![0u8; out_w * out_h * 3];
    out.par_chunks_mut(out_w * 3).enumerate().for_each(|(y, row)| {
        let block_top = origin + y * f * src_stride;
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
        let reduced = box_reduce(&image, (0, 0, 4, 2), 2);
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
    fn orienting_only_transposes_a_target_that_disagrees() {
        // Portrait photo, landscape target: transposed.
        assert_eq!(orient_target((3456, 5184), DISPLAY_IMAGE_SIZE), (400, 600));
        // Landscape photo, landscape target: left alone.
        assert_eq!(orient_target((5184, 3456), DISPLAY_IMAGE_SIZE), DISPLAY_IMAGE_SIZE);
        // And the same against a portrait target.
        assert_eq!(orient_target((3456, 5184), (400, 600)), (400, 600));
        assert_eq!(orient_target((5184, 3456), (400, 600)), DISPLAY_IMAGE_SIZE);
        // A square photo counts as landscape.
        assert_eq!(orient_target((1000, 1000), DISPLAY_IMAGE_SIZE), DISPLAY_IMAGE_SIZE);
    }

    #[test]
    fn a_portrait_photo_stays_portrait() {
        let keep = FitOptions {
            keep_orientation: true,
            ..Default::default()
        };

        let portrait = RgbImage::new(1200, 1800);
        assert_eq!(
            resize_to_fit(&portrait, DISPLAY_IMAGE_SIZE, keep).dimensions(),
            (400, 600)
        );

        let landscape = RgbImage::new(1800, 1200);
        assert_eq!(
            resize_to_fit(&landscape, DISPLAY_IMAGE_SIZE, keep).dimensions(),
            DISPLAY_IMAGE_SIZE
        );

        // The default is the camera's: whatever the photo, the size it was asked for.
        assert_eq!(
            resize_to_fit(&portrait, DISPLAY_IMAGE_SIZE, FitOptions::default()).dimensions(),
            DISPLAY_IMAGE_SIZE
        );
    }

    #[test]
    fn the_cover_rect_is_centred_and_matches_the_target_ratio() {
        let centre = CropOrigin::Center;
        // A 2:3 photo against the 3:2 working size: the height comes off.
        assert_eq!(
            cover_rect((3456, 5184), DISPLAY_IMAGE_SIZE, centre),
            (0, 1440, 3456, 2304)
        );
        // A 3:2 photo against the 2:3 panel: the width comes off.
        assert_eq!(cover_rect((5184, 3456), (400, 600), centre), (1440, 0, 2304, 3456));
        // A photo that already matches keeps every pixel.
        assert_eq!(cover_rect((1200, 800), DISPLAY_IMAGE_SIZE, centre), (0, 0, 1200, 800));

        // Whatever the pair or the origin, the kept rectangle is inside the photo and holds the target's ratio to
        // within a pixel.
        for source in [(3456, 5184), (8256, 5504), (1000, 1000), (1201, 801), (37, 4000)] {
            for target in [DISPLAY_IMAGE_SIZE, (400, 600), (100, 100)] {
                for origin in [
                    centre,
                    CropOrigin::Top,
                    CropOrigin::Bottom,
                    CropOrigin::Left,
                    CropOrigin::Right,
                    CropOrigin::At { x: 9999, y: 9999 },
                ] {
                    let (x, y, w, h) = cover_rect(source, target, origin);
                    assert!(
                        x + w <= source.0 && y + h <= source.1,
                        "{source:?} -> {target:?} from {origin} escapes"
                    );
                    let drift = (u64::from(w) * u64::from(target.1)).abs_diff(u64::from(h) * u64::from(target.0));
                    assert!(
                        drift <= u64::from(target.0).max(u64::from(target.1)),
                        "{source:?} -> {target:?} kept {w}x{h}, off ratio"
                    );
                }
            }
        }
    }

    #[test]
    fn the_origin_moves_the_rectangle_along_whichever_axis_has_slack() {
        // A 2:3 photo against the 3:2 working size keeps 3456x2304, so 2880 rows are free.
        let source = (3456, 5184);
        let size = (3456, 2304);
        for (origin, expected) in [
            (CropOrigin::Center, (0, 1440)),
            (CropOrigin::Top, (0, 0)),
            (CropOrigin::Bottom, (0, 2880)),
            (CropOrigin::At { x: 0, y: 500 }, (0, 500)),
            // Past the end, so it settles against the bottom.
            (CropOrigin::At { x: 0, y: 99999 }, (0, 2880)),
            // The width has no slack, so an x moves nothing and neither does a sideways anchor.
            (CropOrigin::At { x: 700, y: 500 }, (0, 500)),
            (CropOrigin::Left, (0, 1440)),
            (CropOrigin::Right, (0, 1440)),
        ] {
            let (x, y, w, h) = cover_rect(source, DISPLAY_IMAGE_SIZE, origin);
            assert_eq!(((x, y), (w, h)), (expected, size), "{origin} landed wrong");
        }

        // Turn the photo over and the free axis turns with it.
        assert_eq!(cover_rect((5184, 3456), (400, 600), CropOrigin::Left).0, 0);
        assert_eq!(cover_rect((5184, 3456), (400, 600), CropOrigin::Right).0, 2880);
        assert_eq!(cover_rect((5184, 3456), (400, 600), CropOrigin::Top).0, 1440);
    }

    #[test]
    fn an_origin_writes_back_what_it_reads() {
        for origin in [
            CropOrigin::Center,
            CropOrigin::Top,
            CropOrigin::Bottom,
            CropOrigin::Left,
            CropOrigin::Right,
            CropOrigin::At { x: 12, y: 340 },
        ] {
            assert_eq!(origin.to_string().parse(), Ok(origin));
        }

        assert_eq!("  120 , 340 ".parse(), Ok(CropOrigin::At { x: 120, y: 340 }));
        assert_eq!(CropOrigin::default(), CropOrigin::Center);
        for bad in ["middle", "120", "120,", "-1,4", "1,2,3"] {
            assert!(bad.parse::<CropOrigin>().is_err(), "`{bad}` should not parse");
        }
    }

    #[test]
    fn cropping_keeps_the_middle_and_drops_the_edges() {
        // Red sides, a green band down the middle third.
        let banded = |width: u32, height: u32| {
            RgbImage::from_fn(width, height, |x, _| {
                if (width / 3..2 * width / 3).contains(&x) {
                    image::Rgb([0, 255, 0])
                } else {
                    image::Rgb([255, 0, 0])
                }
            })
        };

        // Straight through the triangle filter, and through the box prefilter as well.
        for (width, target) in [(120u32, (40u32, 40u32)), (1200, (100, 100))] {
            let image = banded(width, width / 3);
            let cropped = resize_cropped(&image, target, CropOrigin::Center);
            assert_eq!(cropped.dimensions(), target);
            assert!(
                cropped.pixels().all(|p| p.0 == [0, 255, 0]),
                "{width}px: the crop should hold the green band only, got {:?}",
                cropped.get_pixel(0, 0)
            );

            // Stretching instead keeps the red edges, which is the behaviour being opted out of.
            assert!(resize_image(&image, target).pixels().any(|p| p.0[0] > 128));

            // Taken from the left instead, the same crop holds the red edge only.
            let left = resize_cropped(&image, target, CropOrigin::Left);
            assert!(
                left.pixels().all(|p| p.0 == [255, 0, 0]),
                "{width}px: the left crop should hold the red edge only, got {:?}",
                left.get_pixel(0, 0)
            );

            // And an explicit corner lands on the same pixels as the anchor that names it.
            let free = width - width / 3;
            assert_eq!(
                resize_cropped(&image, target, CropOrigin::At { x: free, y: 0 }),
                resize_cropped(&image, target, CropOrigin::Right)
            );
        }
    }

    #[test]
    fn cropping_and_orientation_together_leave_a_photo_undistorted() {
        // A 3:4 portrait photo: the transpose alone would still stretch it to 2:3.
        let photo = RgbImage::from_fn(1200, 1600, |x, y| image::Rgb([(x / 8) as u8, (y / 8) as u8, 60]));
        let fit = FitOptions {
            keep_orientation: true,
            crop: true,
            ..Default::default()
        };

        let out = resize_to_fit(&photo, DISPLAY_IMAGE_SIZE, fit);
        assert_eq!(out.dimensions(), (400, 600));

        // 3:4 is taller than the 2:3 it is going into, so the crop comes off the width.
        assert_eq!(
            cover_rect(photo.dimensions(), (400, 600), CropOrigin::Center),
            (67, 0, 1066, 1600)
        );
    }

    #[test]
    fn every_preset_names_one_usable_size() {
        for (name, (width, height)) in SIZE_PRESETS {
            assert!(width > 0 && height > 0, "{name} is empty");
            assert_eq!(preset_size(name), Some((width, height)));
            // One name per size table entry, so a lookup is never ambiguous.
            assert_eq!(
                SIZE_PRESETS.iter().filter(|(other, _)| *other == name).count(),
                1,
                "{name} appears twice"
            );
        }

        assert_eq!(preset_size("panel"), Some(DISPLAY_IMAGE_SIZE));
        assert_eq!(preset_size("panel-portrait"), Some(DISPLAY_PANEL_SIZE));
        assert_eq!(preset_size("instagram-reel"), None);
        assert_eq!(preset_names().len(), SIZE_PRESETS.len());
    }

    #[test]
    fn a_preset_follows_the_photo_when_the_orientation_is_kept() {
        let portrait = RgbImage::new(1200, 1600);
        let fit = FitOptions {
            keep_orientation: true,
            crop: true,
            ..Default::default()
        };

        // `iphone` is landscape, so a portrait photo dithers at its transpose.
        let target = preset_size("iphone").expect("the preset exists");
        assert_eq!(orient_target(portrait.dimensions(), target), (3024, 4032));

        let story = preset_size("instagram-story").expect("the preset exists");
        assert_eq!(resize_to_fit(&portrait, story, fit).dimensions(), story);
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
