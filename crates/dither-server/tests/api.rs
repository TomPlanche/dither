//! End-to-end tests over the router, driven in-process. No socket is bound.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use dither_server::{Config, router};
use reframe_dither::{DISPLAY_PANEL_SIZE, RgbImage, io};
use tower::ServiceExt;

/// A small PNG with enough colour variation to exercise the dither.
fn source_png() -> Vec<u8> {
    let (width, height) = (120u32, 90u32);
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
