//! The SvelteKit static build, embedded into the binary. In a debug build
//! rust-embed reads from disk, so a fresh `npm run build` is picked up live.

use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/build"]
#[allow_missing = true]
struct Assets;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    if let Some(f) = Assets::get(path) {
        return file(path, f);
    }
    // Client-side routes fall back to the SPA shell.
    let looks_like_file = path.rsplit('/').next().is_some_and(|last| last.contains('.'));
    if !looks_like_file && let Some(f) = Assets::get("index.html") {
        return file("index.html", f);
    }
    if Assets::iter().next().is_none() {
        return (
            StatusCode::NOT_FOUND,
            "The web UI is not built. Run `cargo xtask build`, or `cargo xtask dev` and open http://localhost:5173.",
        )
            .into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}

fn file(path: &str, f: rust_embed::EmbeddedFile) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache = if path.contains("/immutable/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    ([(header::CONTENT_TYPE, mime.as_ref()), (header::CACHE_CONTROL, cache)], f.data).into_response()
}
