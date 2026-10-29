//! Files: paths from clients, resolved beneath the user's scope, and the operations on them.

pub mod edit;
pub mod list;
pub mod mime;
pub mod ops;
pub mod path;
pub mod raw;
pub mod scope;
pub mod upload;

use axum::http::StatusCode;

use crate::http::error::ApiError;

#[derive(Debug)]
pub enum FsError {
    NotFound,
    Exists,
    NotAFolder,
    IsAFolder,
    InvalidPath,
    Outside,
    IntoItself,
    ScopeRoot,
    TooLarge(u64),
    Changed,
    NotText,
    Io(std::io::Error),
}

impl From<std::io::Error> for FsError {
    fn from(e: std::io::Error) -> FsError {
        match e.raw_os_error() {
            Some(code) if code == rustix::io::Errno::NOENT.raw_os_error() => FsError::NotFound,
            Some(code) if code == rustix::io::Errno::EXIST.raw_os_error() => FsError::Exists,
            Some(code) if code == rustix::io::Errno::NOTDIR.raw_os_error() => FsError::NotAFolder,
            Some(code) if code == rustix::io::Errno::ISDIR.raw_os_error() => FsError::IsAFolder,
            // From openat2 with RESOLVE_BENEATH: the path left the scope.
            Some(code) if code == rustix::io::Errno::XDEV.raw_os_error() => FsError::Outside,
            Some(code) if code == rustix::io::Errno::NAMETOOLONG.raw_os_error() => {
                FsError::InvalidPath
            }
            _ => FsError::Io(e),
        }
    }
}

impl From<rustix::io::Errno> for FsError {
    fn from(e: rustix::io::Errno) -> FsError {
        FsError::from(std::io::Error::from(e))
    }
}

impl From<FsError> for ApiError {
    fn from(e: FsError) -> ApiError {
        use StatusCode as S;
        let (status, code, message) = match e {
            FsError::NotFound => (
                S::NOT_FOUND,
                "not_found",
                "No such file or folder.".to_string(),
            ),
            FsError::Exists => (
                S::CONFLICT,
                "exists",
                "Something with that name is already there. Choose another name, or replace it."
                    .into(),
            ),
            FsError::NotAFolder => (
                S::BAD_REQUEST,
                "not_a_folder",
                "That is a file, not a folder.".into(),
            ),
            FsError::IsAFolder => (
                S::BAD_REQUEST,
                "is_a_folder",
                "That is a folder, not a file.".into(),
            ),
            FsError::InvalidPath => (
                S::BAD_REQUEST,
                "invalid_path",
                "That path is not valid.".into(),
            ),
            FsError::Outside => (
                S::FORBIDDEN,
                "outside",
                "That link leads outside your folder.".into(),
            ),
            FsError::IntoItself => (
                S::BAD_REQUEST,
                "into_itself",
                "A folder cannot go inside itself.".into(),
            ),
            FsError::ScopeRoot => (
                S::BAD_REQUEST,
                "scope_root",
                "Your own folder cannot be deleted, moved or renamed.".into(),
            ),
            FsError::TooLarge(n) => (
                S::PAYLOAD_TOO_LARGE,
                "too_large",
                format!("The file is larger than this server accepts ({n} bytes)."),
            ),
            FsError::Changed => (
                S::PRECONDITION_FAILED,
                "changed",
                "This file changed on the server since you opened it.".into(),
            ),
            FsError::NotText => (
                S::UNSUPPORTED_MEDIA_TYPE,
                "not_text",
                "This file is not text Cairn can edit.".into(),
            ),
            FsError::Io(err) => {
                tracing::error!(event = "file.io_error", error = %err);
                return ApiError::internal();
            }
        };
        ApiError::new(status, code, message)
    }
}
