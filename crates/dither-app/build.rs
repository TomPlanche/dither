//! Writes the one thing the app needs to know at build time: the samples it offers.
//!
//! Hardcoding ten filenames would go stale the first time one is added or
//! renamed, and there is no directory listing to ask for in a browser: the
//! bundle is static files. So the names are read here, where the directory
//! actually is, and compiled in as a constant.

use std::path::Path;
use std::{env, fs};

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("cargo sets the manifest directory");
    let assets = Path::new(&manifest).join("../../assets");

    // So a photo added to the directory shows up without a clean build.
    println!("cargo:rerun-if-changed={}", assets.display());

    let mut names: Vec<String> = fs::read_dir(&assets)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            let name = name.to_ascii_lowercase();
            // What the pipeline can decode, and nothing the directory picks up along the way.
            [".jpg", ".jpeg", ".png"].iter().any(|ext| name.ends_with(ext))
        })
        .collect();
    // Sorted, so the row is in the same order on every machine.
    names.sort();

    let entries: String = names.iter().map(|name| format!("    {name:?},\n")).collect();
    let generated = format!(
        "/// The sample photos in `assets/`, as filenames under the bundle's `samples/`.\npub const SAMPLES: [&str; {}] = [\n{entries}];\n",
        names.len()
    );

    let out_dir = env::var("OUT_DIR").expect("cargo sets the output directory");
    fs::write(Path::new(&out_dir).join("samples.rs"), generated).expect("the sample list is written");
}
