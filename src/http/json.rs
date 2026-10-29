//! JSON request bodies: fixed types, unknown fields refused, every problem answered as an `ApiError`.

use axum::Json;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use serde::de::{Deserialize, DeserializeOwned, Deserializer};

use crate::http::error::ApiError;

pub struct StrictJson<T>(pub T);

impl<S, T> FromRequest<S> for StrictJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(StrictJson(value)),
            Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
                Err(ApiError::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "too_large",
                    "The request is too large.",
                ))
            }
            Err(_) => Err(ApiError::bad_request(
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
