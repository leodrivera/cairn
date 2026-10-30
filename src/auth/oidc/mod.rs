//! OpenID Connect, run by Cairn itself: authorization code flow with PKCE and a verified ID token.

pub mod flow;
pub mod jwt;
pub mod provider;
pub use provider::{Oidc, Provider};
