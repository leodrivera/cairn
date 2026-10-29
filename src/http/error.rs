//! API errors: a status, a stable code and a sentence for people, as `{"error": code, "message": sentence}`.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::users::UserError;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub retry_after: Option<i64>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError {
            status,
            code,
            message: message.into(),
            retry_after: None,
        }
    }

    pub fn bad_request(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    /// Signed out is 403, never 401: only a failed sign-in answers 401, so Fail2ban can count it.
    pub fn signed_out() -> ApiError {
        ApiError::new(StatusCode::FORBIDDEN, "signed_out", "Sign in first.")
    }

    pub fn forbidden() -> ApiError {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Only admins can do this.",
        )
    }

    pub fn not_found() -> ApiError {
        ApiError::new(StatusCode::NOT_FOUND, "not_found", "No such API route.")
    }

    pub fn conflict(code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::CONFLICT, code, message)
    }

    pub fn too_many(seconds: i64) -> ApiError {
        let minutes = (seconds + 59) / 60;
        let unit = if minutes == 1 { "minute" } else { "minutes" };
        ApiError {
            retry_after: Some(seconds),
            ..ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "too_many",
                format!("Too many tries in a row. Wait {minutes} {unit}, then try again."),
            )
        }
    }

    pub fn internal() -> ApiError {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Something went wrong on the server. The log has the details.",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut res = (
            self.status,
            Json(json!({ "error": self.code, "message": self.message })),
        )
            .into_response();
        if let Some(secs) = self.retry_after
            && let Ok(v) = HeaderValue::from_str(&secs.to_string())
        {
            res.headers_mut().insert(header::RETRY_AFTER, v);
        }
        res
    }
}

impl From<UserError> for ApiError {
    fn from(e: UserError) -> ApiError {
        let message = e.message();
        match e {
            UserError::Invalid(_) => ApiError::new(StatusCode::BAD_REQUEST, "invalid", message),
            UserError::EmailTaken => ApiError::conflict("email_taken", message),
            UserError::NotFound => ApiError::new(StatusCode::NOT_FOUND, "not_found", message),
            UserError::LastAdmin => ApiError::conflict("last_admin", message),
            UserError::Db(err) => {
                tracing::error!(event = "db.error", error = %err);
                ApiError::internal()
            }
        }
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(err: rusqlite::Error) -> ApiError {
        tracing::error!(event = "db.error", error = %err);
        ApiError::internal()
    }
}
