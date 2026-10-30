//! The front-end, embedded in the binary at build time.

use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

#[derive(rust_embed::Embed)]
#[folder = "web/dist/"]
struct Dist;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Br,
    Gzip,
    Identity,
}

/// The best encoding the client accepts among brotli and gzip: highest q, brotli on a tie, none on anything odd.
pub fn pick_encoding(accept: Option<&str>) -> Encoding {
    let (mut br, mut gzip, mut star) = (None, None, None);
    for token in accept.unwrap_or("").split(',') {
        let mut parts = token.split(';');
        let name = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let mut q = 1.0_f32;
        for param in parts {
            let Some((k, v)) = param.split_once('=') else {
                return Encoding::Identity;
            };
            if k.trim().eq_ignore_ascii_case("q") {
                match v.trim().parse::<f32>() {
                    Ok(n) if (0.0..=1.0).contains(&n) => q = n,
                    _ => return Encoding::Identity,
                }
            }
        }
        match name.as_str() {
            "br" => br = Some(q),
            "gzip" | "x-gzip" => gzip = Some(q),
            "*" => star = Some(q),
            _ => {}
        }
    }
    let br = br.or(star).unwrap_or(0.0);
    let gzip = gzip.or(star).unwrap_or(0.0);
    if br > 0.0 && br >= gzip {
        Encoding::Br
    } else if gzip > 0.0 {
        Encoding::Gzip
    } else {
        Encoding::Identity
    }
}

/// Paths that have pre-compressed copies in the build.
fn compressible(path: &str) -> bool {
    matches!(
        path.rsplit_once('.').map(|(_, ext)| ext),
        Some("html" | "js" | "css" | "svg")
    )
}

/// A file from the front-end build, or `index.html` for any other path (the front-end routes those).
/// Only GET and HEAD; compressed copies are chosen by `Accept-Encoding` and never served by their own name.
pub async fn serve(method: Method, uri: Uri, headers: HeaderMap) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, HeaderValue::from_static("GET, HEAD"))],
        )
            .into_response();
    }
    // The proxy matches the sign-in routes exactly; a variant (`/auth/sso/`, `/AUTH/SSO`) must not reach the app
    // page, or it would look like a route that skipped the broker.
    let lower = uri.path().to_ascii_lowercase();
    if lower == "/auth" || lower.starts_with("/auth/") {
        return crate::http::sso_routes::not_found();
    }
    let path = uri.path().trim_start_matches('/');
    let direct = !path.is_empty() && !path.ends_with(".br") && !path.ends_with(".gz");
    let path = if direct && Dist::get(path).is_some() {
        path
    } else {
        "index.html"
    };
    let Some(file) = Dist::get(path) else {
        return (
            StatusCode::NOT_FOUND,
            "The front-end is missing from this build.",
        )
            .into_response();
    };
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mut res = (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(mime(path))),
            (header::CACHE_CONTROL, HeaderValue::from_static(cache)),
        ],
        file.data,
    )
        .into_response();
    if compressible(path) {
        let accept = headers
            .get(header::ACCEPT_ENCODING)
            .and_then(|v| v.to_str().ok());
        let variant = match pick_encoding(accept) {
            Encoding::Br => Some(("br", ".br")),
            Encoding::Gzip => Some(("gzip", ".gz")),
            Encoding::Identity => None,
        };
        if let Some((name, ext)) = variant
            && let Some(packed) = Dist::get(&format!("{path}{ext}"))
        {
            *res.body_mut() = axum::body::Body::from(packed.data);
            res.headers_mut()
                .insert(header::CONTENT_ENCODING, HeaderValue::from_static(name));
        }
        res.headers_mut()
            .insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    }
    res
}

fn mime(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("png") => "image/png",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_br_first() {
        assert_eq!(pick_encoding(Some("gzip, deflate, br")), Encoding::Br);
    }

    #[test]
    fn q_zero_excludes() {
        assert_eq!(pick_encoding(Some("br;q=0, gzip")), Encoding::Gzip);
        assert_eq!(pick_encoding(Some("br;q=0, gzip;q=0")), Encoding::Identity);
    }

    #[test]
    fn star_counts() {
        assert_eq!(pick_encoding(Some("*")), Encoding::Br);
        assert_eq!(pick_encoding(Some("br;q=0, *")), Encoding::Gzip);
    }

    #[test]
    fn higher_q_wins() {
        assert_eq!(pick_encoding(Some("br;q=0.5, gzip;q=0.9")), Encoding::Gzip);
    }

    #[test]
    fn none_is_identity() {
        assert_eq!(pick_encoding(None), Encoding::Identity);
        assert_eq!(pick_encoding(Some("")), Encoding::Identity);
        assert_eq!(pick_encoding(Some("identity")), Encoding::Identity);
        assert_eq!(pick_encoding(Some("br;q=nonsense")), Encoding::Identity);
    }
}
