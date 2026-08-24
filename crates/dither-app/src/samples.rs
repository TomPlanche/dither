//! The photos the app offers when you have not brought your own.
//!
//! They are the files in the repository's `assets/`, copied beside the bundle
//! by Trunk rather than compiled into it: nine megabytes of JPEG would dwarf
//! the WebAssembly module and be downloaded whether or not anyone picked one.
//! What is compiled in is the list of names, which `build.rs` reads from the
//! directory so it cannot fall behind what is actually there.

include!(concat!(env!("OUT_DIR"), "/samples.rs"));

/// Where a sample lives, relative to the page.
pub fn url(file: &str) -> String {
    format!("samples/{file}")
}

/// The samples with a label each, in the order they will be shown.
///
/// The files are named for the stock library and the photographer, so the
/// label is the photographer: the shelf mark in the middle says nothing to
/// anyone looking at a row of buttons. One who appears more than once is
/// numbered, since two buttons reading the same thing would be a coin toss.
pub fn labelled() -> Vec<(&'static str, String)> {
    let names: Vec<&'static str> = SAMPLES.iter().map(|file| photographer(file)).collect();

    let mut seen: Vec<&str> = Vec::new();
    SAMPLES
        .iter()
        .zip(&names)
        .map(|(file, name)| {
            let repeated = names.iter().filter(|other| *other == name).count() > 1;
            seen.push(name);
            let spelled = name.replace('-', " ");
            let label = if repeated {
                let nth = seen.iter().filter(|other| *other == name).count();
                format!("{spelled} {nth}")
            } else {
                spelled
            };
            (*file, label)
        })
        .collect()
}

/// The photographer's name out of a stock filename.
///
/// `pexels-mark-theunissen-2157442753-34724738.jpg` is the shape to read: a
/// library, a name that may be several words, then the numbers the library
/// files it under. Dropping the extension, the first word and every trailing
/// run of digits leaves the name.
fn photographer(file: &str) -> &str {
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    let body = stem.strip_prefix("pexels-").unwrap_or(stem);

    let mut name = body;
    while let Some((head, tail)) = name.rsplit_once('-') {
        if tail.chars().all(|c| c.is_ascii_digit()) {
            name = head;
        } else {
            break;
        }
    }

    // A filename that was all numbers leaves nothing to call it, so it keeps what it had.
    if name.is_empty() { body } else { name }
}

#[cfg(test)]
mod tests {
    use super::photographer;

    #[test]
    fn a_photographer_survives_the_shelf_marks_around_them() {
        assert_eq!(photographer("pexels-thales13-33322524.jpg"), "thales13");
        assert_eq!(
            photographer("pexels-mark-theunissen-2157442753-34724738.jpg"),
            "mark-theunissen"
        );
        assert_eq!(photographer("pexels-juju-649563-29285223.jpg"), "juju");
        // Nothing to strip, and nothing left if everything were stripped.
        assert_eq!(photographer("beach.png"), "beach");
        assert_eq!(photographer("pexels-12345.jpg"), "12345");
    }
}
