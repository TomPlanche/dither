//! The interface: a photo on the left, the settings that shape it on the right.
//!
//! State is two signals deep on purpose. `source` holds the decoded upload and
//! changes once per file; `sized` holds it cut down to the working size and
//! changes whenever the framing does. The dither reads `sized`, so a colour
//! slider never pays for the resample of a 12-megapixel photo.

use std::sync::Arc;

use dither_core::{MAX_CROP_ZOOM, Palette, RgbImage};
use leptos::prelude::*;
use leptos::task::spawn_local;
use web_sys::{DragEvent, File, HtmlInputElement, KeyboardEvent};

use crate::browser;
use crate::footer::Footer;
use crate::pipeline::{self, Frame};
use crate::samples;
use crate::settings::{Anchor, Geometry, Look, MAX_DIMENSION, MAX_SCALE, Method, Resize, bayer_from_side};

#[component]
pub fn App() -> impl IntoView {
    let source = RwSignal::new(None::<Arc<RgbImage>>);
    let sized = RwSignal::new(None::<Arc<RgbImage>>);
    let geometry = RwSignal::new(Geometry::default());
    let look = RwSignal::new(Look::default());
    let scale = RwSignal::new(1_u32);
    let stem = RwSignal::new(String::from("image"));
    let error = RwSignal::new(None::<String>);
    let fetching = RwSignal::new(false);
    let output_size = RwSignal::new(None::<(u32, u32)>);

    // The finished frame is only ever read by the download button, so it stays
    // out of the reactive graph: storing it as a signal would ask every reader
    // to re-run for a buffer none of them look at.
    let frame = StoredValue::new(None::<Frame>);
    let canvas = NodeRef::<leptos::html::Canvas>::new();
    let picker = NodeRef::<leptos::html::Input>::new();

    // The stage is a drop target and a button both: an empty one opens the file
    // input that lives down in the panel, so the largest thing on the page is
    // also the easiest way to fill it.
    let choose = move || {
        if source.get_untracked().is_none()
            && let Some(input) = picker.get_untracked()
        {
            input.click();
        }
    };

    // Stage one. Runs on a new photo, or on any change to the framing.
    Effect::new(move |_| match source.get() {
        Some(photo) => sized.set(Some(Arc::new(pipeline::fit(&photo, geometry.get())))),
        None => sized.set(None),
    });

    // Stage two. Runs on top of stage one, and on every palette or method change.
    Effect::new(move |_| {
        let Some(working) = sized.get() else {
            output_size.set(None);
            frame.set_value(None);
            return;
        };

        let rendered = pipeline::shade(&working, look.get(), scale.get());
        output_size.set(Some((rendered.width, rendered.height)));

        if let Some(element) = canvas.get()
            && let Err(err) = browser::draw(&element, &rendered)
        {
            error.set(Some(format!("the preview did not paint: {err:?}")));
        }
        frame.set_value(Some(rendered));
    });

    // Where a picked file and a picked sample meet: both are bytes and a name by this point.
    let accept = move |bytes: Vec<u8>, name: &str| {
        stem.set(file_stem(name));
        match pipeline::decode(&bytes) {
            Ok(photo) => {
                error.set(None);
                // The working size starts as the photo's own, so the fields say something true before they are
                // touched and the default of keeping that size is a visible no-op rather than a blank.
                geometry.update(|geom| geom.size = Some(photo.dimensions()));
                source.set(Some(Arc::new(photo)));
            },
            Err(err) => error.set(Some(format!("that file did not decode: {}", browser::describe(&err)))),
        }
    };

    let load = move |file: File| {
        let name = file.name();
        spawn_local(async move {
            match browser::read_file(&file).await {
                Ok(bytes) => accept(bytes, &name),
                Err(_) => error.set(Some("that file could not be read".to_string())),
            }
        });
    };

    // Which sample is on screen: the credit names it, and the next roll steps around it.
    let picked = RwSignal::new(None::<usize>);

    // A sample is a couple of megabytes over the network, so the row says so while one is on its way.
    let load_sample = move |file: &'static str| {
        fetching.set(true);
        spawn_local(async move {
            match browser::fetch_bytes(&samples::url(file)).await {
                Ok(bytes) => {
                    // These are 20-megapixel photos, and dithering one at its own size is seconds of work on the
                    // tab's only thread. A quarter of each side is a sixteenth of that, which is fast enough to
                    // start turning the sliders on. Set before the photo lands, so the first render is already the
                    // small one rather than a slow one nobody asked for.
                    geometry.update(|geom| geom.resize = Resize::Factor(0.25));
                    accept(bytes, file);
                },
                Err(_) => error.set(Some(format!("{file} could not be loaded"))),
            }
            fetching.set(false);
        });
    };

    // One button for the whole set, so the roll is what picks the photo. It draws from the samples other than the
    // one already on screen, which costs a `Vec` of indices and buys a second click that always changes something.
    let choose_random = move || {
        let others: Vec<usize> = (0..samples::SAMPLES.len())
            .filter(|index| Some(*index) != picked.get_untracked())
            .collect();
        let Some(&index) = others.get((js_sys::Math::random() * others.len() as f64) as usize) else {
            return;
        };

        picked.set(Some(index));
        load_sample(samples::SAMPLES[index]);
    };

    // The row of names is gone, so the credit is what carries the photographer now.
    let credit = Signal::derive(move || picked.get().map(|index| samples::labelled()[index].1.clone()));

    // What the number fields show: the working size once one is picked, and the photo's own until then.
    let sides = Signal::derive(move || {
        let photo = source.get().map_or((0, 0), |photo| photo.dimensions());
        geometry.get().sides(photo)
    });

    let save = move |_| {
        frame.with_value(|held| {
            let Some(rendered) = held else { return };
            match rendered.output.encode_png() {
                Ok(png) => {
                    let name = format!("{}_{}.png", stem.get_untracked(), rendered.output.suffix());
                    if let Err(err) = browser::download(&png, &name) {
                        error.set(Some(format!("the download did not start: {err:?}")));
                    }
                },
                Err(err) => error.set(Some(format!("the PNG did not encode: {}", browser::describe(&err)))),
            }
        });
    };

    view! {
        // A column so the footer can sit at the bottom of a short page rather than
        // halfway up it, which is what `margin-top: auto` needs to push against.
        <div class="shell">
        <main class="app">
            <header class="masthead">
                <h1>"dither"</h1>
                <p>"Six colours, picked in the browser. The whole pipeline runs here, in WebAssembly."</p>
            </header>

            <section
                class="stage"
                class:empty=move || source.get().is_none()
                role=move || source.get().is_none().then_some("button")
                tabindex=move || source.get().is_none().then_some("0")
                on:click=move |_| choose()
                on:keydown=move |ev: KeyboardEvent| {
                    if matches!(ev.key().as_str(), "Enter" | " ") {
                        ev.prevent_default();
                        choose();
                    }
                }
                on:dragover=move |ev: DragEvent| ev.prevent_default()
                on:drop=move |ev: DragEvent| {
                    ev.prevent_default();
                    if let Some(file) = ev
                        .data_transfer()
                        .and_then(|transfer| transfer.files())
                        .and_then(|files| files.get(0))
                    {
                        load(file);
                    }
                }
            >
                <canvas node_ref=canvas class="preview" class:waiting=move || source.get().is_none() />
                <Show when=move || source.get().is_none()>
                    <p class="hint">"Drop a photo here, click to choose one, or pick a sample below."</p>
                </Show>
            </section>

            <aside class="panel">
                <div class="row">
                    <label class="file">
                        "Choose a photo"
                        <input
                            node_ref=picker
                            type="file"
                            accept="image/jpeg,image/png"
                            on:change=move |ev| {
                                let input: HtmlInputElement = event_target(&ev);
                                if let Some(file) = input.files().and_then(|files| files.get(0)) {
                                    load(file);
                                }
                            }
                        />
                    </label>
                    <button class="save" disabled=move || source.get().is_none() on:click=save>
                        "Download PNG"
                    </button>
                </div>

                // One button and a credit, so it stays a row under the file picker rather than a
                // panel of its own: a fieldset and its legend cost more height here than they explain.
                // The fetch says so in the button's own label, which is the only place it can go now.
                <Show when=move || !samples::SAMPLES.is_empty()>
                    <div class="samples">
                        <button
                            class="sample"
                            disabled=move || fetching.get()
                            on:click=move |_| choose_random()
                        >
                            {move || if fetching.get() { "Fetching..." } else { "Choose random image" }}
                        </button>
                        <Show when=move || credit.get().is_some()>
                            <span class="credit">"photo by " {move || credit.get()}</span>
                        </Show>
                    </div>
                </Show>

                <Show when=move || error.get().is_some()>
                    <p class="error">{move || error.get().unwrap_or_default()}</p>
                </Show>

                <fieldset>
                    <legend>"Dither"</legend>

                    <label class="control">
                        <span class="name">"Method"</span>
                        <select
                            prop:value=move || look.get().method.slug()
                            on:change=move |ev| {
                                let chosen = Method::from_slug(&event_target_value(&ev));
                                look.update(|look| look.method = chosen);
                            }
                        >
                            {Method::ALL
                                .into_iter()
                                .map(|method| view! { <option value=method.slug()>{method.label()}</option> })
                                .collect_view()}
                        </select>
                    </label>

                    <Show when=move || look.get().method.is_ordered()>
                        <label class="control">
                            <span class="name">"Bayer size"</span>
                            <select
                                prop:value=move || look.get().bayer_size.side().to_string()
                                on:change=move |ev| {
                                    let side = event_target_value(&ev).parse().unwrap_or(4);
                                    look.update(|look| look.bayer_size = bayer_from_side(side));
                                }
                            >
                                <option value="2">"2 x 2"</option>
                                <option value="4">"4 x 4"</option>
                                <option value="8">"8 x 8"</option>
                            </select>
                        </label>

                        <Slider
                            label="Threshold scale"
                            min=0.0
                            max=2.0
                            step=0.01
                            value=Signal::derive(move || look.get().threshold_scale)
                            on_change=Callback::new(move |next| look.update(|look| look.threshold_scale = next))
                        />
                    </Show>
                </fieldset>

                <fieldset>
                    <legend>"Palette"</legend>

                    <Slider
                        label="Saturation"
                        min=0.0
                        max=1.0
                        step=0.01
                        value=Signal::derive(move || look.get().saturation)
                        on_change=Callback::new(move |next| look.update(|look| look.saturation = next))
                    />
                    <div class="swatches">
                        {move || {
                            let palette = Palette::new(look.get().saturation);
                            let colors = *palette.colors();
                            colors
                                .into_iter()
                                .map(|[r, g, b]| {
                                    view! { <span class="swatch" style=format!("background:rgb({r} {g} {b})") /> }
                                })
                                .collect_view()
                        }}
                    </div>

                    <Slider
                        label="Brightness"
                        min=0.5
                        max=2.0
                        step=0.01
                        value=Signal::derive(move || look.get().brightness)
                        on_change=Callback::new(move |next| look.update(|look| look.brightness = next))
                    />
                    <Slider
                        label="Colour"
                        min=0.5
                        max=2.0
                        step=0.01
                        value=Signal::derive(move || look.get().color)
                        on_change=Callback::new(move |next| look.update(|look| look.color = next))
                    />
                </fieldset>

                <fieldset>
                    <legend>"Framing"</legend>

                    <div class="pair">
                        <Number
                            label="Width"
                            value=Signal::derive(move || sides.get().0)
                            on_change=Callback::new(move |next| {
                                let (_, height) = sides.get_untracked();
                                geometry.update(|geom| geom.size = Some((next, height)));
                            })
                        />
                        <Number
                            label="Height"
                            value=Signal::derive(move || sides.get().1)
                            on_change=Callback::new(move |next| {
                                let (width, _) = sides.get_untracked();
                                geometry.update(|geom| geom.size = Some((width, next)));
                            })
                        />
                    </div>

                    <label class="control">
                        <span class="name">"Resize"</span>
                        <select
                            prop:value=move || geometry.get().resize.slug()
                            on:change=move |ev| {
                                let chosen = Resize::from_slug(&event_target_value(&ev));
                                geometry.update(|geom| geom.resize = chosen);
                            }
                        >
                            {Resize::ALL
                                .into_iter()
                                .map(|mode| view! { <option value=mode.slug()>{mode.label()}</option> })
                                .collect_view()}
                        </select>
                    </label>

                    <Show when=move || !geometry.get().resize.names_a_size()>
                        <p class="note">
                            {move || {
                                if geometry.get().crop {
                                    "Width and height are read as a shape here rather than a size: the crop cuts that shape out at the photo's own resolution."
                                } else {
                                    "Width and height only shape the crop, and there is no crop, so nothing is using them. Turn on the crop below, or scale to the size instead."
                                }
                            }}
                        </p>
                    </Show>

                    <Toggle
                        label="Keep orientation"
                        value=Signal::derive(move || geometry.get().keep_orientation)
                        on_change=Callback::new(move |next| geometry.update(|geom| geom.keep_orientation = next))
                    />
                    <Toggle
                        label="Crop instead of stretch"
                        value=Signal::derive(move || geometry.get().crop)
                        on_change=Callback::new(move |next| geometry.update(|geom| geom.crop = next))
                    />

                    <Show when=move || geometry.get().crop>
                        <label class="control">
                            <span class="name">"Crop from"</span>
                            <select
                                prop:value=move || geometry.get().crop_from.slug()
                                on:change=move |ev| {
                                    let chosen = Anchor::from_slug(&event_target_value(&ev));
                                    geometry.update(|geom| geom.crop_from = chosen);
                                }
                            >
                                {Anchor::ALL
                                    .into_iter()
                                    .map(|anchor| view! { <option value=anchor.slug()>{anchor.label()}</option> })
                                    .collect_view()}
                            </select>
                        </label>

                        <Slider
                            label="Crop zoom"
                            min=1.0
                            max=MAX_CROP_ZOOM as f64
                            step=0.1
                            value=Signal::derive(move || geometry.get().crop_zoom as f64)
                            on_change=Callback::new(move |next| {
                                geometry.update(|geom| geom.crop_zoom = next as f32)
                            })
                        />
                    </Show>
                </fieldset>

                <fieldset>
                    <legend>"Output"</legend>

                    <label class="control">
                        <span class="name">
                            "Scale"
                            <output>{move || format!("{}x", scale.get())}</output>
                        </span>
                        <input
                            type="range"
                            min="1"
                            max=MAX_SCALE.to_string()
                            step="1"
                            prop:value=move || scale.get().to_string()
                            on:input=move |ev| {
                                if let Ok(next) = event_target_value(&ev).parse::<u32>() {
                                    scale.set(next.clamp(1, MAX_SCALE));
                                }
                            }
                        />
                    </label>

                    <p class="readout">
                        {move || match output_size.get() {
                            Some((width, height)) => format!("{width} x {height} pixels"),
                            None => "no photo yet".to_string(),
                        }}
                    </p>
                </fieldset>
            </aside>
        </main>
        <Footer />
        </div>
    }
}

/// A labelled range whose current value is shown beside its name.
#[component]
fn Slider(
    label: &'static str,
    min: f64,
    max: f64,
    step: f64,
    value: Signal<f64>,
    on_change: Callback<f64>,
) -> impl IntoView {
    view! {
        <label class="control">
            <span class="name">
                {label}
                <output>{move || format!("{:.2}", value.get())}</output>
            </span>
            <input
                type="range"
                min=min.to_string()
                max=max.to_string()
                step=step.to_string()
                prop:value=move || value.get().to_string()
                on:input=move |ev| {
                    if let Ok(next) = event_target_value(&ev).parse::<f64>() {
                        on_change.run(next);
                    }
                }
            />
        </label>
    }
}

/// A dimension box, clamped to what the pipeline will accept.
#[component]
fn Number(label: &'static str, value: Signal<u32>, on_change: Callback<u32>) -> impl IntoView {
    view! {
        <label class="control">
            <span class="name">{label}</span>
            <input
                type="number"
                min="1"
                max=MAX_DIMENSION.to_string()
                prop:value=move || value.get().to_string()
                on:change=move |ev| {
                    if let Ok(next) = event_target_value(&ev).parse::<u32>() {
                        on_change.run(next.clamp(1, MAX_DIMENSION));
                    }
                }
            />
        </label>
    }
}

/// A checkbox that reads as a sentence.
#[component]
fn Toggle(label: &'static str, value: Signal<bool>, on_change: Callback<bool>) -> impl IntoView {
    view! {
        <label class="control toggle">
            <input
                type="checkbox"
                prop:checked=move || value.get()
                on:change=move |ev| {
                    let input: HtmlInputElement = event_target(&ev);
                    on_change.run(input.checked());
                }
            />
            <span class="name">{label}</span>
        </label>
    }
}

/// Drops the extension, so `beach.jpg` downloads as `beach_dithered.png`.
fn file_stem(name: &str) -> String {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem).to_string()
}
