//! Command line front end for the reframe dithering pipeline.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::builder::TypedValueParser;
use clap::{Parser, ValueEnum};
use reframe_dither::{
    BayerSize, DitherMethod, DitherOptions, IndexedImage, Orientation, RgbImage, apply_dithering, display, io, resize,
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
}

impl From<MethodArg> for DitherMethod {
    fn from(value: MethodArg) -> Self {
        use reframe_dither::{ATKINSON, BURKES, FLOYD_STEINBERG, JARVIS_JUDICE_NINKE, STUCKI};
        match value {
            MethodArg::FloydSteinberg => DitherMethod::ErrorDiffusion(FLOYD_STEINBERG),
            MethodArg::Atkinson => DitherMethod::ErrorDiffusion(ATKINSON),
            MethodArg::Stucki => DitherMethod::ErrorDiffusion(STUCKI),
            MethodArg::Burkes => DitherMethod::ErrorDiffusion(BURKES),
            MethodArg::Jarvis => DitherMethod::ErrorDiffusion(JARVIS_JUDICE_NINKE),
            MethodArg::Ordered => DitherMethod::Ordered,
        }
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

    /// Working size, as WIDTHxHEIGHT.
    #[arg(long, default_value = "600x400", value_name = "WxH", value_parser = parse_size)]
    size: (u32, u32),

    /// Dither at the source resolution instead of resizing first.
    #[arg(long)]
    no_resize: bool,

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

impl Cli {
    fn dither_options(&self) -> DitherOptions {
        DitherOptions {
            saturation: self.saturation,
            brightness_factor: self.brightness,
            color_factor: self.color,
            method: self.method.into(),
            bayer_size: BayerSize::from_side_or_default(self.bayer_size),
            threshold_scale: self.threshold_scale,
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

fn process(cli: &Cli, input: &Path) -> Result<(), Box<dyn Error>> {
    let started = std::time::Instant::now();
    let photo = io::load_rgb(input)?;
    let source_size = photo.dimensions();

    let working: RgbImage = if cli.no_resize {
        photo
    } else {
        resize::resize_image(&photo, cli.size)
    };

    let options = cli.dither_options();

    // Packing the frame buffer runs the dither too, so only do the work once.
    let (dithered, buffer) = if cli.buffer {
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
        (dithered, Some(buf))
    } else {
        (apply_dithering(&working, &options), None)
    };

    let final_image: IndexedImage = if cli.upscale_2x {
        dithered.scale_nearest(2)
    } else {
        dithered
    };

    let out_path = cli.output_path(input);
    let buffer_path = out_path.with_extension("bin");

    if cli.dry_run {
        println!("{} -> {}", input.display(), out_path.display());
        if buffer.is_some() {
            println!("{} -> {}", input.display(), buffer_path.display());
        }
        return Ok(());
    }

    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }

    match cli.format {
        FormatArg::Indexed => io::save_indexed_png(&final_image, &out_path)?,
        FormatArg::Rgb => io::save_rgb_png(&final_image.to_rgb(), &out_path)?,
    }

    if let Some(buf) = &buffer {
        fs::write(&buffer_path, buf)?;
    }

    if cli.verbose {
        println!(
            "{} ({}x{}) -> {} ({}x{}) in {:.0}ms",
            input.display(),
            source_size.0,
            source_size.1,
            out_path.display(),
            final_image.width(),
            final_image.height(),
            started.elapsed().as_secs_f64() * 1000.0,
        );
    } else {
        println!("{}", out_path.display());
    }

    if let Some(buf) = &buffer
        && cli.verbose
    {
        println!("  frame buffer: {} ({} bytes)", buffer_path.display(), buf.len());
    }

    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let mut failed = 0usize;
    for input in &cli.inputs {
        if let Err(e) = process(&cli, input) {
            eprintln!("error: {}: {e}", input.display());
            failed += 1;
        }
    }

    if failed > 0 {
        eprintln!("{failed} of {} image(s) failed", cli.inputs.len());
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
