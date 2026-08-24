//! The footer, in the shape the portfolio's own uses: one line of credit and
//! links divided by slashes.

use leptos::prelude::*;

/// The project, and the repository every link here points into.
const REPOSITORY: &str = "https://github.com/TomPlanche/dither";

/// Who made it, and where they are.
const AUTHOR: (&str, &str) = ("Tom Planche", "https://tomplanche.com");

/// What follows the credit on the same line, as `(label, href)`.
///
/// Each one brings its own slash, which works because the credit always comes
/// first: there is never a leading separator to suppress, and adding a link is
/// adding a row to this table.
const LINKS: [(&str, &str); 1] = [("Repo", REPOSITORY)];

#[component]
pub fn Footer() -> impl IntoView {
    view! {
        <footer class="footer">
            <p class="made">
                "made by "
                <a href=AUTHOR.1 target="_blank" rel="noopener noreferrer">
                    {AUTHOR.0}
                </a>
                {LINKS
                    .into_iter()
                    .map(|(label, href)| {
                        view! {
                            <span class="sep">"/"</span>
                            <a href=href target="_blank" rel="noopener noreferrer">
                                {label}
                            </a>
                        }
                    })
                    .collect_view()}
            </p>
        </footer>
    }
}
