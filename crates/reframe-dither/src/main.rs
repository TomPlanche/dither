//! Command line front end for the reframe dithering pipeline.

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::builder::TypedValueParser;
use clap::{Parser, ValueEnum};
use rayon::prelude::*;
use reframe_dither::{
    BayerSize, CropOrigin, DitherMethod, DitherOptions, FitOptions, IndexedImage, MAX_CROP_ZOOM, Orientation, RgbImage,
    apply_dithering, display, io, resize,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum MethodArg {
    /// Error diffusion. The default, and what the camera ships with.
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
        use reframe_dither::{ATKINSON, BURKES, FLOYD_STEINBERG, JARVIS_JUDICE_NINKE, STUCKI};
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
    /// Indexed PNG carrying the 6-colour palette, as the camera saves it.
    Indexed,
    /// Plain RGB PNG, for viewers that dislike palette images.
    Rgb,
}

/// Dither photos to the reframe e-paper camera's 6-colour palette.
#[derive(Debug, Parser)]
#[command(name = "reframe-dither", version, about, long_about = None)]
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

    /// Blend between the pure and the muted panel palettes, 0.0 to 1.0.
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

    /// Working size, as WIDTHxHEIGHT. With --preset it is the box the ratio is fitted inside.
    #[arg(long, default_value = "600x400", value_name = "WxH", value_parser = parse_size)]
    size: (u32, u32),

    /// Aspect ratio by name, for the shapes a platform expects. Reshapes --size rather than replacing it.
    #[arg(
        long,
        value_name = "NAME",
        value_parser = clap::builder::PossibleValuesParser::new(resize::preset_names())
            .map(|name| resize::preset_ratio(&name).expect("the parser only accepts preset names")),
    )]
    preset: Option<(u32, u32)>,

    /// Dither at the source resolution instead of resizing first.
    #[arg(long)]
    no_resize: bool,

    /// Scale by a fraction of the source instead of to the working size: 0.75 takes a quarter off.
    #[arg(long, conflicts_with = "no_resize", value_name = "F", value_parser = parse_factor)]
    resize: Option<f64>,

    /// Keep the photo's orientation: a portrait photo resizes to the transpose of the working size.
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

    /// Double the output with nearest-neighbour, matching the dashboard export.
    #[arg(long)]
    upscale_2x: bool,

    /// Output encoding.
    #[arg(short, long, value_enum, default_value = "indexed")]
    format: FormatArg,

    /// Also write the packed e-paper frame buffer next to each image.
    #[arg(long)]
    buffer: bool,

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
    /// `--preset panel-portrait` against the default 600x400 is the panel's own 400x600.
    fn working_size(&self) -> (u32, u32) {
        match self.preset {
            Some(ratio) => resize::ratio_size(self.size, ratio),
            None => self.size,
        }
    }

    /// The shape the geometry is measured against, whatever `--no-resize` says.
    ///
    /// `--no-resize` keeps the source resolution, but a crop still needs a shape to aim at, and this is it. So it means
    /// no scaling rather than no framing, and `--crop` keeps working underneath it.
    fn working_ratio(&self) -> (u32, u32) {
        self.preset.unwrap_or_else(|| self.working_size())
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
    /// `<stem>_dithered.png`, matching the camera's own naming.
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

    // What the crop kept, so `--verbose` can say why a coordinate did not move anything.
    // The shape the framing is measured against, which is the working size itself when scaling to it.
    let target = if cli.no_resize || cli.resize.is_some() {
        cli.working_ratio()
    } else {
        cli.working_size()
    };
    let kept = cli.crop.then(|| resize::fitted_rect(source_size, target, cli.fit()));

    let working: RgbImage = match cli.resize {
        Some(factor) => resize::scale_to_fit(&photo, target, cli.fit(), factor),
        // Keeping the source pixels is no reason to stop framing them.
        None if cli.no_resize && cli.crop => resize::crop_to_fit(&photo, target, cli.fit()),
        None if cli.no_resize => photo,
        None => resize::resize_to_fit(&photo, target, cli.fit()),
    };

    // Packing the frame buffer runs the dither too, so only do the work once.
    let (rendered, buffer) = match cli.dither_options() {
        None => (Rendered::Plain(working), None),
        Some(options) if cli.buffer => {
            let (buf, dithered, orientation) = display::dither_to_display_buffer(&working, &options);
            if orientation == Orientation::Unexpected {
                eprintln!(
                    "warning: {} is {}x{}, not {}x{} or {}x{}; rotating anyway",
                    input.display(),
                    working.width(),
                    working.height(),
                    resize::DISPLAY_IMAGE_SIZE.0,
                    resize::DISPLAY_IMAGE_SIZE.1,
                    reframe_dither::DISPLAY_PANEL_SIZE.0,
                    reframe_dither::DISPLAY_PANEL_SIZE.1,
                );
            }
            (Rendered::Palette(dithered), Some(buf))
        },
        Some(options) => (Rendered::Palette(apply_dithering(&working, &options)), None),
    };

    let final_image = if cli.upscale_2x { rendered.scaled(2) } else { rendered };

    let out_path = cli.output_path(input);
    let buffer_path = out_path.with_extension("bin");

    if cli.dry_run {
        let mut report = format!("{} -> {}", input.display(), out_path.display());
        if buffer.is_some() {
            let _ = write!(report, "\n{} -> {}", input.display(), buffer_path.display());
        }
        return Ok(report);
    }

    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }

    final_image.save(&out_path, cli.format)?;

    if let Some(buf) = &buffer {
        fs::write(&buffer_path, buf)?;
    }

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

    if let Some(buf) = &buffer {
        let _ = write!(
            report,
            "\n  frame buffer: {} ({} bytes)",
            buffer_path.display(),
            buf.len()
        );
    }

    Ok(report)
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // The panel takes palette slots, so there is nothing to pack without a dither. Caught once rather than per input.
    if cli.buffer && cli.method == MethodArg::None {
        eprintln!("error: --buffer needs a dithered image, and --method none skips the dither");
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
