//! Resizing to the panel's working size.

use std::fmt;
use std::str::FromStr;

use image::RgbImage;
use image::imageops::{self, FilterType};
use rayon::prelude::*;

/// The landscape size the pipeline dithers at.
pub const DISPLAY_IMAGE_SIZE: (u32, u32) = (600, 400);

/// As far into a photo as a crop will go.
///
/// Past this the kept rectangle is a stamp being blown up to the working size, which the resize can only blur.
pub const MAX_CROP_ZOOM: f32 = 10.0;

/// Aspect ratios that go by a name, for a caller that would rather not work the shape out itself.
///
/// A preset names a shape, never a pixel count. [`ratio_size`] fits it inside whatever working size was asked for, so
/// how much to dither stays the caller's to pick and a preset only ever reframes it.
///
/// The two panel entries are the layouts the frame buffer takes, and against the pipeline's own 600x400 they land on
/// it exactly, which is what [`crate::display`] can pack. The rest are the shapes the platforms crop to, so a dithered
/// photo can be posted without something else reframing it.
///
/// A preset names one orientation. The other one is [`FitOptions::keep_orientation`], which transposes whichever of
/// these is asked for: `iphone` on a portrait photo dithers at 3:4.
pub const RATIO_PRESETS: [(&str, (u32, u32)); 7] = [
    // The panel's two layouts, which 600x400 and 400x600 are the pixel counts of.
    ("panel", (3, 2)),
    ("panel-portrait", (2, 3)),
    ("instagram-post", (1, 1)),
    ("instagram-portrait", (4, 5)),
    // 1.91:1, which the feed states in decimals rather than whole sides.
    ("instagram-landscape", (191, 100)),
    ("instagram-story", (9, 16)),
    // The iPhone's default 4:3 photo.
    ("iphone", (4, 3)),
];

/// The aspect ratio a preset names, or `None` when nothing goes by that name.
pub fn preset_ratio(name: &str) -> Option<(u32, u32)> {
    RATIO_PRESETS
        .iter()
        .find(|(preset, _)| *preset == name)
        .map(|(_, ratio)| *ratio)
}

/// The preset names, in the order [`RATIO_PRESETS`] lists them.
pub const fn preset_names() -> [&'static str; RATIO_PRESETS.len()] {
    let mut names = [""; RATIO_PRESETS.len()];
    let mut i = 0;
    while i < RATIO_PRESETS.len() {
        names[i] = RATIO_PRESETS[i].0;
        i += 1;
    }
    names
}

/// The largest `ratio`-shaped size that fits inside `bounds`.
///
/// This is what a preset resolves to: the ratio picks the shape and `bounds` picks the scale, so the working size a
/// caller asked for still says how many pixels get dithered.
///
/// `bounds` is turned over first when the ratio disagrees with it, which is what keeps a portrait ratio against a
/// landscape working size from being squeezed into its short side: `panel-portrait` inside 600x400 is the panel's own
/// 400x600, not 266x400. A caller that wants the photo's orientation rather than the ratio's has
/// [`FitOptions::keep_orientation`], which transposes the result the same way.
///
/// A zero side leaves nothing to fit, so `bounds` comes back unchanged.
pub fn ratio_size(bounds: (u32, u32), ratio: (u32, u32)) -> (u32, u32) {
    if bounds.0 == 0 || bounds.1 == 0 || ratio.0 == 0 || ratio.1 == 0 {
        return bounds;
    }
    ratio_fit(orient_target(ratio, bounds), ratio)
}

/// How a photo that does not share the working size's shape is made to fit it.
///
/// The flags are off by default, which is the plainest thing to do: the photo is stretched into the working size
/// whatever shape it arrived in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitOptions {
    /// Transpose the working size for a photo of the other orientation, so a portrait photo stays portrait.
    pub keep_orientation: bool,
    /// Crop to the working size's aspect ratio rather than stretching the photo into it.
    pub crop: bool,
    /// Which part of the photo the crop keeps. Ignored unless `crop`.
    pub crop_from: CropOrigin,
    /// How far into the photo the crop moves, 1.0 being as much of it as the ratio allows. Ignored unless `crop`.
    ///
    /// At 1.0 the kept rectangle touches two opposite edges, so it can only slide along one axis. Anything above that
    /// keeps a proportionally smaller rectangle, which frees the other axis too: 2.0 keeps half the width and half the
    /// height, so [`CropOrigin::At`] can then reach anywhere in the photo.
    pub crop_zoom: f32,
}

impl Default for FitOptions {
    fn default() -> Self {
        Self {
            keep_orientation: false,
            crop: false,
            crop_from: CropOrigin::Center,
            crop_zoom: 1.0,
        }
    }
}

/// Which part of a photo a crop keeps.
///
/// An anchor takes the largest rectangle the target's ratio allows and puts it against a side. Such a rectangle spans
/// the photo's full width or its full height, never neither, so an anchor can only move it along the axis that has
/// slack: `Top` on a photo that is losing its sides behaves like `Center`.
///
/// [`CropOrigin::At`] works the other way round, and is the one to reach for when a coordinate has to mean what it
/// says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CropOrigin {
    /// The middle, which is what a photographer usually framed for.
    #[default]
    Center,
    Top,
    Bottom,
    Left,
    Right,
    /// Where the crop starts, in source pixels, `0,0` being the photo's top-left.
    ///
    /// The corner is kept as given and the rectangle grows from it, so `0,200` always drops the top 200 rows. What it
    /// costs is size: the rectangle is only as large as what is left below and to the right of the corner, so a corner
    /// far into the photo leaves a small one to blow back up.
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
    let source = image.dimensions();
    resize_region(
        image,
        fitted_rect(source, target, fit),
        fitted_size(source, target, fit),
    )
}

/// Repeats every pixel `factor` times in both directions.
///
/// The counterpart of [`crate::IndexedImage::scale_nearest`] for a photo that was never dithered, so `scale` means the
/// same thing whether or not the palette stage ran. A factor of 1 hands the photo straight back.
pub fn scale_nearest(image: &RgbImage, factor: u32) -> RgbImage {
    if factor <= 1 {
        return image.clone();
    }
    imageops::resize(
        image,
        image.width() * factor,
        image.height() * factor,
        FilterType::Nearest,
    )
}

/// The size [`resize_to_fit`] would produce for a photo of `source` dimensions.
///
/// That is `target` itself, unless `keep_orientation` turns it over to follow the photo.
pub fn fitted_size(source: (u32, u32), target: (u32, u32), fit: FitOptions) -> (u32, u32) {
    if fit.keep_orientation {
        orient_target(source, target)
    } else {
        target
    }
}

/// The part of a photo [`resize_to_fit`] would read, kept at its own resolution.
///
/// The crop without the scale, for a caller that asked to keep the source pixels. `target` is then read for its aspect
/// ratio alone, since nothing is being fitted to its size: a 1536x2048 photo cropped to 9:16 from `0,200` comes out
/// 1039x1848, not 1080x1920.
pub fn crop_to_fit(image: &RgbImage, target: (u32, u32), fit: FitOptions) -> RgbImage {
    scale_to_fit(image, target, fit, 1.0)
}

/// The part of a photo [`resize_to_fit`] would read, scaled by `factor`.
///
/// Between [`resize_to_fit`], which lands on a size, and [`crop_to_fit`], which lands on none: this one keeps the
/// photo's own proportions and asks only how much smaller, so 0.75 takes three quarters of each side of whatever the
/// crop kept. `target` is therefore read for its shape alone, the way [`crop_to_fit`] reads it.
///
/// A factor above 1.0 enlarges, which the triangle filter can only interpolate. The result is never empty: each side
/// rounds to at least one pixel.
pub fn scale_to_fit(image: &RgbImage, target: (u32, u32), fit: FitOptions, factor: f64) -> RgbImage {
    let rect = fitted_rect(image.dimensions(), target, fit);
    let factor = if factor.is_finite() && factor > 0.0 {
        factor
    } else {
        1.0
    };
    let scaled = |side: u32| ((f64::from(side) * factor).round() as u32).max(1);

    resize_region(image, rect, (scaled(rect.2), scaled(rect.3)))
}

/// The region of the photo [`resize_to_fit`] reads, as `(x, y, width, height)`.
///
/// The whole photo unless `crop`, which is what a caller reports when it wants to say which part of an upload was
/// used, and why a coordinate it asked for did not move anything.
pub fn fitted_rect(source: (u32, u32), target: (u32, u32), fit: FitOptions) -> (u32, u32, u32, u32) {
    if fit.crop {
        cover_rect(source, fitted_size(source, target, fit), fit.crop_from, fit.crop_zoom)
    } else {
        (0, 0, source.0, source.1)
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

/// Scales the part of a photo that already has the target's aspect ratio, taken from `origin` at `zoom`.
///
/// Nothing is distorted, and what it costs instead is the edges: a 3:2 photo against the 2:3 panel keeps the middle
/// third or so of its width. The crop is never materialised, so this reads the source once, the same as a plain resize.
pub fn resize_cropped(image: &RgbImage, target: (u32, u32), origin: CropOrigin, zoom: f32) -> RgbImage {
    resize_region(image, cover_rect(image.dimensions(), target, origin, zoom), target)
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

/// A rectangle of `source` with `target`'s aspect ratio, placed at `origin`, as `(x, y, width, height)`.
///
/// The two origins work from opposite ends. An anchor asks for the largest rectangle there is and then puts it against
/// a side, so it can only slide along whichever axis has slack. A corner is the other way round: the corner is what was
/// asked for, so it is kept, and the rectangle is the largest that fits in what the photo still offers below and to the
/// right of it. That is what makes `0,200` drop the top 200 rows on any photo, rather than being ignored on one whose
/// rectangle already spans the full height.
///
/// A corner near the far edge therefore leaves very little to keep, which the resize can only blur back up to the
/// working size. [`fitted_rect`] reports what was kept, so a caller can see it coming.
///
/// `zoom` shrinks whatever the origin settled on, staying at 1.0 for all of it. Below 1.0 there is nothing more to
/// keep, so it is treated as 1.0.
pub fn cover_rect(source: (u32, u32), target: (u32, u32), origin: CropOrigin, zoom: f32) -> (u32, u32, u32, u32) {
    let (sw, sh) = source;
    if target.0 == 0 || target.1 == 0 || sw == 0 || sh == 0 {
        return (0, 0, sw, sh);
    }

    // Both sides shrink by the same factor, so the ratio holds to within the rounding.
    let zoom = if zoom.is_finite() { zoom.max(1.0) } else { 1.0 };
    let shrink = |(width, height): (u32, u32)| {
        if zoom > 1.0 {
            (
                ((width as f32 / zoom).round() as u32).clamp(1, width),
                ((height as f32 / zoom).round() as u32).clamp(1, height),
            )
        } else {
            (width, height)
        }
    };

    if let CropOrigin::At { x, y } = origin {
        // The corner cannot be past the last pixel, since a rectangle has to have something to cover.
        let (x, y) = (x.min(sw - 1), y.min(sh - 1));
        let (width, height) = shrink(ratio_fit((sw - x, sh - y), target));
        return (x, y, width, height);
    }

    let (width, height) = shrink(ratio_fit(source, target));

    // What the rectangle leaves over. At a zoom of 1.0 that is only ever one axis.
    let (free_x, free_y) = (sw - width, sh - height);
    let (x, y) = match origin {
        CropOrigin::Top => (free_x / 2, 0),
        CropOrigin::Bottom => (free_x / 2, free_y),
        CropOrigin::Left => (0, free_y / 2),
        CropOrigin::Right => (free_x, free_y / 2),
        // `At` returned above, so this is `Center`.
        _ => (free_x / 2, free_y / 2),
    };

    (x, y, width, height)
}

/// The largest `target`-shaped rectangle that fits inside `space`.
///
/// The comparison is `sw * th` against `tw * sh` rather than a pair of divisions, so the ratios are exact and a space
/// that already matches the target keeps every pixel of it. Rounding down keeps the rectangle inside.
fn ratio_fit(space: (u32, u32), target: (u32, u32)) -> (u32, u32) {
    let (sw, sh) = space;
    let (sw64, sh64) = (u64::from(sw), u64::from(sh));
    let (tw64, th64) = (u64::from(target.0), u64::from(target.1));

    if sw64 * th64 > tw64 * sh64 {
        // The space is the wider of the two, so the sides come off.
        (((sh64 * tw64 / th64) as u32).clamp(1, sw), sh)
    } else {
        (sw, ((sw64 * th64 / tw64) as u32).clamp(1, sh))
    }
}

/// Scales the `(x, y, width, height)` region of a photo to `target`.
fn resize_region(image: &RgbImage, rect: (u32, u32, u32, u32), target: (u32, u32)) -> RgbImage {
    let (x, y, width, height) = rect;
    if (width, height) == target {
        // Nothing to scale, so the region is copied out as it is, which for the whole photo is a plain clone.
        return imageops::crop_imm(image, x, y, width, height).to_image();
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
    use crate::display::DISPLAY_PANEL_SIZE;

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

        // The default: whatever the photo, the size it was asked for.
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
            cover_rect((3456, 5184), DISPLAY_IMAGE_SIZE, centre, 1.0),
            (0, 1440, 3456, 2304)
        );
        // A 3:2 photo against the 2:3 panel: the width comes off.
        assert_eq!(cover_rect((5184, 3456), (400, 600), centre, 1.0), (1440, 0, 2304, 3456));
        // A photo that already matches keeps every pixel.
        assert_eq!(
            cover_rect((1200, 800), DISPLAY_IMAGE_SIZE, centre, 1.0),
            (0, 0, 1200, 800)
        );

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
                    let (x, y, w, h) = cover_rect(source, target, origin, 1.0);
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
    fn an_anchor_slides_the_largest_rectangle_along_the_axis_that_has_slack() {
        // A 2:3 photo against the 3:2 working size keeps 3456x2304, so 2880 rows are free.
        let source = (3456, 5184);
        let size = (3456, 2304);
        for (origin, expected) in [
            (CropOrigin::Center, (0, 1440)),
            (CropOrigin::Top, (0, 0)),
            (CropOrigin::Bottom, (0, 2880)),
            // The width has no slack, so a sideways anchor moves nothing.
            (CropOrigin::Left, (0, 1440)),
            (CropOrigin::Right, (0, 1440)),
        ] {
            let (x, y, w, h) = cover_rect(source, DISPLAY_IMAGE_SIZE, origin, 1.0);
            assert_eq!(((x, y), (w, h)), (expected, size), "{origin} landed wrong");
        }

        // Turn the photo over and the free axis turns with it.
        assert_eq!(cover_rect((5184, 3456), (400, 600), CropOrigin::Left, 1.0).0, 0);
        assert_eq!(cover_rect((5184, 3456), (400, 600), CropOrigin::Right, 1.0).0, 2880);
        assert_eq!(cover_rect((5184, 3456), (400, 600), CropOrigin::Top, 1.0).0, 1440);
    }

    #[test]
    fn a_corner_is_kept_and_the_rectangle_takes_what_is_left_of_the_photo() {
        // The case an anchor cannot express: a 3:4 photo into 9:16 keeps the full height, so no anchor drops the top.
        let source = (1536, 2048);
        let story = (1080, 1920);
        assert_eq!(cover_rect(source, story, CropOrigin::Top, 1.0), (192, 0, 1152, 2048));

        // The corner is honoured, and 1848 rows are left below it to fit a 9:16 rectangle into.
        assert_eq!(
            cover_rect(source, story, CropOrigin::At { x: 0, y: 200 }, 1.0),
            (0, 200, 1039, 1848)
        );
        // Which is the point: the top 200 rows are gone, whatever the photo's shape.
        for source in [(1536, 2048), (3456, 5184), (4000, 4000), (5184, 3456)] {
            let (x, y, ..) = cover_rect(source, story, CropOrigin::At { x: 0, y: 200 }, 1.0);
            assert_eq!((x, y), (0, 200), "{source:?} moved the corner");
        }

        // A corner has to leave at least one pixel, however far past the edge it points.
        assert_eq!(
            cover_rect(source, story, CropOrigin::At { x: 99999, y: 99999 }, 1.0),
            (1535, 2047, 1, 1)
        );

        // And the zoom shrinks whatever the corner left, from the corner.
        assert_eq!(
            cover_rect(source, story, CropOrigin::At { x: 0, y: 200 }, 2.0),
            (0, 200, 520, 924)
        );
    }

    #[test]
    fn a_corner_and_a_size_that_match_hand_back_the_original_pixels() {
        // A 100x200 photo cropped from 50,100 into a 50x100 working size. The corner leaves exactly 50x100, which is
        // the size asked for, so there is nothing to scale and the pixels come through untouched.
        let photo = RgbImage::from_fn(100, 200, |x, y| {
            image::Rgb([x as u8, (y / 2) as u8, ((x + y) % 251) as u8])
        });
        let fit = FitOptions {
            crop: true,
            crop_from: CropOrigin::At { x: 50, y: 100 },
            ..Default::default()
        };

        assert_eq!(fitted_rect(photo.dimensions(), (50, 100), fit), (50, 100, 50, 100));

        let out = resize_to_fit(&photo, (50, 100), fit);
        assert_eq!(out.dimensions(), (50, 100));
        for y in 0..100 {
            for x in 0..50 {
                assert_eq!(
                    out.get_pixel(x, y),
                    photo.get_pixel(50 + x, 100 + y),
                    "pixel {x},{y} moved"
                );
            }
        }

        // The shape is what fixes the rectangle, not the size: a square target from the same corner keeps 50x50, and
        // that too comes out pixel for pixel.
        assert_eq!(fitted_rect(photo.dimensions(), (50, 50), fit), (50, 100, 50, 50));
        assert_eq!(
            resize_to_fit(&photo, (50, 50), fit).get_pixel(10, 10),
            photo.get_pixel(60, 110)
        );

        // Ask for more than the corner leaves and the same region is scaled up to it instead.
        assert_eq!(fitted_rect(photo.dimensions(), (100, 100), fit), (50, 100, 50, 50));
        assert_eq!(resize_to_fit(&photo, (100, 100), fit).dimensions(), (100, 100));
    }

    #[test]
    fn a_factor_shrinks_whatever_the_framing_kept() {
        let photo = RgbImage::from_fn(100, 200, |x, y| image::Rgb([x as u8, (y / 2) as u8, 60]));

        // No crop: three quarters of each side of the whole photo.
        let plain = FitOptions::default();
        assert_eq!(scale_to_fit(&photo, (600, 400), plain, 0.75).dimensions(), (75, 150));
        assert_eq!(scale_to_fit(&photo, (600, 400), plain, 0.5).dimensions(), (50, 100));

        // With a crop, the factor applies to the rectangle that was kept rather than to the photo.
        let cropped = FitOptions {
            crop: true,
            ..Default::default()
        };
        let rect = fitted_rect(photo.dimensions(), (9, 16), cropped);
        assert_eq!(rect, (0, 11, 100, 177));
        assert_eq!(
            scale_to_fit(&photo, (9, 16), cropped, 0.75).dimensions(),
            (75, 133),
            "0.75 of the 100x177 the crop kept"
        );

        // 1.0 is the crop untouched, and a factor small enough to round a side away still leaves a pixel.
        assert_eq!(
            crop_to_fit(&photo, (9, 16), cropped),
            scale_to_fit(&photo, (9, 16), cropped, 1.0)
        );
        assert_eq!(scale_to_fit(&photo, (600, 400), plain, 0.001).dimensions(), (1, 1));

        // A factor that means nothing leaves the photo at its own size.
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(scale_to_fit(&photo, (600, 400), plain, bad).dimensions(), (100, 200));
        }
    }

    #[test]
    fn cropping_without_scaling_keeps_the_source_pixels() {
        // A 3:4 photo framed 9:16 from 200 rows down. Only the ratio of the target is read, not its size.
        let photo = RgbImage::from_fn(1536, 2048, |x, y| image::Rgb([(x / 8) as u8, (y / 8) as u8, 60]));
        let fit = FitOptions {
            crop: true,
            crop_from: CropOrigin::At { x: 0, y: 200 },
            ..Default::default()
        };

        let cropped = crop_to_fit(&photo, (9, 16), fit);
        assert_eq!(cropped.dimensions(), (1039, 1848));
        assert_eq!(
            fitted_rect(photo.dimensions(), (9, 16), fit),
            (0, 200, 1039, 1848),
            "the rect a caller reports is the one that was read"
        );

        // Naming the size rather than the ratio keeps the same pixels, since only the shape is consulted.
        assert_eq!(crop_to_fit(&photo, (1080, 1920), fit).dimensions(), (1039, 1848));

        // The pixels are the source's own, not resampled: the corner is where the crop started.
        assert_eq!(cropped.get_pixel(0, 0), photo.get_pixel(0, 200));
        assert_eq!(cropped.get_pixel(500, 500), photo.get_pixel(500, 700));

        // Scaling to the same size instead lands on 1080x1920, which is the other half of the pair.
        assert_eq!(resize_to_fit(&photo, (1080, 1920), fit).dimensions(), (1080, 1920));

        // And with no crop asked for, the photo comes back whole.
        assert_eq!(crop_to_fit(&photo, (9, 16), FitOptions::default()), photo);
    }

    #[test]
    fn a_zoom_shrinks_the_rectangle_and_frees_an_anchor_on_both_axes() {
        let source = (1536, 2048);
        let story = (1080, 1920);

        // A zoom is a proportional shrink, and anything under 1.0 is the whole rectangle.
        for (zoom, expected) in [(1.0, 1152), (1.5, 768), (4.0, 288), (0.5, 1152), (f32::NAN, 1152)] {
            assert_eq!(
                cover_rect(source, story, CropOrigin::Center, zoom).2,
                expected,
                "zoom {zoom} kept the wrong width"
            );
        }

        // Shrunk, the rectangle no longer touches the top and bottom, so a vertical anchor has somewhere to go.
        let (x, y, w, h) = cover_rect(source, story, CropOrigin::Top, 2.0);
        assert_eq!((x, y, w, h), (480, 0, 576, 1024));
        assert_eq!(
            cover_rect(source, story, CropOrigin::Bottom, 2.0),
            (480, 1024, 576, 1024)
        );

        // Both sides shrank by the same factor, so the ratio still holds.
        let drift = (u64::from(w) * u64::from(story.1)).abs_diff(u64::from(h) * u64::from(story.0));
        assert!(drift <= u64::from(story.1), "{w}x{h} drifted off 9:16");
    }

    #[test]
    fn the_reported_rect_is_the_one_the_resize_reads() {
        // Green in the top-left quarter, red everywhere else.
        let image = RgbImage::from_fn(400, 400, |x, y| {
            if x < 200 && y < 200 {
                image::Rgb([0, 255, 0])
            } else {
                image::Rgb([255, 0, 0])
            }
        });
        let target = (100, 100);

        let fit = FitOptions {
            crop: true,
            crop_from: CropOrigin::At { x: 0, y: 0 },
            crop_zoom: 2.0,
            ..Default::default()
        };
        assert_eq!(fitted_rect(image.dimensions(), target, fit), (0, 0, 200, 200));
        assert_eq!(fitted_size(image.dimensions(), target, fit), target);

        // What the rect says was kept is what came back.
        let out = resize_to_fit(&image, target, fit);
        assert_eq!(out.dimensions(), target);
        assert!(out.pixels().all(|p| p.0 == [0, 255, 0]), "expected the green quarter");

        // Without the crop the whole photo is read, which is what a caller reports for an uncropped request.
        assert_eq!(
            fitted_rect(image.dimensions(), target, FitOptions::default()),
            (0, 0, 400, 400)
        );

        // And the rect follows the transposed size when the orientation is kept.
        let fit = FitOptions {
            keep_orientation: true,
            crop: true,
            ..Default::default()
        };
        let portrait = RgbImage::new(400, 800);
        assert_eq!(fitted_size(portrait.dimensions(), DISPLAY_IMAGE_SIZE, fit), (400, 600));
        assert_eq!(
            fitted_rect(portrait.dimensions(), DISPLAY_IMAGE_SIZE, fit),
            (0, 100, 400, 600)
        );
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
            let cropped = resize_cropped(&image, target, CropOrigin::Center, 1.0);
            assert_eq!(cropped.dimensions(), target);
            assert!(
                cropped.pixels().all(|p| p.0 == [0, 255, 0]),
                "{width}px: the crop should hold the green band only, got {:?}",
                cropped.get_pixel(0, 0)
            );

            // Stretching instead keeps the red edges, which is the behaviour being opted out of.
            assert!(resize_image(&image, target).pixels().any(|p| p.0[0] > 128));

            // Taken from the left instead, the same crop holds the red edge only.
            let left = resize_cropped(&image, target, CropOrigin::Left, 1.0);
            assert!(
                left.pixels().all(|p| p.0 == [255, 0, 0]),
                "{width}px: the left crop should hold the red edge only, got {:?}",
                left.get_pixel(0, 0)
            );

            // And an explicit corner lands on the same pixels as the anchor that names it.
            let free = width - width / 3;
            assert_eq!(
                resize_cropped(&image, target, CropOrigin::At { x: free, y: 0 }, 1.0),
                resize_cropped(&image, target, CropOrigin::Right, 1.0)
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
            cover_rect(photo.dimensions(), (400, 600), CropOrigin::Center, 1.0),
            (67, 0, 1066, 1600)
        );
    }

    #[test]
    fn every_preset_names_one_usable_ratio() {
        for (name, (width, height)) in RATIO_PRESETS {
            assert!(width > 0 && height > 0, "{name} has a zero side");
            assert_eq!(preset_ratio(name), Some((width, height)));
            // One name per table entry, so a lookup is never ambiguous.
            assert_eq!(
                RATIO_PRESETS.iter().filter(|(other, _)| *other == name).count(),
                1,
                "{name} appears twice"
            );
        }

        assert_eq!(preset_ratio("panel"), Some((3, 2)));
        assert_eq!(preset_ratio("instagram-story"), Some((9, 16)));
        assert_eq!(preset_ratio("instagram-reel"), None);
        assert_eq!(preset_names().len(), RATIO_PRESETS.len());
    }

    #[test]
    fn a_preset_reshapes_the_working_size_rather_than_replacing_it() {
        // The shape comes from the preset, the scale from the size it is fitted inside.
        let sized = |name: &str, bounds| ratio_size(bounds, preset_ratio(name).expect("the preset exists"));
        for (name, expected) in [
            ("panel", DISPLAY_IMAGE_SIZE),
            ("panel-portrait", DISPLAY_PANEL_SIZE),
            ("instagram-post", (400, 400)),
            ("instagram-portrait", (400, 500)),
            ("instagram-landscape", (600, 314)),
            ("instagram-story", (337, 600)),
            ("iphone", (533, 400)),
        ] {
            assert_eq!(sized(name, DISPLAY_IMAGE_SIZE), expected, "{name} landed wrong");
        }

        // The same names against a larger working size cost more pixels and keep their shape.
        assert_eq!(sized("instagram-story", (1080, 1080)), (607, 1080));
        assert_eq!(sized("iphone", (1080, 1080)), (1080, 810));

        // Whatever the pair, the result is inside the bounds it was given, or inside their transpose when the ratio
        // turned them over.
        for bounds in [(600, 400), (1080, 1080), (37, 4000), (4096, 2160), (1, 1)] {
            for (name, ratio) in RATIO_PRESETS {
                let (width, height) = ratio_size(bounds, ratio);
                let room = orient_target(ratio, bounds);
                assert!(
                    width <= room.0 && height <= room.1 && width > 0 && height > 0,
                    "{name} in {bounds:?} gave {width}x{height}"
                );
            }
        }

        // Nothing to fit leaves the bounds alone rather than collapsing them.
        assert_eq!(ratio_size((600, 400), (0, 3)), (600, 400));
        assert_eq!(ratio_size((0, 400), (3, 2)), (0, 400));
    }

    #[test]
    fn a_preset_follows_the_photo_when_the_orientation_is_kept() {
        let portrait = RgbImage::new(1200, 1600);
        let fit = FitOptions {
            keep_orientation: true,
            crop: true,
            ..Default::default()
        };

        // `iphone` is landscape, so a portrait photo dithers at the transpose of what it resolved to.
        let target = ratio_size(DISPLAY_IMAGE_SIZE, preset_ratio("iphone").expect("the preset exists"));
        assert_eq!(target, (533, 400));
        assert_eq!(resize_to_fit(&portrait, target, fit).dimensions(), (400, 533));

        // Turning the ratio over instead lands on the same size, so which end the transpose happens at does not matter.
        let turned = ratio_size(DISPLAY_IMAGE_SIZE, (3, 4));
        assert_eq!(turned, (400, 533));

        // And a portrait photo against a portrait preset is already the right way round.
        let story = ratio_size(
            DISPLAY_IMAGE_SIZE,
            preset_ratio("instagram-story").expect("the preset exists"),
        );
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
