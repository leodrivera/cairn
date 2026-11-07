//! JSON request bodies: fixed types, unknown fields refused, every problem answered as an `ApiError`.

use std::time::Duration;

use axum::Json;
use axum::extract::{FromRequest, FromRequestParts, Path, Request};
use axum::http::StatusCode;
use axum::http::request::Parts;
use serde::de::{Deserialize, DeserializeOwned, Deserializer};

use crate::http::error::ApiError;

/// A JSON body is small: it arrives in full within this time, or the request ends.
const BODY_WAIT: Duration = Duration::from_secs(30);

/// The most a route open to anyone reads: a sign-in or a link password, with a CAPTCHA token. Vendors set no common
/// limit on tokens, so this leaves room for them to grow, and a body waiting for its end holds little memory.
pub const ANONYMOUS_BODY: usize = 64 * 1024;

pub struct StrictJson<T>(pub T);

impl<S, T> FromRequest<S> for StrictJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        let body = tokio::time::timeout(BODY_WAIT, Json::<T>::from_request(req, state)).await;
        match body {
            Err(_) => Err(ApiError::new(
                StatusCode::REQUEST_TIMEOUT,
                "body_timeout",
                "The request took too long to arrive. Try again.",
            )),
            Ok(Ok(Json(value))) => Ok(StrictJson(value)),
            Ok(Err(rejection)) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
                Err(ApiError::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "too_large",
                    "The request is too large.",
                ))
            }
            Ok(Err(_)) => Err(ApiError::bad_request(
                "The request body is not valid JSON for this action.",
            )),
        }
    }
}

/// For fields where absent leaves a value alone and `null` clears it: `Option<Option<T>>` with `#[serde(default)]`.
pub fn some<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

/// A numeric id from the path, like `/api/users/{id}`; anything else is a JSON `400`, not axum's plain text.
pub struct IdPath(pub i64);

impl<S: Send + Sync> FromRequestParts<S> for IdPath {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        Path::<i64>::from_request_parts(parts, state)
            .await
            .map(|Path(id)| IdPath(id))
            .map_err(|_| ApiError::bad_request("The id in the address must be a number."))
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::header;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_body_that_stops_arriving_is_answered() {
        let body = Body::from_stream(futures_util::stream::pending::<
            Result<&'static [u8], std::io::Error>,
        >());
        let req = axum::http::Request::builder()
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .unwrap();
        let read = StrictJson::<serde_json::Value>::from_request(req, &());
        let answer = tokio::time::timeout(Duration::from_secs(60), read)
            .await
            .expect("still waiting");
        assert_eq!(answer.err().unwrap().status, StatusCode::REQUEST_TIMEOUT);
    }
}
