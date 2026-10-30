//! Server-side CAPTCHA check for the password form. reCAPTCHA v2, Turnstile and hCaptcha share one contract
//! (POST secret, response and remoteip; read success and hostname), so a vendor is a row of data. Fails closed.

use std::net::IpAddr;
use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::config::{CaptchaConfig, CaptchaVendor, Secret};

pub struct Captcha {
    vendor: CaptchaVendor,
    site_key: String,
    secret: Secret,
    hostname: String,
    verify_url: String,
    agent: ureq::Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptchaFailure {
    Missing,
    Invalid,
    Hostname,
    Unreachable,
}

impl CaptchaFailure {
    pub fn reason(&self) -> &'static str {
        match self {
            CaptchaFailure::Missing => "missing",
            CaptchaFailure::Invalid => "invalid",
            CaptchaFailure::Hostname => "hostname",
            CaptchaFailure::Unreachable => "unreachable",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            CaptchaFailure::Missing => "Confirm you are not a robot, then sign in again.",
            CaptchaFailure::Invalid | CaptchaFailure::Hostname => {
                "The anti-robot check failed. Try again."
            }
            CaptchaFailure::Unreachable => {
                "The anti-robot check could not be reached. Try again in a minute."
            }
        }
    }
}

#[derive(Deserialize)]
struct Answer {
    success: bool,
    #[serde(default)]
    hostname: Option<String>,
}

impl Captcha {
    /// `None` when CAPTCHA is off. The config check already made sure the public URL exists when it is on.
    pub fn from_config(c: &CaptchaConfig, public_url: Option<&Url>) -> Option<Captcha> {
        let CaptchaConfig::On {
            vendor,
            site_key,
            secret,
        } = c
        else {
            return None;
        };
        let verify_url = match vendor {
            CaptchaVendor::Recaptcha => "https://www.google.com/recaptcha/api/siteverify",
            CaptchaVendor::Turnstile => "https://challenges.cloudflare.com/turnstile/v0/siteverify",
            CaptchaVendor::Hcaptcha => "https://api.hcaptcha.com/siteverify",
        };
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .http_status_as_error(false)
            .tls_config(crate::tls::config())
            .build()
            .into();
        Some(Captcha {
            vendor: *vendor,
            site_key: site_key.clone(),
            secret: secret.clone(),
            hostname: public_url
                .and_then(Url::host_str)
                .unwrap_or_default()
                .to_string(),
            verify_url: verify_url.to_string(),
            agent,
        })
    }

    pub fn with_verify_url(self, url: String) -> Captcha {
        Captcha {
            verify_url: url,
            ..self
        }
    }

    pub fn vendor_name(&self) -> &'static str {
        match self.vendor {
            CaptchaVendor::Recaptcha => "recaptcha",
            CaptchaVendor::Turnstile => "turnstile",
            CaptchaVendor::Hcaptcha => "hcaptcha",
        }
    }

    pub fn site_key(&self) -> &str {
        &self.site_key
    }

    /// Hosts the widget needs: (script-src, frame-src).
    pub fn csp_hosts(&self) -> (&'static [&'static str], &'static [&'static str]) {
        match self.vendor {
            CaptchaVendor::Recaptcha => (
                &["https://www.google.com", "https://www.gstatic.com"],
                &["https://www.google.com"],
            ),
            CaptchaVendor::Turnstile => (
                &["https://challenges.cloudflare.com"],
                &["https://challenges.cloudflare.com"],
            ),
            CaptchaVendor::Hcaptcha => (
                &["https://hcaptcha.com", "https://*.hcaptcha.com"],
                &["https://hcaptcha.com", "https://*.hcaptcha.com"],
            ),
        }
    }

    pub async fn verify(&self, token: Option<&str>, ip: IpAddr) -> Result<(), CaptchaFailure> {
        let token = token
            .filter(|t| !t.is_empty())
            .ok_or(CaptchaFailure::Missing)?
            .to_string();
        let (agent, url, secret) = (
            self.agent.clone(),
            self.verify_url.clone(),
            self.secret.expose().to_string(),
        );
        let answer = tokio::task::spawn_blocking(move || -> Result<Answer, ureq::Error> {
            let ip = ip.to_string();
            let form = [
                ("secret", secret.as_str()),
                ("response", token.as_str()),
                ("remoteip", ip.as_str()),
            ];
            agent
                .post(&url)
                .send_form(form)?
                .body_mut()
                .read_json::<Answer>()
        })
        .await;
        let answer = match answer {
            Ok(Ok(answer)) => answer,
            Ok(Err(e)) => {
                tracing::warn!(event = "captcha.unreachable", vendor = self.vendor_name(), error = %e);
                return Err(CaptchaFailure::Unreachable);
            }
            Err(_) => return Err(CaptchaFailure::Unreachable),
        };
        if !answer.success {
            return Err(CaptchaFailure::Invalid);
        }
        if answer.hostname.as_deref() != Some(self.hostname.as_str()) || self.hostname.is_empty() {
            return Err(CaptchaFailure::Hostname);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use axum::routing::post;

    use super::*;
    use crate::config::{CaptchaConfig, CaptchaVendor, Config, Vars};

    fn config() -> Config {
        let vars: Vars = [
            ("CAIRN_CAPTCHA", "recaptcha"),
            ("CAIRN_PUBLIC_URL", "https://files.example.com"),
            ("CAIRN_CAPTCHA_SITE_KEY", "site-key"),
            ("CAIRN_CAPTCHA_SECRET", "the-secret"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        Config::from_vars(&vars).unwrap()
    }

    /// A vendor that answers `reply` after `delay` and keeps the last form it received.
    async fn fake(reply: &'static str, delay: Duration) -> (String, Arc<Mutex<String>>) {
        let seen = Arc::new(Mutex::new(String::new()));
        let keep = seen.clone();
        let app = axum::Router::new().route(
            "/verify",
            post(move |body: String| async move {
                *keep.lock().unwrap() = body;
                tokio::time::sleep(delay).await;
                ([("content-type", "application/json")], reply)
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/verify", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, seen)
    }

    fn captcha(url: String) -> Captcha {
        let c = config();
        Captcha::from_config(&c.captcha, c.public_url.as_ref())
            .unwrap()
            .with_verify_url(url)
    }

    const IP: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 9));

    #[test]
    fn off_is_none() {
        assert!(Captcha::from_config(&CaptchaConfig::None, None).is_none());
    }

    #[test]
    fn vendor_data() {
        let c = config();
        let cap = Captcha::from_config(&c.captcha, c.public_url.as_ref()).unwrap();
        assert!(matches!(
            c.captcha,
            CaptchaConfig::On {
                vendor: CaptchaVendor::Recaptcha,
                ..
            }
        ));
        assert_eq!(
            (cap.vendor_name(), cap.site_key()),
            ("recaptcha", "site-key")
        );
        assert_eq!(
            cap.csp_hosts().0,
            ["https://www.google.com", "https://www.gstatic.com"]
        );
    }

    #[tokio::test]
    async fn missing_token() {
        let (url, _) = fake(
            r#"{"success":true,"hostname":"files.example.com"}"#,
            Duration::ZERO,
        )
        .await;
        assert!(matches!(
            captcha(url).verify(None, IP).await,
            Err(CaptchaFailure::Missing)
        ));
    }

    #[tokio::test]
    async fn success_with_matching_host() {
        let (url, seen) = fake(
            r#"{"success":true,"hostname":"files.example.com"}"#,
            Duration::ZERO,
        )
        .await;
        assert!(captcha(url).verify(Some("tok"), IP).await.is_ok());
        let form = seen.lock().unwrap().clone();
        for part in ["secret=the-secret", "response=tok", "remoteip=203.0.113.9"] {
            assert!(form.contains(part), "{part} missing in {form}");
        }
    }

    #[tokio::test]
    async fn wrong_host() {
        let (url, _) = fake(
            r#"{"success":true,"hostname":"evil.example.net"}"#,
            Duration::ZERO,
        )
        .await;
        assert!(matches!(
            captcha(url).verify(Some("tok"), IP).await,
            Err(CaptchaFailure::Hostname)
        ));
    }

    #[tokio::test]
    async fn success_false() {
        let (url, _) = fake(r#"{"success":false}"#, Duration::ZERO).await;
        assert!(matches!(
            captcha(url).verify(Some("tok"), IP).await,
            Err(CaptchaFailure::Invalid)
        ));
    }

    #[tokio::test]
    async fn unreachable() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let url = format!("http://127.0.0.1:{port}/verify");
        assert!(matches!(
            captcha(url).verify(Some("tok"), IP).await,
            Err(CaptchaFailure::Unreachable)
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn slow_vendor() {
        let (url, _) = fake(
            r#"{"success":true,"hostname":"files.example.com"}"#,
            Duration::from_secs(7),
        )
        .await;
        let start = Instant::now();
        assert!(matches!(
            captcha(url).verify(Some("tok"), IP).await,
            Err(CaptchaFailure::Unreachable)
        ));
        assert!(start.elapsed() < Duration::from_secs(6));
    }
}
