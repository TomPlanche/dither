//! End-to-end tests over the router, driven in-process. No socket is bound.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use dither_server::{Config, router};
use reframe_dither::{DISPLAY_PANEL_SIZE, RgbImage, io};
use tower::ServiceExt;

/// A small landscape PNG with enough colour variation to exercise the dither.
fn source_png() -> Vec<u8> {
    sized_png(120, 90)
}

/// A 3:1 image, black but for a white band down the middle third.
///
/// Both colours survive the dither exactly, so what comes back says which part of the source was kept.
fn banded_png() -> Vec<u8> {
    let (width, height) = (120u32, 40u32);
    let mut pixels = Vec::with_capacity((width * height * 3) as usize);
    for _ in 0..height {
        for x in 0..width {
            let band = (width / 3..2 * width / 3).contains(&x);
            let v = if band { 255u8 } else { 0 };
            pixels.extend_from_slice(&[v, v, v]);
        }
    }

    let image = RgbImage::from_raw(width, height, pixels).expect("the buffer matches the size");
    io::encode_rgb_png(&image).expect("the test image encodes")
}

/// The same image at an arbitrary size, for the orientation tests.
fn sized_png(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&[(x * 2) as u8, (y * 2) as u8, ((x + y) % 256) as u8]);
        }
    }

    let image = RgbImage::from_raw(width, height, pixels).expect("the buffer matches the size");
    io::encode_rgb_png(&image).expect("the test image encodes")
}

async fn call(request: Request<Body>) -> Response {
    router(Arc::new(Config::default()))
        .oneshot(request)
        .await
        .expect("the router answers")
}

async fn body_bytes(response: Response) -> Bytes {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body reads")
}

fn post(uri: &str, content_type: &str, body: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", content_type)
        .body(Body::from(body))
        .expect("the request builds")
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("the request builds")
}

#[tokio::test]
async fn health_reports_ok() {
    let response = call(get("/health")).await;
    assert_eq!(response.status(), StatusCode::OK);

    let json: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).expect("health is JSON");
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn options_defaults_round_trip_into_a_query_string() {
    let response = call(get("/api/options")).await;
    assert_eq!(response.status(), StatusCode::OK);

    let json: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).expect("options is JSON");
    assert_eq!(json["defaults"]["method"], "floyd-steinberg");
    assert_eq!(json["defaults"]["width"], 600);

    // Seven slots, blended at the default saturation, ready for CSS.
    let palette = json["panel"]["palette"].as_array().expect("palette is an array");
    assert_eq!(palette.len(), 7);
    assert!(palette.iter().all(|color| {
        let color = color.as_str().unwrap_or_default();
        color.len() == 7 && color.starts_with('#') && color[1..].chars().all(|c| c.is_ascii_hexdigit())
    }));

    // Every default is a name the endpoints also accept.
    let defaults = json["defaults"].as_object().expect("defaults is an object");
    let query: Vec<String> = defaults
        .iter()
        .map(|(key, value)| format!("{key}={}", value.to_string().trim_matches('"')))
        .collect();

    let response = call(post(
        &format!("/api/dither?{}", query.join("&")),
        "image/png",
        source_png(),
    ))
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_raw_body_comes_back_as_a_png_at_the_working_size() {
    let response = call(post("/api/dither", "image/png", source_png())).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/png");
    assert_eq!(response.headers()["x-image-size"], "600x400");

    let bytes = body_bytes(response).await;
    assert_eq!(&bytes[1..4], b"PNG", "the response is a PNG");
}

#[tokio::test]
async fn a_multipart_image_field_is_accepted() {
    let boundary = "----dithertest";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"content-disposition: form-data; name=\"image\"; filename=\"photo.png\"\r\n");
    body.extend_from_slice(b"content-type: image/png\r\n\r\n");
    body.extend_from_slice(&source_png());
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let response = call(post(
        "/api/dither?method=ordered&bayer_size=8",
        &format!("multipart/form-data; boundary={boundary}"),
        body,
    ))
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-image-size"], "600x400");
}

#[tokio::test]
async fn keep_orientation_transposes_the_working_size_for_a_portrait_upload() {
    let response = call(post(
        "/api/dither?keep_orientation=true",
        "image/png",
        sized_png(90, 120),
    ))
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-image-size"], "400x600");

    // A landscape upload is unaffected.
    let response = call(post("/api/dither?keep_orientation=true", "image/png", source_png())).await;
    assert_eq!(response.headers()["x-image-size"], "600x400");

    // And without the flag a portrait upload is still squashed into the landscape size.
    let response = call(post("/api/dither", "image/png", sized_png(90, 120))).await;
    assert_eq!(response.headers()["x-image-size"], "600x400");
}

#[tokio::test]
async fn a_preset_reshapes_the_working_size_rather_than_replacing_it() {
    // 9:16 fitted inside the default 600x400, which the ratio turns over first.
    let response = call(post("/api/dither?preset=instagram-story", "image/png", source_png())).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-image-size"], "337x600");

    // The pair still says how much gets dithered, so a bigger one buys more pixels of the same shape.
    let uri = "/api/dither?preset=instagram-story&width=1080&height=1080";
    let response = call(post(uri, "image/png", source_png())).await;
    assert_eq!(response.headers()["x-image-size"], "607x1080");

    // And a smaller one costs them: 2:3 inside 320x240 rather than the panel's own 400x600.
    let uri = "/api/dither?preset=panel-portrait&width=320&height=240";
    let response = call(post(uri, "image/png", source_png())).await;
    assert_eq!(response.headers()["x-image-size"], "213x320");

    // It follows the photo when the orientation is kept: the portrait preset turned over for a landscape upload.
    let uri = "/api/dither?preset=panel-portrait&keep_orientation=true";
    let response = call(post(uri, "image/png", source_png())).await;
    assert_eq!(response.headers()["x-image-size"], "600x400");
}

#[tokio::test]
async fn options_lists_every_preset_with_its_ratio() {
    let json: serde_json::Value =
        serde_json::from_slice(&body_bytes(call(get("/api/options")).await).await).expect("options is JSON");

    let presets = json["presets"].as_array().expect("presets is an array");
    let names: Vec<&str> = presets
        .iter()
        .map(|preset| preset["name"].as_str().expect("a preset name"))
        .collect();
    assert_eq!(
        names,
        [
            "panel",
            "panel-portrait",
            "instagram-post",
            "instagram-portrait",
            "instagram-landscape",
            "instagram-story",
            "iphone"
        ]
    );
    assert_eq!(presets[0]["ratio"], serde_json::json!([3, 2]));

    // Every listed name end to end. A preset is fitted inside `width` and `height`, so none of them can cost more than
    // the default 600x400 does, and all seven are affordable here.
    for preset in presets {
        let name = preset["name"].as_str().expect("a preset name");
        let response = call(post(&format!("/api/dither?preset={name}"), "image/png", source_png())).await;
        assert_eq!(response.status(), StatusCode::OK, "{name} should be accepted");

        let ratio = preset["ratio"].as_array().expect("a preset ratio");
        let (rw, rh) = (
            ratio[0].as_u64().expect("a width"),
            ratio[1].as_u64().expect("a height"),
        );

        // What came back is the ratio the listing advertised, to within the rounding, and inside the 600x400 that was
        // asked for or inside its transpose when the ratio turned it over.
        let size = response.headers()["x-image-size"].to_str().expect("a size header");
        let (width, height) = size.split_once('x').expect("WIDTHxHEIGHT");
        let (width, height): (u64, u64) = (width.parse().expect("a width"), height.parse().expect("a height"));
        let room = if rh > rw { (400, 600) } else { (600, 400) };
        assert!(
            width <= room.0 && height <= room.1,
            "{name} came back {size}, outside the 600x400 it was fitted inside"
        );
        let drift = (width * rh).abs_diff(height * rw);
        assert!(drift <= rw.max(rh), "{name} came back {size}, off {rw}:{rh}");
    }
}

#[tokio::test]
async fn an_unknown_preset_is_refused_with_the_names_that_work() {
    let response = call(post("/api/dither?preset=tiktok", "image/png", source_png())).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).expect("the error is JSON");
    let error = json["error"].as_str().expect("error is a string");
    assert!(
        error.contains("instagram-story"),
        "the message should list the names: {error}"
    );
}

#[tokio::test]
async fn crop_keeps_the_middle_of_a_photo_the_working_size_does_not_fit() {
    let uri = "/api/dither?width=40&height=40&crop=true";
    let response = call(post(uri, "image/png", banded_png())).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-image-size"], "40x40");

    // The 3:1 source is cropped to the square middle, which is the white band alone.
    let cropped = io::decode_rgb(&body_bytes(response).await).expect("the result decodes");
    assert!(
        cropped.pixels().all(|p| p.0.iter().all(|&c| c > 200)),
        "the crop should hold the white band only, got {:?}",
        cropped.get_pixel(0, 0)
    );

    // Without it the source is squashed into the square, black edges and all.
    let response = call(post("/api/dither?width=40&height=40", "image/png", banded_png())).await;
    let squashed = io::decode_rgb(&body_bytes(response).await).expect("the result decodes");
    assert!(squashed.pixels().any(|p| p.0.iter().all(|&c| c < 100)));
}

#[tokio::test]
async fn crop_from_chooses_which_part_is_kept() {
    // The band sits in the middle, so anything taken from an edge is the black surround instead.
    for origin in ["left", "right", "0,0", "80,0"] {
        let uri = format!("/api/dither?width=40&height=40&crop=true&crop_from={origin}");
        let response = call(post(&uri, "image/png", banded_png())).await;

        assert_eq!(response.status(), StatusCode::OK, "{origin} should be accepted");
        let cropped = io::decode_rgb(&body_bytes(response).await).expect("the result decodes");
        assert!(
            cropped.pixels().all(|p| p.0.iter().all(|&c| c < 100)),
            "from {origin}: expected the black edge, got {:?}",
            cropped.get_pixel(0, 0)
        );
    }

    // A name that means nothing is a 400 rather than a silent centre crop.
    let uri = "/api/dither?width=40&height=40&crop=true&crop_from=middle";
    let response = call(post(uri, "image/png", banded_png())).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).expect("the error is JSON");
    let error = json["error"].as_str().expect("error is a string");
    assert!(error.contains("X,Y"), "the message should give the syntax: {error}");
}

#[tokio::test]
async fn method_none_returns_the_framing_without_dithering_it() {
    // The band is grey rather than black or white, so the palette would be visible in the result.
    let grey = {
        let image = RgbImage::from_raw(4, 4, vec![120u8; 4 * 4 * 3]).expect("the buffer matches the size");
        io::encode_rgb_png(&image).expect("the test image encodes")
    };

    let response = call(post(
        "/api/dither?method=none&width=8&height=8",
        "image/png",
        grey.clone(),
    ))
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/png");
    assert_eq!(response.headers()["x-image-size"], "8x8");

    // Straight back at the working size, in its own colour, which no palette slot carries.
    let plain = io::decode_rgb(&body_bytes(response).await).expect("the result decodes");
    assert_eq!(plain.dimensions(), (8, 8));
    assert!(
        plain.pixels().all(|p| p.0 == [120, 120, 120]),
        "expected the photo untouched, got {:?}",
        plain.get_pixel(0, 0)
    );

    // The crop and the scale still apply, and the crop rect is still reported.
    let uri = "/api/dither?method=none&width=40&height=40&crop=true&crop_from=40,0&scale=2";
    let response = call(post(uri, "image/png", banded_png())).await;
    assert_eq!(response.headers()["x-image-size"], "80x80");
    assert_eq!(response.headers()["x-crop-rect"], "40,0,40,40");

    // `format` has no palette to choose, so it is the plain PNG either way.
    let uri = "/api/dither?method=none&width=8&height=8&format=indexed";
    let response = call(post(uri, "image/png", grey)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let plain = io::decode_rgb(&body_bytes(response).await).expect("the result decodes");
    assert!(plain.pixels().all(|p| p.0 == [120, 120, 120]));
}

#[tokio::test]
async fn the_panel_refuses_an_undithered_image() {
    let response = call(post("/api/buffer?method=none", "image/png", source_png())).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).expect("the error is JSON");
    let error = json["error"].as_str().expect("error is a string");
    assert!(
        error.contains("method=none"),
        "the message should name the cause: {error}"
    );
}

#[tokio::test]
async fn resize_false_still_crops_it_just_does_not_scale() {
    // The 120x40 source, framed square from 20 rows down: 20x20 of its own pixels, not scaled to 40x40.
    let uri = "/api/dither?method=none&resize=false&width=40&height=40&crop=true&crop_from=0,20";
    let response = call(post(uri, "image/png", banded_png())).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-crop-rect"], "0,20,20,20");
    assert_eq!(response.headers()["x-image-size"], "20x20");

    // Moving the corner moves the result, which is the whole point.
    let uri = "/api/dither?method=none&resize=false&width=40&height=40&crop=true&crop_from=40,0";
    let response = call(post(uri, "image/png", banded_png())).await;
    assert_eq!(response.headers()["x-crop-rect"], "40,0,40,40");
    let cropped = io::decode_rgb(&body_bytes(response).await).expect("the result decodes");
    assert!(
        cropped.pixels().all(|p| p.0.iter().all(|&c| c > 200)),
        "the corner should land on the white band, got {:?}",
        cropped.get_pixel(0, 0)
    );

    // Without a crop, `resize=false` still hands back the whole photo untouched.
    let uri = "/api/dither?method=none&resize=false";
    let response = call(post(uri, "image/png", banded_png())).await;
    assert_eq!(response.headers()["x-crop-rect"], "0,0,120,40");
    assert_eq!(response.headers()["x-image-size"], "120x40");
}

#[tokio::test]
async fn the_crop_rect_header_says_which_part_of_the_upload_was_read() {
    // No crop: the whole 120x40 photo, which also tells a client what the source measured.
    let response = call(post("/api/dither?width=40&height=40", "image/png", banded_png())).await;
    assert_eq!(response.headers()["x-crop-rect"], "0,0,120,40");

    // Cropped to the square middle.
    let uri = "/api/dither?width=40&height=40&crop=true";
    let response = call(post(uri, "image/png", banded_png())).await;
    assert_eq!(response.headers()["x-crop-rect"], "40,0,40,40");

    // The buffer route reports it too, on a size the panel takes.
    let response = call(post("/api/buffer?crop=true", "image/png", banded_png())).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-crop-rect"], "30,0,60,40");
}

#[tokio::test]
async fn a_crop_corner_is_kept_on_both_axes() {
    // The white band runs from x=40 to x=80, so a corner at 40,0 lands on it exactly.
    let uri = "/api/dither?width=40&height=40&crop=true&crop_from=40,0";
    let response = call(post(uri, "image/png", banded_png())).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-crop-rect"], "40,0,40,40");
    let cropped = io::decode_rgb(&body_bytes(response).await).expect("the result decodes");
    assert!(cropped.pixels().all(|p| p.0.iter().all(|&c| c > 200)));

    // A corner partway down keeps what is left below it. The y is never ignored, whatever the photo's shape: this is
    // the 3:1 source whose square crop would otherwise span the full height.
    let uri = "/api/dither?width=40&height=40&crop=true&crop_from=0,20";
    let response = call(post(uri, "image/png", banded_png())).await;
    assert_eq!(response.headers()["x-crop-rect"], "0,20,20,20");

    // Past the last pixel it keeps that pixel rather than sliding back to somewhere it was not asked for.
    let uri = "/api/dither?width=40&height=40&crop=true&crop_from=99999,99999";
    let response = call(post(uri, "image/png", banded_png())).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-crop-rect"], "119,39,1,1");
}

#[tokio::test]
async fn the_crop_zoom_shrinks_what_the_origin_settled_on() {
    // Centred, the square crop of a 3:1 photo is the 40x40 middle.
    let uri = "/api/dither?width=40&height=40&crop=true";
    let response = call(post(uri, "image/png", banded_png())).await;
    assert_eq!(response.headers()["x-crop-rect"], "40,0,40,40");

    // Zoomed, it keeps 20x20 of the same middle, still inside the white band.
    let uri = "/api/dither?width=40&height=40&crop=true&crop_zoom=2";
    let response = call(post(uri, "image/png", banded_png())).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-image-size"], "40x40");
    assert_eq!(response.headers()["x-crop-rect"], "50,10,20,20");

    let zoomed = io::decode_rgb(&body_bytes(response).await).expect("the result decodes");
    assert!(
        zoomed.pixels().all(|p| p.0.iter().all(|&c| c > 200)),
        "the zoomed crop sits inside the white band, got {:?}",
        zoomed.get_pixel(0, 0)
    );

    // Out of range is a 400 naming the bounds.
    let uri = "/api/dither?width=40&height=40&crop=true&crop_zoom=0.5";
    let response = call(post(uri, "image/png", banded_png())).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).expect("the error is JSON");
    let error = json["error"].as_str().expect("error is a string");
    assert!(error.contains("1.0"), "the message should give the range: {error}");
}

#[tokio::test]
async fn crop_from_without_crop_is_refused_rather_than_ignored() {
    let response = call(post("/api/dither?crop_from=top", "image/png", source_png())).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).expect("the error is JSON");
    let error = json["error"].as_str().expect("error is a string");
    assert!(
        error.contains("crop=true"),
        "the message should say what is missing: {error}"
    );

    // Adding the crop is all it takes.
    let response = call(post("/api/dither?crop_from=top&crop=true", "image/png", source_png())).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_portrait_upload_reaches_the_panel_without_a_rotation() {
    let response = call(post(
        "/api/buffer?keep_orientation=true",
        "image/png",
        sized_png(90, 120),
    ))
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-panel-orientation"], "panel");
    assert_eq!(response.headers()["x-image-size"], "400x600");

    let expected = (DISPLAY_PANEL_SIZE.0 * DISPLAY_PANEL_SIZE.1) as usize / 2;
    assert_eq!(body_bytes(response).await.len(), expected);
}

#[tokio::test]
async fn scale_multiplies_the_output() {
    let response = call(post("/api/dither?scale=2", "image/png", source_png())).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-image-size"], "1200x800");
}

#[tokio::test]
async fn the_buffer_endpoint_returns_a_packed_panel_frame() {
    let response = call(post("/api/buffer", "image/png", source_png())).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/octet-stream");
    assert_eq!(response.headers()["x-panel-orientation"], "rotated");

    // Two 4-bit codes per byte, over the whole portrait panel.
    let expected = (DISPLAY_PANEL_SIZE.0 * DISPLAY_PANEL_SIZE.1) as usize / 2;
    assert_eq!(body_bytes(response).await.len(), expected);
}

#[tokio::test]
async fn a_size_the_panel_cannot_show_is_refused() {
    let response = call(post("/api/buffer?width=320&height=240", "image/png", source_png())).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).expect("the error is JSON");
    assert!(
        json["error"].as_str().expect("error is a string").contains("400x600"),
        "the message should name the sizes the panel takes: {json}"
    );
}

#[tokio::test]
async fn bad_input_fails_as_json() {
    let cases = [
        ("/api/dither?saturation=3", "image/png", source_png()),
        ("/api/dither?bayer_size=5", "image/png", source_png()),
        ("/api/dither?nonsense=1", "image/png", source_png()),
        ("/api/dither", "image/png", Vec::new()),
        ("/api/dither", "image/png", b"not an image".to_vec()),
    ];

    for (uri, content_type, body) in cases {
        let response = call(post(uri, content_type, body)).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri} should be refused");

        let json: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).expect("the error is JSON");
        assert_eq!(json["status"], 400);
        assert!(json["error"].is_string(), "{uri} should explain itself");
    }
}

#[tokio::test]
async fn an_oversized_body_is_refused() {
    let config = Config {
        max_upload_bytes: 1024,
        ..Config::default()
    };
    let response = router(Arc::new(config))
        .oneshot(post("/api/dither", "image/png", source_png()))
        .await
        .expect("the router answers");

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
