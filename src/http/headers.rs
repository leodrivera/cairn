//! Security headers on every response.

use axum::Router;
use axum::http::{HeaderName, HeaderValue, header};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::auth::captcha::Captcha;

/// The app's CSP. The app is one page, so when CAPTCHA is on the whole app allows the vendor's widget hosts; file
/// and share routes send a stricter CSP of their own, which this one never replaces.
pub fn csp(captcha: Option<&Captcha>) -> String {
    let (scripts, frames) = captcha.map(Captcha::csp_hosts).unwrap_or((&[], &[]));
    let mut script_src = String::from("'self'");
    for host in scripts {
        script_src.push(' ');
        script_src.push_str(host);
    }
    let frame_src = if frames.is_empty() {
        String::new()
    } else {
        format!("; frame-src {}", frames.join(" "))
    };
    format!(
        "default-src 'self'; img-src 'self' blob: data:; style-src 'self'; script-src {script_src}; \
         frame-ancestors 'none'; base-uri 'none'; form-action 'self'{frame_src}"
    )
}

pub fn apply(router: Router, captcha: Option<&Captcha>) -> Router {
    let set = |name: HeaderName, value: &'static str| {
        SetResponseHeaderLayer::overriding(name, HeaderValue::from_static(value))
    };
    let csp = HeaderValue::from_str(&csp(captcha)).expect("CSP is ASCII");
    router
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            csp,
        ))
        .layer(set(header::REFERRER_POLICY, "no-referrer"))
        .layer(set(
            HeaderName::from_static("cross-origin-opener-policy"),
            "same-origin",
        ))
        .layer(set(header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use tower::ServiceExt;

    use super::*;

    fn recaptcha() -> crate::auth::captcha::Captcha {
        let vars: crate::config::Vars = [
            ("CAIRN_CAPTCHA", "recaptcha"),
            ("CAIRN_PUBLIC_URL", "https://files.example.com"),
            ("CAIRN_CAPTCHA_SITE_KEY", "k"),
            ("CAIRN_CAPTCHA_SECRET", "s"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let c = crate::config::Config::from_vars(&vars).unwrap();
        crate::auth::captcha::Captcha::from_config(&c.captcha, c.public_url.as_ref()).unwrap()
    }

    #[test]
    fn csp_without_captcha_unchanged() {
        assert_eq!(
            csp(None),
            "default-src 'self'; img-src 'self' blob: data:; style-src 'self'; script-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"
        );
    }

    #[test]
    fn csp_with_recaptcha() {
        let csp = csp(Some(&recaptcha()));
        assert!(
            csp.contains("script-src 'self' https://www.google.com https://www.gstatic.com;"),
            "{csp}"
        );
        assert!(csp.contains("frame-src https://www.google.com"), "{csp}");
        assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
    }

    #[tokio::test]
    async fn route_csp_is_kept() {
        let strict = "default-src 'none'; sandbox";
        let app = apply(
            Router::new().route(
                "/raw",
                get(move || async move { ([(header::CONTENT_SECURITY_POLICY, strict)], "x") }),
            ),
            None,
        );
        let res = app
            .oneshot(Request::get("/raw").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.headers()[header::CONTENT_SECURITY_POLICY], strict);
    }
}
