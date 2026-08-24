//! Command line front end for the core dithering pipeline.

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::builder::TypedValueParser;
use clap::{Parser, ValueEnum};
use dither_core::{
    BayerSize, CropOrigin, DitherMethod, DitherOptions, FitOptions, IndexedImage, MAX_CROP_ZOOM, RgbImage,
    apply_dithering, io, resize,
};
use rayon::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum MethodArg {
    /// Error diffusion. The default, and the sharpest of the kernels.
    FloydSteinberg,
    /// Error diffusion that sheds some error: cleaner highlights, more contrast.
    Atkinson,
    /// Wider error spread than Floyd-Steinberg, so smoother gradients.
    Stucki,
    /// Like Stucki but cheaper, with a grainier look.
    Burkes,
    /// The widest error spread, and the smoothest.
    Jarvis,
    /// Bayer threshold matrix. Structured rather than organic.
    Ordered,
    /// No dithering: writes the photo resized and cropped, for checking the framing.
    None,
}

impl MethodArg {
    /// The pipeline's own method, or `None` when the run is only about the framing.
    fn dither(self) -> Option<DitherMethod> {
        use dither_core::{ATKINSON, BURKES, FLOYD_STEINBERG, JARVIS_JUDICE_NINKE, STUCKI};
        Some(match self {
            MethodArg::FloydSteinberg => DitherMethod::ErrorDiffusion(FLOYD_STEINBERG),
            MethodArg::Atkinson => DitherMethod::ErrorDiffusion(ATKINSON),
            MethodArg::Stucki => DitherMethod::ErrorDiffusion(STUCKI),
            MethodArg::Burkes => DitherMethod::ErrorDiffusion(BURKES),
            MethodArg::Jarvis => DitherMethod::ErrorDiffusion(JARVIS_JUDICE_NINKE),
            MethodArg::Ordered => DitherMethod::Ordered,
            MethodArg::None => return Option::None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum FormatArg {
    /// Indexed PNG carrying the palette. Smaller, and the default.
    Indexed,
    /// Plain RGB PNG, for viewers that dislike palette images.
    Rgb,
}

/// Dither photos to a fixed colour palette.
#[derive(Debug, Parser)]
#[command(name = "dither-core", version, about, long_about = None)]
struct Cli {
    /// Images to process.
    #[arg(required = true, value_name = "IMAGE")]
    inputs: Vec<PathBuf>,

    /// Output file for a single input, or a directory for several.
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Dithering algorithm.
    #[arg(short, long, value_enum, default_value = "floyd-steinberg")]
    method: MethodArg,

    /// Blend between the pure and the muted palettes, 0.0 to 1.0.
    #[arg(long, default_value_t = 0.6, value_name = "F")]
    saturation: f64,

    /// Brightness multiplier applied before dithering.
    #[arg(long, default_value_t = 1.1, value_name = "F")]
    brightness: f64,

    /// Colour intensity multiplier applied before dithering.
    #[arg(long, default_value_t = 1.4, value_name = "F")]
    color: f64,

    /// Bayer matrix size. Ordered dithering only.
    #[arg(
        long,
        default_value_t = 4,
        value_parser = clap::builder::PossibleValuesParser::new(["2", "4", "8"])
            .map(|s| s.parse::<u32>().expect("value is one of 2, 4 or 8")),
        value_name = "N"
    )]
    bayer_size: u32,

    /// Scales the Bayer threshold amplitude. Ordered dithering only.
    #[arg(long, default_value_t = 1.0, value_name = "F")]
    threshold_scale: f64,

    /// Working size, as WIDTHxHEIGHT. Left out, the photo keeps its own size. With --preset it is the box the ratio
    /// is fitted inside.
    #[arg(long, value_name = "WxH", value_parser = parse_size)]
    size: Option<(u32, u32)>,

    /// Aspect ratio by name, for the shapes a platform expects. Reshapes --size, or the photo itself when there is no
    /// --size, rather than replacing either.
    #[arg(
        long,
        value_name = "NAME",
        value_parser = clap::builder::PossibleValuesParser::new(resize::preset_names())
            .map(|name| resize::preset_ratio(&name).expect("the parser only accepts preset names")),
    )]
    preset: Option<(u32, u32)>,

    /// Read --size as a shape only, and dither at the photo's own resolution. Without --size there is nothing to
    /// scale to anyway, so this only says anything alongside one.
    #[arg(long)]
    no_resize: bool,

    /// Scale by a fraction of what the framing kept: 0.75 takes a quarter off, 0.125 an eighth of each side.
    #[arg(long, conflicts_with = "no_resize", value_name = "F", value_parser = parse_factor)]
    resize: Option<f64>,

    /// Keep the photo's orientation: a portrait photo resizes to the transpose of the working size. Only bites when
    /// something named a size or a shape, since a photo already has its own.
    #[arg(long)]
    keep_orientation: bool,

    /// Crop to the working size's aspect ratio instead of stretching the photo into it.
    #[arg(long)]
    crop: bool,

    /// Which part the crop keeps: center, top, bottom, left, right, or a corner as X,Y.
    #[arg(long, requires = "crop", default_value = "center", value_name = "WHERE")]
    crop_from: CropOrigin,

    /// How far into the photo the crop moves. Above 1.0 it keeps a smaller rectangle, which frees both axes.
    #[arg(long, requires = "crop", default_value_t = 1.0, value_name = "F", value_parser = parse_zoom)]
    crop_zoom: f32,

    /// Double the output with nearest-neighbour.
    #[arg(long)]
    upscale_2x: bool,

    /// Output encoding.
    #[arg(short, long, value_enum, default_value = "indexed")]
    format: FormatArg,

    /// Report what would be written without writing it.
    #[arg(long)]
    dry_run: bool,

    /// Print per-image timings.
    #[arg(short, long)]
    verbose: bool,
}

fn parse_size(raw: &str) -> Result<(u32, u32), String> {
    let (w, h) = raw
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("expected WIDTHxHEIGHT, got `{raw}`"))?;
    let w: u32 = w.trim().parse().map_err(|_| format!("bad width `{w}`"))?;
    let h: u32 = h.trim().parse().map_err(|_| format!("bad height `{h}`"))?;
    if w == 0 || h == 0 {
        return Err("width and height must be non-zero".into());
    }
    Ok((w, h))
}

/// The resize fraction, which is a part of the photo's own size rather than a size of its own.
fn parse_factor(raw: &str) -> Result<f64, String> {
    let factor: f64 = raw
        .trim()
        .parse()
        .map_err(|_| format!("expected a number, got `{raw}`"))?;
    if !factor.is_finite() || factor <= 0.0 || factor > 1.0 {
        return Err(format!("expected a fraction between 0 and 1, got {factor}"));
    }
    Ok(factor)
}

/// The crop zoom, which has to be at least 1.0: below that there is nothing left inside the photo to keep.
fn parse_zoom(raw: &str) -> Result<f32, String> {
    let zoom: f32 = raw
        .trim()
        .parse()
        .map_err(|_| format!("expected a number, got `{raw}`"))?;
    if !zoom.is_finite() || !(1.0..=MAX_CROP_ZOOM).contains(&zoom) {
        return Err(format!("expected 1.0 to {MAX_CROP_ZOOM}, got {zoom}"));
    }
    Ok(zoom)
}

impl Cli {
    /// The dithering to run, or `None` under `--method none`.
    fn dither_options(&self) -> Option<DitherOptions> {
        self.method.dither().map(|method| DitherOptions {
            saturation: self.saturation,
            brightness_factor: self.brightness,
            color_factor: self.color,
            method,
            bayer_size: BayerSize::from_side_or_default(self.bayer_size),
            threshold_scale: self.threshold_scale,
        })
    }

    /// The size to dither at: `--size`, reshaped to `--preset`'s ratio when one was named.
    ///
    /// A preset only ever picks the shape, so `--size` still says how much gets dithered. It is fitted inside the pair
    /// rather than replacing it, and the pair is turned over first when the ratio disagrees with it, so
    /// `--preset instagram-story --size 600x400` is 337x600 rather than 225x400.
    ///
    /// With no `--size` the photo's own dimensions stand in, so the run keeps whatever resolution arrived and a preset
    /// only reshapes it. [`resize::working_size`] is what decides between the two, and the server and the browser
    /// front end call the same function, so none of the three can drift.
    fn working_size(&self, source: (u32, u32)) -> (u32, u32) {
        resize::working_size(source, self.size, self.preset)
    }

    fn fit(&self) -> FitOptions {
        FitOptions {
            keep_orientation: self.keep_orientation,
            crop: self.crop,
            crop_from: self.crop_from,
            crop_zoom: self.crop_zoom,
        }
    }

    /// Where a given input's dithered PNG should land.
    ///
    /// A single input may name its output file directly; otherwise outputs are
    /// `<stem>_dithered.png`, alongside the input.
    fn output_path(&self, input: &Path) -> PathBuf {
        let default_name = {
            let stem = input.file_stem().unwrap_or_default().to_string_lossy();
            PathBuf::from(format!("{stem}_dithered.png"))
        };

        match &self.output {
            // A lone input plus a path that is not an existing directory names the file.
            Some(path) if self.inputs.len() == 1 && !path.is_dir() => path.clone(),
            Some(dir) => dir.join(default_name),
            None => input.parent().map(|p| p.join(&default_name)).unwrap_or(default_name),
        }
    }
}

/// What came out of the pipeline: palette slots, or the plain photo when `--method none` skipped the dither.
enum Rendered {
    Palette(IndexedImage),
    Plain(RgbImage),
}

impl Rendered {
    fn dimensions(&self) -> (u32, u32) {
        match self {
            Rendered::Palette(image) => image.size(),
            Rendered::Plain(image) => image.dimensions(),
        }
    }

    fn scaled(self, factor: u32) -> Self {
        match self {
            Rendered::Palette(image) => Rendered::Palette(image.scale_nearest(factor)),
            Rendered::Plain(image) => Rendered::Plain(resize::scale_nearest(&image, factor)),
        }
    }

    /// Writes it out. An undithered photo has no palette to index, so `--format` does not apply to it.
    fn save(&self, path: &Path, format: FormatArg) -> Result<(), io::IoError> {
        match (self, format) {
            (Rendered::Palette(image), FormatArg::Indexed) => io::save_indexed_png(image, path),
            (Rendered::Palette(image), FormatArg::Rgb) => io::save_rgb_png(&image.to_rgb(), path),
            (Rendered::Plain(image), _) => io::save_rgb_png(image, path),
        }
    }
}

/// Runs the pipeline for one input and returns what should be printed for it.
///
/// The report is returned rather than printed because inputs are processed in parallel, and interleaved lines would be
/// unreadable.
fn process(cli: &Cli, input: &Path) -> Result<String, Box<dyn Error>> {
    let started = std::time::Instant::now();
    let photo = io::load_rgb(input)?;
    let source_size = photo.dimensions();

    // The size to dither at, and what the crop kept, so `--verbose` can say why a coordinate did not move anything.
    let target = cli.working_size(source_size);
    let kept = cli.crop.then(|| resize::fitted_rect(source_size, target, cli.fit()));

    // Naming a --size is itself the request to scale to it; without one, or under --no-resize, nothing is scaled and
    // a crop is what cuts the shape out at the resolution the photo already had. The server resolves the same three
    // cases from `resize=true|false|<fraction>`.
    let working: RgbImage = match (cli.resize, cli.no_resize) {
        (Some(factor), _) => resize::scale_to_fit(&photo, target, cli.fit(), factor),
        (None, false) if cli.size.is_some() => resize::resize_to_fit(&photo, target, cli.fit()),
        (None, _) if cli.crop => resize::crop_to_fit(&photo, target, cli.fit()),
        (None, _) => photo,
    };

    let rendered = match cli.dither_options() {
        None => Rendered::Plain(working),
        Some(options) => Rendered::Palette(apply_dithering(&working, &options)),
    };

    let final_image = if cli.upscale_2x { rendered.scaled(2) } else { rendered };

    let out_path = cli.output_path(input);

    if cli.dry_run {
        return Ok(format!("{} -> {}", input.display(), out_path.display()));
    }

    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }

    final_image.save(&out_path, cli.format)?;

    if !cli.verbose {
        return Ok(out_path.display().to_string());
    }

    let mut report = format!(
        "{} ({}x{}) -> {} ({}x{}) in {:.0}ms",
        input.display(),
        source_size.0,
        source_size.1,
        out_path.display(),
        final_image.dimensions().0,
        final_image.dimensions().1,
        started.elapsed().as_secs_f64() * 1000.0,
    );

    if let Some((x, y, width, height)) = kept {
        let _ = write!(report, "\n  crop: {width}x{height} from {x},{y}");
    }

    Ok(report)
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // A shape with nothing to reshape would be read, accepted and then do nothing at all, which is worth saying out
    // loud rather than leaving to be noticed in the output. The server refuses the same combination.
    if cli.preset.is_some() && cli.size.is_none() && !cli.crop {
        eprintln!("error: --preset needs either --size to be fitted inside, or --crop to be cut out of the photo");
        return ExitCode::FAILURE;
    }

    // Decoding dominates the pipeline and is single-threaded per photo, so a batch is fastest with one photo per core.
    // `map` over an indexed parallel iterator keeps the results in input order, so the output does not depend on which
    // thread happened to finish first.
    let reports: Vec<Result<String, String>> = cli
        .inputs
        .par_iter()
        .map(|input| process(&cli, input).map_err(|e| format!("error: {}: {e}", input.display())))
        .collect();

    let mut failed = 0usize;
    for report in &reports {
        match report {
            Ok(text) => println!("{text}"),
            Err(text) => {
                eprintln!("{text}");
                failed += 1;
            },
        }
    }

    if failed > 0 {
        eprintln!("{failed} of {} image(s) failed", cli.inputs.len());
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
