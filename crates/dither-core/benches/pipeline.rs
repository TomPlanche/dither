//! Stage-by-stage benchmark of the dithering pipeline over the photos in `assets/`.
//!
//! Run with `cargo bench -p dither-core`. There is no external harness: the
//! stages here run in the tens of milliseconds, so a warmup plus a handful of
//! timed repetitions is enough to separate them, and the report stays readable.
//!
//! Every stage is reported as the total across the whole asset set, so the
//! numbers add up to what the CLI spends on `dither-core assets/*.jpg`.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use reframe_dither::diffusion::{ATKINSON, BURKES, FLOYD_STEINBERG, JARVIS_JUDICE_NINKE, STUCKI};
use reframe_dither::dither::OrderedLut;
use reframe_dither::{
    DitherMethod, DitherOptions, PanelPalette, RgbImage, apply_dithering, display, enhance, io, resize,
};

/// The size the pipeline dithers at.
const WORKING: (u32, u32) = resize::DISPLAY_IMAGE_SIZE;

/// One decoded sample photo, kept at full resolution.
struct Asset {
    name: String,
    bytes: Vec<u8>,
    full: RgbImage,
}

impl Asset {
    /// Source pixels, in megapixels.
    fn megapixels(&self) -> f64 {
        (self.full.width() as f64 * self.full.height() as f64) / 1e6
    }
}

fn assets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

fn load_assets() -> Vec<Asset> {
    let dir = assets_dir();
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"))
        })
        .collect();
    // Sorted so runs stay comparable across machines.
    paths.sort();

    assert!(!paths.is_empty(), "no photos in {}", dir.display());

    paths
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path).expect("readable photo");
            let full = io::decode_rgb(&bytes).expect("decodable photo");
            Asset {
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                bytes,
                full,
            }
        })
        .collect()
}

/// A timed stage: the best and median of several passes over the whole set.
struct Measurement {
    stage: &'static str,
    best: Duration,
    median: Duration,
}

/// Times `body` over `reps` passes after `warmup` untimed ones.
///
/// `body` is expected to cover the entire asset set, so one sample is one full
/// batch. The best sample is the one to compare against: it is the run least
/// disturbed by the scheduler, and the medians only exist to show the spread.
fn measure(stage: &'static str, warmup: usize, reps: usize, mut body: impl FnMut()) -> Measurement {
    for _ in 0..warmup {
        body();
    }

    let mut samples = Vec::with_capacity(reps);
    for _ in 0..reps {
        let started = Instant::now();
        body();
        samples.push(started.elapsed());
    }
    samples.sort_unstable();

    Measurement {
        stage,
        best: samples[0],
        median: samples[samples.len() / 2],
    }
}

/// Keeps the optimiser from deleting a stage whose result is otherwise unused.
#[inline]
fn keep<T>(value: T) {
    std::hint::black_box(value);
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn main() {
    let assets = load_assets();
    let count = assets.len();
    let megapixels: f64 = assets.iter().map(Asset::megapixels).sum();

    println!(
        "dither-core pipeline benchmark: {count} photos, {megapixels:.1} MP total, working size {}x{}",
        WORKING.0, WORKING.1
    );
    println!();
    for asset in &assets {
        println!(
            "  {:<48} {:>5}x{:<5} {:>6.1} MP  {:>6.1} KiB",
            asset.name,
            asset.full.width(),
            asset.full.height(),
            asset.megapixels(),
            asset.bytes.len() as f64 / 1024.0,
        );
    }
    println!();

    let options = DitherOptions::default();
    let palette = PanelPalette::new(options.saturation);

    // Inputs for the later stages, so each stage is timed on its own.
    let sized: Vec<RgbImage> = assets.iter().map(|a| resize::resize_image(&a.full, WORKING)).collect();
    let enhanced: Vec<RgbImage> = sized
        .iter()
        .map(|image| {
            let mut work = image.clone();
            enhance::brightness(&mut work, options.brightness_factor as f32);
            enhance::color(&mut work, options.color_factor as f32);
            work
        })
        .collect();
    let indexed: Vec<_> = enhanced.iter().map(|image| apply_dithering(image, &options)).collect();

    let mut results = Vec::new();

    results.push(measure("decode jpeg", 1, 5, || {
        for asset in &assets {
            keep(io::decode_rgb(&asset.bytes).expect("decodable"));
        }
    }));

    results.push(measure("resize to 600x400", 1, 5, || {
        for asset in &assets {
            keep(resize::resize_image(&asset.full, WORKING));
        }
    }));

    results.push(measure("enhance brightness+colour", 2, 20, || {
        for image in &sized {
            let mut work = image.clone();
            enhance::brightness(&mut work, options.brightness_factor as f32);
            enhance::color(&mut work, options.color_factor as f32);
            keep(work);
        }
    }));

    for (stage, method) in [
        ("dither floyd-steinberg", DitherMethod::ErrorDiffusion(FLOYD_STEINBERG)),
        ("dither atkinson", DitherMethod::ErrorDiffusion(ATKINSON)),
        ("dither burkes", DitherMethod::ErrorDiffusion(BURKES)),
        ("dither stucki", DitherMethod::ErrorDiffusion(STUCKI)),
        ("dither jarvis", DitherMethod::ErrorDiffusion(JARVIS_JUDICE_NINKE)),
        ("dither ordered", DitherMethod::Ordered),
    ] {
        let options = DitherOptions { method, ..options };
        results.push(measure(stage, 1, 10, || {
            for image in &enhanced {
                keep(apply_dithering(image, &options));
            }
        }));
    }

    // Charged to every ordered dither, since `apply_dithering` rebuilds it.
    results.push(measure("  of which: ordered LUT build", 1, 10, || {
        for _ in 0..count {
            keep(OrderedLut::new(&palette));
        }
    }));

    results.push(measure("rotate + pack frame buffer", 2, 20, || {
        for image in &indexed {
            keep(display::img2buffer(image));
        }
    }));

    results.push(measure("encode indexed png", 2, 20, || {
        for image in &indexed {
            keep(io::encode_indexed_png(image).expect("encodable"));
        }
    }));

    let end_to_end = measure("END TO END (decode..encode)", 1, 5, || {
        for asset in &assets {
            let photo = io::decode_rgb(&asset.bytes).expect("decodable");
            let working = resize::resize_image(&photo, WORKING);
            let dithered = apply_dithering(&working, &options);
            keep(io::encode_indexed_png(&dithered).expect("encodable"));
        }
    });

    let total = end_to_end.best;
    let mut table = String::new();
    writeln!(
        table,
        "{:<34} {:>10} {:>10} {:>10} {:>8}",
        "stage", "best (ms)", "median", "per image", "share"
    )
    .unwrap();
    writeln!(table, "{}", "-".repeat(76)).unwrap();
    for r in &results {
        writeln!(
            table,
            "{:<34} {:>10.1} {:>10.1} {:>10.2} {:>7.1}%",
            r.stage,
            ms(r.best),
            ms(r.median),
            ms(r.best) / count as f64,
            100.0 * r.best.as_secs_f64() / total.as_secs_f64(),
        )
        .unwrap();
    }
    writeln!(table, "{}", "-".repeat(76)).unwrap();
    writeln!(
        table,
        "{:<34} {:>10.1} {:>10.1} {:>10.2} {:>7.1}%",
        end_to_end.stage,
        ms(end_to_end.best),
        ms(end_to_end.median),
        ms(end_to_end.best) / count as f64,
        100.0,
    )
    .unwrap();
    print!("{table}");

    println!();
    println!(
        "throughput: {:.1} MP/s decoded, {:.1} photos/s end to end",
        megapixels / results[0].best.as_secs_f64(),
        count as f64 / total.as_secs_f64(),
    );
}
