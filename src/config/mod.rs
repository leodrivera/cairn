//! Configuration from environment variables. Every problem is collected, so startup can report them all at once.

pub mod secret;

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use ipnet::IpNet;
use url::Url;

pub use secret::{Secret, ct_eq, read_secret};

pub type Vars = BTreeMap<String, String>;

#[derive(Debug, Clone)]
pub struct Config {
    pub root: PathBuf,
    pub data: PathBuf,
    pub addr: SocketAddr,
    pub public_url: Option<Url>,
    pub trusted_proxies: Vec<IpNet>,
    pub auth: AuthConfig,
    pub captcha: CaptchaConfig,
    /// `None` means no limit.
    pub upload_max: Option<u64>,
    pub edit_max: u64,
    pub log: LogLevel,
    pub dev_user: Option<String>,
    pub threads: usize,
    /// `CAIRN_TRASH_DAYS`: when set, it wins over the setting in the database.
    pub trash_days: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub password: bool,
    pub oauth: Option<OauthBroker>,
    pub oidc: Vec<OidcProvider>,
}

#[derive(Debug, Clone)]
pub struct OauthBroker {
    pub url: Url,
    pub secret: Secret,
    pub logout_url: Option<Url>,
}

#[derive(Debug, Clone)]
pub struct OidcProvider {
    pub id: String,
    pub name: String,
    pub issuer: Url,
    pub client_id: String,
    pub client_secret: Secret,
}

#[derive(Debug, Clone)]
pub enum CaptchaConfig {
    None,
    On {
        vendor: CaptchaVendor,
        site_key: String,
        secret: Secret,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptchaVendor {
    Recaptcha,
    Turnstile,
    Hcaptcha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// The `CAIRN_*` variables of this process. Empty values count as unset, as Compose writes `VAR=` for "no value".
pub fn vars_from_env() -> Vars {
    std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .filter(|(k, v)| k.starts_with("CAIRN_") && !v.is_empty())
        .collect()
}

const KNOWN: &[&str] = &[
    "CAIRN_ROOT",
    "CAIRN_DATA",
    "CAIRN_ADDR",
    "CAIRN_PUBLIC_URL",
    "CAIRN_TRUSTED_PROXIES",
    "CAIRN_AUTH_METHODS",
    "CAIRN_OAUTH_BROKER_URL",
    "CAIRN_OAUTH_SECRET",
    "CAIRN_OAUTH_SECRET_FILE",
    "CAIRN_OAUTH_LOGOUT_URL",
    "CAIRN_OIDC_PROVIDERS",
    "CAIRN_CAPTCHA",
    "CAIRN_CAPTCHA_SITE_KEY",
    "CAIRN_CAPTCHA_SECRET",
    "CAIRN_CAPTCHA_SECRET_FILE",
    "CAIRN_UPLOAD_MAX",
    "CAIRN_EDIT_MAX",
    "CAIRN_LOG",
    "CAIRN_DEV_USER",
    "CAIRN_THREADS",
    "CAIRN_TRASH_DAYS",
];

const OIDC_SUFFIXES: &[&str] = &[
    "ISSUER",
    "CLIENT_ID",
    "CLIENT_SECRET",
    "CLIENT_SECRET_FILE",
    "NAME",
];

/// Reads values and records problems, so one pass reports every problem.
struct Reader<'a> {
    vars: &'a Vars,
    errors: Vec<String>,
}

impl Reader<'_> {
    fn get(&self, name: &str) -> Option<&str> {
        self.vars
            .get(name)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    fn fail(&mut self, message: String) {
        self.errors.push(message);
    }

    fn list(&self, name: &str) -> Vec<String> {
        self.get(name)
            .map(|v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn path(&mut self, name: &str, default: &str) -> PathBuf {
        let path = PathBuf::from(self.get(name).unwrap_or(default));
        if !path.is_absolute() {
            self.fail(format!("{name}: must be an absolute path, like {default}."));
        }
        path
    }

    fn http_url(&mut self, name: &str) -> Option<Url> {
        let value = self.get(name)?;
        match Url::parse(value) {
            Ok(url) if matches!(url.scheme(), "http" | "https") && url.has_host() => Some(url),
            _ => {
                self.fail(format!(
                    "{name}: not an absolute http(s) URL. Use a URL like https://files.example.com."
                ));
                None
            }
        }
    }

    fn number(&mut self, name: &str, default: u64, ok: impl Fn(u64) -> bool, hint: &str) -> u64 {
        match self.get(name) {
            None => default,
            Some(v) => match v.parse::<u64>() {
                Ok(n) if ok(n) => n,
                _ => {
                    self.fail(format!("{name}: {hint}"));
                    default
                }
            },
        }
    }

    fn secret(&mut self, name: &str) -> Option<Secret> {
        read_secret(self.vars, name).unwrap_or_else(|e| {
            self.fail(e);
            None
        })
    }
}

impl Config {
    pub fn from_vars(vars: &Vars) -> Result<Config, Vec<String>> {
        let mut r = Reader {
            vars,
            errors: Vec::new(),
        };

        let root = r.path("CAIRN_ROOT", "/srv");
        let data = r.path("CAIRN_DATA", "/data");
        let addr = match r.get("CAIRN_ADDR").unwrap_or("0.0.0.0:8080").parse() {
            Ok(addr) => addr,
            Err(_) => {
                r.fail(
                    "CAIRN_ADDR: not an address and port. Use a value like 0.0.0.0:8080.".into(),
                );
                SocketAddr::from(([0, 0, 0, 0], 8080))
            }
        };
        let public_url = r.http_url("CAIRN_PUBLIC_URL");

        let mut trusted_proxies = Vec::new();
        for item in r.list("CAIRN_TRUSTED_PROXIES") {
            match item.parse::<IpNet>().or_else(|_| item.parse::<IpAddr>().map(IpNet::from)) {
                Ok(net) => trusted_proxies.push(net),
                Err(_) => r.fail(format!(
                    "CAIRN_TRUSTED_PROXIES: {item} is not an IP address or CIDR. Use values like 172.16.0.0/12."
                )),
            }
        }

        let mut methods = r.list("CAIRN_AUTH_METHODS");
        if r.get("CAIRN_AUTH_METHODS").is_none() {
            methods.push("password".into());
        }
        if methods.is_empty() {
            r.fail(
                "CAIRN_AUTH_METHODS: empty. Use password, oauth, oidc or a comma-separated mix."
                    .into(),
            );
        }
        for m in &methods {
            if !matches!(m.as_str(), "password" | "oauth" | "oidc") {
                r.fail(format!(
                    "CAIRN_AUTH_METHODS: unknown method {m}. Use password, oauth, oidc or a comma-separated mix."
                ));
            }
        }
        let has = |m: &str| methods.iter().any(|x| x == m);

        let vendor = match r.get("CAIRN_CAPTCHA").unwrap_or("none") {
            "none" => None,
            "recaptcha" => Some(CaptchaVendor::Recaptcha),
            "turnstile" => Some(CaptchaVendor::Turnstile),
            "hcaptcha" => Some(CaptchaVendor::Hcaptcha),
            other => {
                let other = other.to_string();
                r.fail(format!("CAIRN_CAPTCHA: unknown provider {other}. Use none, recaptcha, turnstile or hcaptcha."));
                None
            }
        };

        if (has("oauth") || has("oidc") || vendor.is_some())
            && public_url.is_none()
            && r.get("CAIRN_PUBLIC_URL").is_none()
        {
            r.fail("CAIRN_PUBLIC_URL: required when sign-in uses oauth, oidc or a CAPTCHA. Set it to the address people use, like https://files.example.com.".into());
        }

        let oauth = if has("oauth") {
            let url = r.http_url("CAIRN_OAUTH_BROKER_URL");
            if r.get("CAIRN_OAUTH_BROKER_URL").is_none() {
                r.fail("CAIRN_OAUTH_BROKER_URL: required with oauth. Set it to the broker's internal URL, like http://tinyauth:3000.".into());
            }
            let secret = r.secret("CAIRN_OAUTH_SECRET");
            match &secret {
                None if !r.errors.iter().any(|e| e.starts_with("CAIRN_OAUTH_SECRET")) => r.fail(
                    "CAIRN_OAUTH_SECRET: required with oauth. Set it, or CAIRN_OAUTH_SECRET_FILE, to the secret the broker sends as X-Auth-Secret.".into(),
                ),
                Some(s) if s.expose().chars().count() < 32 => {
                    let n = s.expose().chars().count();
                    r.fail(format!("CAIRN_OAUTH_SECRET: too short ({n} characters). Use at least 32."));
                }
                _ => {}
            }
            let logout_url = r.http_url("CAIRN_OAUTH_LOGOUT_URL");
            url.zip(secret).map(|(url, secret)| OauthBroker {
                url,
                secret,
                logout_url,
            })
        } else {
            None
        };

        let provider_ids = r.list("CAIRN_OIDC_PROVIDERS");
        let mut oidc = Vec::new();
        if has("oidc") {
            if provider_ids.is_empty() {
                r.fail(
                    "CAIRN_OIDC_PROVIDERS: required with oidc. List the provider ids, like google."
                        .into(),
                );
            }
            let mut seen = BTreeSet::new();
            for id in &provider_ids {
                if !valid_id(id) {
                    r.fail(format!(
                        "CAIRN_OIDC_PROVIDERS: invalid id {id}. Use lowercase letters, digits and -, up to 32 characters."
                    ));
                    continue;
                }
                if !seen.insert(id.clone()) {
                    r.fail(format!(
                        "CAIRN_OIDC_PROVIDERS: {id} is listed twice. List each provider once."
                    ));
                    continue;
                }
                if let Some(p) = oidc_provider(&mut r, id) {
                    oidc.push(p);
                }
            }
        }

        let captcha = match vendor {
            None => CaptchaConfig::None,
            Some(vendor) => {
                let site_key = r.get("CAIRN_CAPTCHA_SITE_KEY").map(String::from);
                if site_key.is_none() {
                    r.fail("CAIRN_CAPTCHA_SITE_KEY: required with a CAPTCHA. Set it to the site key from the vendor.".into());
                }
                let secret = r.secret("CAIRN_CAPTCHA_SECRET");
                if secret.is_none()
                    && !r
                        .errors
                        .iter()
                        .any(|e| e.starts_with("CAIRN_CAPTCHA_SECRET"))
                {
                    r.fail("CAIRN_CAPTCHA_SECRET: required with a CAPTCHA. Set it, or CAIRN_CAPTCHA_SECRET_FILE, to the secret key from the vendor.".into());
                }
                match (site_key, secret) {
                    (Some(site_key), Some(secret)) => CaptchaConfig::On {
                        vendor,
                        site_key,
                        secret,
                    },
                    _ => CaptchaConfig::None,
                }
            }
        };

        let upload_max = r.number(
            "CAIRN_UPLOAD_MAX",
            0,
            |_| true,
            "not a whole number of bytes. Use 0 for no limit, or a size like 10737418240.",
        );
        let edit_max = r.number(
            "CAIRN_EDIT_MAX",
            2_097_152,
            |n| n > 0,
            "must be a whole number of bytes above 0, like 2097152.",
        );
        let threads = r.number(
            "CAIRN_THREADS",
            1,
            |n| (1..=64).contains(&n),
            "must be a whole number from 1 to 64.",
        ) as usize;
        let trash_days = match r.get("CAIRN_TRASH_DAYS") {
            None => None,
            Some(v) => match v.parse::<u32>() {
                Ok(n) if n <= crate::trash::MAX_DAYS => Some(n),
                _ => {
                    r.fail("CAIRN_TRASH_DAYS: must be a whole number from 0 to 3650.".into());
                    None
                }
            },
        };

        let log = match r.get("CAIRN_LOG").unwrap_or("info") {
            "debug" => LogLevel::Debug,
            "info" => LogLevel::Info,
            "warn" => LogLevel::Warn,
            "error" => LogLevel::Error,
            other => {
                let other = other.to_string();
                r.fail(format!(
                    "CAIRN_LOG: unknown level {other}. Use debug, info, warn or error."
                ));
                LogLevel::Info
            }
        };

        let dev_user = r.get("CAIRN_DEV_USER").map(str::to_lowercase);
        if dev_user.is_some() {
            if !cfg!(debug_assertions) {
                r.fail("CAIRN_DEV_USER: only works in development builds. Unset it.".into());
            } else if !dev_user.as_deref().is_some_and(looks_like_email) {
                r.fail(
                    "CAIRN_DEV_USER: not an e-mail address. Use the e-mail of an existing user."
                        .into(),
                );
            }
        }

        let mut known: BTreeSet<String> = KNOWN.iter().map(|s| s.to_string()).collect();
        for id in &provider_ids {
            let prefix = oidc_prefix(id);
            known.extend(OIDC_SUFFIXES.iter().map(|s| format!("{prefix}{s}")));
        }
        for name in vars
            .keys()
            .filter(|k| k.starts_with("CAIRN_") && !known.contains(*k))
        {
            r.fail(format!(
                "{name}: unknown variable. Check the spelling against the list in the docs."
            ));
        }

        if !r.errors.is_empty() {
            return Err(r.errors);
        }
        Ok(Config {
            root,
            data,
            addr,
            public_url,
            trusted_proxies,
            auth: AuthConfig {
                password: has("password"),
                oauth,
                oidc,
            },
            captcha,
            upload_max: (upload_max > 0).then_some(upload_max),
            edit_max,
            log,
            dev_user,
            threads,
            trash_days,
        })
    }
}

fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    id.len() <= 32
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn oidc_prefix(id: &str) -> String {
    format!("CAIRN_OIDC_{}_", id.to_uppercase().replace('-', "_"))
}

fn oidc_provider(r: &mut Reader, id: &str) -> Option<OidcProvider> {
    let prefix = oidc_prefix(id);
    let issuer_var = format!("{prefix}ISSUER");
    let issuer = match r.get(&issuer_var).map(Url::parse) {
        None => {
            r.fail(format!(
                "{issuer_var}: required. Set it to the provider's issuer URL, like https://accounts.google.com."
            ));
            None
        }
        Some(Ok(url)) if url.scheme() == "https" && url.has_host() => Some(url),
        Some(Ok(url)) if url.scheme() == "http" => {
            r.fail(format!(
                "{issuer_var}: must start with https://. Use the provider's https issuer URL."
            ));
            None
        }
        Some(_) => {
            r.fail(format!(
                "{issuer_var}: not a URL. Use the provider's https issuer URL."
            ));
            None
        }
    };
    let id_var = format!("{prefix}CLIENT_ID");
    let client_id = r.get(&id_var).map(String::from);
    if client_id.is_none() {
        r.fail(format!(
            "{id_var}: required. Set it to the client id registered with the provider."
        ));
    }
    let secret_var = format!("{prefix}CLIENT_SECRET");
    let client_secret = r.secret(&secret_var);
    if client_secret.is_none() && !r.errors.iter().any(|e| e.starts_with(&secret_var)) {
        r.fail(format!(
            "{secret_var}: required. Set it, or {secret_var}_FILE, to the client secret registered with the provider."
        ));
    }
    let name = r.get(&format!("{prefix}NAME")).unwrap_or(id).to_string();
    Some(OidcProvider {
        id: id.to_string(),
        name,
        issuer: issuer?,
        client_id: client_id?,
        client_secret: client_secret?,
    })
}

fn looks_like_email(s: &str) -> bool {
    match s.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !s.chars().any(char::is_whitespace)
                && !domain.contains('@')
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vars {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn errors(pairs: &[(&str, &str)]) -> Vec<String> {
        Config::from_vars(&vars(pairs)).unwrap_err()
    }

    const PUBLIC_URL_MSG: &str = "CAIRN_PUBLIC_URL: required when sign-in uses oauth, oidc or a CAPTCHA. Set it to the address people use, like https://files.example.com.";

    #[test]
    fn trash_days_parsed() {
        for (v, want) in [("0", 0), ("3650", 3650), ("30", 30)] {
            let c = Config::from_vars(&vars(&[("CAIRN_TRASH_DAYS", v)])).unwrap();
            assert_eq!(c.trash_days, Some(want));
        }
        assert_eq!(Config::from_vars(&vars(&[])).unwrap().trash_days, None);
        for v in ["-1", "3651", "x"] {
            assert_eq!(
                errors(&[("CAIRN_TRASH_DAYS", v)]),
                vec!["CAIRN_TRASH_DAYS: must be a whole number from 0 to 3650.".to_string()],
                "{v}"
            );
        }
    }

    #[test]
    fn defaults() {
        let c = Config::from_vars(&Vars::new()).unwrap();
        assert_eq!(c.root, PathBuf::from("/srv"));
        assert_eq!(c.data, PathBuf::from("/data"));
        assert_eq!(c.addr, "0.0.0.0:8080".parse().unwrap());
        assert!(c.public_url.is_none());
        assert!(c.trusted_proxies.is_empty());
        assert!(c.auth.password && c.auth.oauth.is_none() && c.auth.oidc.is_empty());
        assert!(matches!(c.captcha, CaptchaConfig::None));
        assert_eq!((c.upload_max, c.edit_max, c.threads), (None, 2_097_152, 1));
        assert_eq!(c.log, LogLevel::Info);
        assert!(c.dev_user.is_none());
    }

    #[test]
    fn lists_with_spaces() {
        let c = Config::from_vars(&vars(&[
            ("CAIRN_AUTH_METHODS", "password, oidc"),
            ("CAIRN_PUBLIC_URL", "https://files.example.com"),
            ("CAIRN_OIDC_PROVIDERS", " google "),
            ("CAIRN_OIDC_GOOGLE_ISSUER", "https://accounts.google.com"),
            ("CAIRN_OIDC_GOOGLE_CLIENT_ID", "id"),
            ("CAIRN_OIDC_GOOGLE_CLIENT_SECRET", "s"),
            (
                "CAIRN_TRUSTED_PROXIES",
                "10.0.0.0/8, 172.16.0.0/12,192.168.1.1",
            ),
        ]))
        .unwrap();
        assert!(c.auth.password);
        assert_eq!(c.auth.oidc.len(), 1);
        let nets: Vec<String> = c.trusted_proxies.iter().map(|n| n.to_string()).collect();
        assert_eq!(nets, ["10.0.0.0/8", "172.16.0.0/12", "192.168.1.1/32"]);
    }

    #[test]
    fn oauth_needs_its_variables() {
        assert_eq!(
            errors(&[("CAIRN_AUTH_METHODS", "oauth")]),
            [
                PUBLIC_URL_MSG,
                "CAIRN_OAUTH_BROKER_URL: required with oauth. Set it to the broker's internal URL, like http://tinyauth:3000.",
                "CAIRN_OAUTH_SECRET: required with oauth. Set it, or CAIRN_OAUTH_SECRET_FILE, to the secret the broker sends as X-Auth-Secret.",
            ]
        );
    }

    #[test]
    fn short_oauth_secret() {
        let errs = errors(&[
            ("CAIRN_AUTH_METHODS", "oauth"),
            ("CAIRN_PUBLIC_URL", "https://files.example.com"),
            ("CAIRN_OAUTH_BROKER_URL", "http://tinyauth:3000"),
            ("CAIRN_OAUTH_SECRET", &"x".repeat(31)),
        ]);
        assert_eq!(
            errs,
            ["CAIRN_OAUTH_SECRET: too short (31 characters). Use at least 32."]
        );
    }

    #[test]
    fn oidc_provider() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"from-file\n").unwrap();
        let path = file.path().to_str().unwrap();
        let c = Config::from_vars(&vars(&[
            ("CAIRN_AUTH_METHODS", "oidc"),
            ("CAIRN_PUBLIC_URL", "https://files.example.com"),
            ("CAIRN_OIDC_PROVIDERS", "google,my-idp"),
            ("CAIRN_OIDC_GOOGLE_ISSUER", "https://accounts.google.com"),
            ("CAIRN_OIDC_GOOGLE_CLIENT_ID", "id"),
            ("CAIRN_OIDC_GOOGLE_CLIENT_SECRET", "s"),
            ("CAIRN_OIDC_GOOGLE_NAME", "Google"),
            ("CAIRN_OIDC_MY_IDP_ISSUER", "https://idp.example.com"),
            ("CAIRN_OIDC_MY_IDP_CLIENT_ID", "id2"),
            ("CAIRN_OIDC_MY_IDP_CLIENT_SECRET_FILE", path),
        ]))
        .unwrap();
        assert!(!c.auth.password);
        let [google, idp] = &c.auth.oidc[..] else {
            panic!("two providers expected")
        };
        assert_eq!(
            (
                google.id.as_str(),
                google.name.as_str(),
                google.client_id.as_str()
            ),
            ("google", "Google", "id")
        );
        assert_eq!(google.issuer.as_str(), "https://accounts.google.com/");
        assert_eq!((idp.id.as_str(), idp.name.as_str()), ("my-idp", "my-idp"));
        assert_eq!(idp.client_secret.expose(), "from-file");
    }

    #[test]
    fn oidc_http_issuer() {
        let errs = errors(&[
            ("CAIRN_AUTH_METHODS", "oidc"),
            ("CAIRN_PUBLIC_URL", "https://files.example.com"),
            ("CAIRN_OIDC_PROVIDERS", "google"),
            ("CAIRN_OIDC_GOOGLE_ISSUER", "http://idp.example.com"),
            ("CAIRN_OIDC_GOOGLE_CLIENT_ID", "id"),
            ("CAIRN_OIDC_GOOGLE_CLIENT_SECRET", "s"),
        ]);
        assert_eq!(
            errs,
            [
                "CAIRN_OIDC_GOOGLE_ISSUER: must start with https://. Use the provider's https issuer URL."
            ]
        );
    }

    #[test]
    fn captcha_needs_keys() {
        let errs = errors(&[("CAIRN_CAPTCHA", "recaptcha")]);
        assert_eq!(errs.len(), 3, "{errs:?}");
        assert!(errs.contains(&PUBLIC_URL_MSG.to_string()));
        assert!(
            errs.iter()
                .any(|e| e.starts_with("CAIRN_CAPTCHA_SITE_KEY: required"))
        );
        assert!(
            errs.iter()
                .any(|e| e.starts_with("CAIRN_CAPTCHA_SECRET: required"))
        );
    }

    #[test]
    fn captcha_on() {
        let c = Config::from_vars(&vars(&[
            ("CAIRN_CAPTCHA", "recaptcha"),
            ("CAIRN_PUBLIC_URL", "https://files.example.com"),
            ("CAIRN_CAPTCHA_SITE_KEY", "6Lc"),
            ("CAIRN_CAPTCHA_SECRET", "secret"),
        ]))
        .unwrap();
        assert!(matches!(
            c.captcha,
            CaptchaConfig::On {
                vendor: CaptchaVendor::Recaptcha,
                ..
            }
        ));
    }

    #[test]
    fn unknown_variable() {
        assert_eq!(
            errors(&[("CAIRN_CAPCHA", "x")]),
            ["CAIRN_CAPCHA: unknown variable. Check the spelling against the list in the docs."]
        );
    }

    #[test]
    fn secret_file_form_is_known() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"secret").unwrap();
        let c = Config::from_vars(&vars(&[
            ("CAIRN_CAPTCHA", "turnstile"),
            ("CAIRN_PUBLIC_URL", "https://files.example.com"),
            ("CAIRN_CAPTCHA_SITE_KEY", "k"),
            ("CAIRN_CAPTCHA_SECRET_FILE", file.path().to_str().unwrap()),
        ]));
        assert!(c.is_ok(), "{c:?}");
    }

    #[test]
    fn every_problem_reported() {
        let errs = errors(&[
            ("CAIRN_ADDR", "nope"),
            ("CAIRN_LOG", "loud"),
            ("CAIRN_THREADS", "0"),
        ]);
        assert_eq!(errs.len(), 3, "{errs:?}");
        for var in ["CAIRN_ADDR:", "CAIRN_LOG:", "CAIRN_THREADS:"] {
            assert!(
                errs.iter().any(|e| e.starts_with(var)),
                "{var} missing in {errs:?}"
            );
        }
    }

    #[test]
    fn unknown_auth_method() {
        let errs = errors(&[("CAIRN_AUTH_METHODS", "password,ldap")]);
        assert_eq!(
            errs,
            [
                "CAIRN_AUTH_METHODS: unknown method ldap. Use password, oauth, oidc or a comma-separated mix."
            ]
        );
    }

    #[test]
    fn debug_hides_secrets() {
        let secret = "x".repeat(32);
        let c = Config::from_vars(&vars(&[
            ("CAIRN_AUTH_METHODS", "oauth"),
            ("CAIRN_PUBLIC_URL", "https://files.example.com"),
            ("CAIRN_OAUTH_BROKER_URL", "http://tinyauth:3000"),
            ("CAIRN_OAUTH_SECRET", &secret),
        ]))
        .unwrap();
        assert!(!format!("{c:?}").contains(&secret));
    }
}
