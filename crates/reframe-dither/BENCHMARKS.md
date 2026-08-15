# Benchmarks

A running log of `benches/pipeline.rs`, one entry per change that moves the numbers. The point is to make optimisation work auditable: every claim of a speedup should have a row here showing what it was before, what it is after, and what it cost elsewhere.

## Reproducing

```bash
cargo bench -p reframe-dither
```

The bench reads every `.jpg` in `assets/` at the repository root, sorted by filename, so a run on another machine measures the same work. It needs the `image-io` feature, which is on by default.

The CLI wall clock is a separate cross-check, and it should stay close to the bench's end-to-end figure. If the two drift apart, the bench has stopped covering something the CLI actually does:

```bash
cargo build --release -p reframe-dither
time ./target/release/reframe-dither assets/*.jpg -o /tmp/bench-out
du -sk /tmp/bench-out
```

## Method

Each stage is timed over the whole photo set, so one sample is one full batch and the per-stage rows add up to the end-to-end row. Stages run a warmup pass first, then several timed passes.

The reported number is the **best** sample, not the mean. On a laptop the mean mostly measures what else the machine was doing; the best sample is the run least disturbed by the scheduler and is the more stable basis for comparing two versions of the same code. The median is reported alongside it to show the spread. When best and median are more than a few percent apart, the machine was busy and the run is worth repeating.

Two cautions when comparing entries:

- Only compare numbers taken on the same machine. The environment block in each entry records which one.
- Thermal state matters. A resize measurement drifted 25% between two runs of identical code during this work, so treat any single-digit-percent change as noise unless it reproduces.

## Environment

Unless an entry says otherwise, measurements are from:

| | |
| --- | --- |
| Machine | Apple M2 Max, 12 cores (8 performance, 4 efficiency) |
| OS | macOS 15.6.1 |
| Toolchain | rustc 1.97.1 |
| Profile | `release`, `opt-level = 3` |
| Dataset | 10 JPEG photos in `assets/`, 177.0 MP total, 8256x5504 at the largest |

## Trend

| Date | Change | Batch (ms) | vs baseline | CLI wall clock | PNG output |
| --- | --- | ---: | ---: | ---: | ---: |
| 2026-08-15 | Baseline, no optimisations | 1061 | 1.00x | 1.10 s | 532 KiB |

## History

### 2026-08-15: baseline, no optimisations

First measurement, taken before any change to the pipeline. This is the reference every later entry compares against.

| stage | best (ms) | median | per image | share |
| --- | ---: | ---: | ---: | ---: |
| resize to 600x400 | 501.7 | 502.0 | 50.17 | 47.3% |
| decode jpeg | 391.9 | 404.2 | 39.19 | 36.9% |
| encode indexed png | 95.3 | 96.5 | 9.53 | 9.0% |
| dither jarvis | 65.8 | 66.1 | 6.58 | 6.2% |
| dither stucki | 65.5 | 66.1 | 6.55 | 6.2% |
| dither burkes | 63.6 | 65.4 | 6.36 | 6.0% |
| dither floyd-steinberg | 62.8 | 63.3 | 6.28 | 5.9% |
| dither atkinson | 62.1 | 62.6 | 6.21 | 5.8% |
| dither ordered | 26.9 | 27.1 | 2.69 | 2.5% |
| of which: ordered LUT build | 15.1 | 15.2 | 1.51 | 1.4% |
| enhance brightness+colour | 3.8 | 3.9 | 0.38 | 0.4% |
| rotate + pack frame buffer | 2.0 | 2.1 | 0.20 | 0.2% |
| **end to end (decode..encode)** | **1061.2** | **1064.8** | **106.12** | **100%** |

Throughput: 451.6 MP/s decoded, 9.4 photos/s end to end.

The shares only sum past 100% because the dither rows are alternatives to each other; end to end runs Floyd-Steinberg, the default.

Three observations from this baseline:

**Dithering is not the bottleneck.** The default kernel is 5.9% of the run. All five kernels land within 6% of each other despite Floyd-Steinberg having 4 taps and Jarvis having 12, which says the per-pixel cost is dominated by something other than spreading the error, most likely the nearest-colour search and the per-pixel accessor overhead.

**Resize and decode are 84% of the run.** Resize alone costs more than everything else combined, which is surprising for a stage that produces 240000 pixels. The input is what makes it expensive: a triangle filter widens its kernel as it downscales, so on a 45 MP photo it spans roughly a hundred source pixels per output pixel.

**Two smaller oddities.** Building the ordered mode's lookup table is 56% of that mode's cost and is redone per image, although it depends only on the saturation. And encoding a 240 KB indexed PNG takes 9.5 ms, which is a lot for that much data.

## Adding an entry

For each change that moves the numbers:

1. Run the bench on an otherwise idle machine, and repeat it if best and median disagree by more than a few percent.
2. Add a row to the trend table and a section to the history, newest last.
3. Record what got slower or larger as well as what got faster. A change that trades output size or fidelity for speed is only judgeable if both sides are written down.
4. If the change alters output pixels, say by how much, and use a perceptual measure rather than an exact-match count. Error diffusion is chaotic: a sub-level change to one input pixel flips a slot choice and cascades, so exact-match percentages collapse while the image stays visually identical. Compare local colour averages over blocks instead.
