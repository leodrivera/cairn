//! Who is asking: the client's IP and whether the request came over HTTPS, read through trusted proxies only.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::{FromRef, FromRequestParts};
use axum::http::HeaderMap;
use axum::http::request::Parts;
use ipnet::IpNet;

use crate::http::AppState;

/// The TCP peer of a connection, put on every request by `serve_on`.
#[derive(Debug, Clone, Copy)]
pub struct Peer(pub SocketAddr);

#[derive(Debug, Clone, Copy)]
pub struct Client {
    pub ip: IpAddr,
    pub https: bool,
}

static WARNED: AtomicBool = AtomicBool::new(false);

impl Client {
    pub fn from_parts(peer: Option<IpAddr>, headers: &HeaderMap, trusted: &[IpNet]) -> Client {
        let Some(peer) = peer else {
            return Client {
                ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
                https: false,
            };
        };
        let is_trusted = |ip: &IpAddr| trusted.iter().any(|net| net.contains(ip));
        let forwarded = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok());
        if !is_trusted(&peer) {
            if forwarded.is_some() && !WARNED.swap(true, Ordering::Relaxed) {
                // Behind a proxy without CAIRN_TRUSTED_PROXIES every client looks like the proxy: one shared
                // rate limit, Fail2ban banning the proxy, cookies without Secure.
                tracing::warn!(
                    event = "http.forwarded_ignored",
                    peer = %peer,
                    hint = "requests come through a proxy that is not trusted; set CAIRN_TRUSTED_PROXIES to its network",
                );
            }
            return Client {
                ip: peer,
                https: false,
            };
        }
        // Walk from the right, the end our proxies wrote. The first address outside them is the client. An entry that
        // does not parse stops the walk at the last hop we trust: everything left of it came from the client.
        let mut ip = peer;
        for entry in forwarded
            .unwrap_or("")
            .rsplit(',')
            .map(str::trim)
            .filter(|e| !e.is_empty())
        {
            match parse_hop(entry) {
                Some(hop) if is_trusted(&hop) => ip = hop,
                Some(hop) => {
                    ip = hop;
                    break;
                }
                None => break,
            }
        }
        let https = headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("https"));
        Client { ip, https }
    }
}

/// An address, with or without a port: `203.0.113.9`, `203.0.113.9:5555`, `2001:db8::1`, `[2001:db8::1]:443`.
fn parse_hop(entry: &str) -> Option<IpAddr> {
    entry
        .parse()
        .ok()
        .or_else(|| entry.parse::<SocketAddr>().ok().map(|a| a.ip()))
        .or_else(|| entry.strip_prefix('[')?.strip_suffix(']')?.parse().ok())
}

impl<S> FromRequestParts<S> for Client
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = AppState::from_ref(state);
        let peer = parts.extensions.get::<Peer>().map(|p| p.0.ip());
        Ok(Client::from_parts(
            peer,
            &parts.headers,
            &app.config.trusted_proxies,
        ))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderMap;

    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    fn trusted() -> Vec<IpNet> {
        vec!["10.0.0.0/8".parse().unwrap()]
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn untrusted_peer_ignores_headers() {
        let h = headers(&[
            ("x-forwarded-for", "203.0.113.9"),
            ("x-forwarded-proto", "https"),
        ]);
        let c = Client::from_parts(Some(ip("198.51.100.7")), &h, &trusted());
        assert_eq!((c.ip, c.https), (ip("198.51.100.7"), false));
    }

    #[test]
    fn trusted_proxy_uses_forwarded_for() {
        let h = headers(&[
            ("x-forwarded-for", "203.0.113.9, 10.0.0.2"),
            ("x-forwarded-proto", "https"),
        ]);
        let c = Client::from_parts(Some(ip("10.0.0.1")), &h, &trusted());
        assert_eq!((c.ip, c.https), (ip("203.0.113.9"), true));
    }

    #[test]
    fn spoofed_leftmost_ignored() {
        let h = headers(&[("x-forwarded-for", "1.2.3.4, 203.0.113.9")]);
        let c = Client::from_parts(Some(ip("10.0.0.1")), &h, &trusted());
        assert_eq!(c.ip, ip("203.0.113.9"));
    }

    #[test]
    fn garbage_entry() {
        let h = headers(&[("x-forwarded-for", "nonsense")]);
        let c = Client::from_parts(Some(ip("10.0.0.1")), &h, &trusted());
        assert_eq!(c.ip, ip("10.0.0.1"));
    }

    #[test]
    fn no_peer_is_local() {
        let c = Client::from_parts(None, &HeaderMap::new(), &trusted());
        assert_eq!((c.ip, c.https), (ip("127.0.0.1"), false));
    }

    #[test]
    fn port_is_stripped() {
        let h = headers(&[("x-forwarded-for", "1.2.3.4, 203.0.113.9:5555")]);
        assert_eq!(
            Client::from_parts(Some(ip("10.0.0.1")), &h, &trusted()).ip,
            ip("203.0.113.9")
        );
        let h = headers(&[("x-forwarded-for", "[2001:db8::1]:443")]);
        assert_eq!(
            Client::from_parts(Some(ip("10.0.0.1")), &h, &trusted()).ip,
            ip("2001:db8::1")
        );
    }

    #[test]
    fn unreadable_entry_never_falls_back_to_the_client_written_part() {
        let h = headers(&[("x-forwarded-for", "1.2.3.4, nonsense")]);
        assert_eq!(
            Client::from_parts(Some(ip("10.0.0.1")), &h, &trusted()).ip,
            ip("10.0.0.1")
        );
        let h = headers(&[("x-forwarded-for", "1.2.3.4, nonsense, 10.0.0.2")]);
        assert_eq!(
            Client::from_parts(Some(ip("10.0.0.1")), &h, &trusted()).ip,
            ip("10.0.0.2")
        );
    }
}
