//! The trash page's routes: `/api/trash`. Each person sees what was deleted from places they may write to now.

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::fs::FsError;
use crate::fs::path::RelPath;
use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::files_routes::{FileUser, blocking, failure};
use crate::http::json::StrictJson;
use crate::trash::{self, Entry, Via};
use crate::users::{self, User};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/trash", get(list).delete(delete_forever))
        .route("/trash/restore", post(restore))
        .route("/trash/empty", post(empty))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ids {
    ids: Vec<String>,
}

fn check_ids(ids: &[String]) -> Result<(), ApiError> {
    if ids.is_empty() || ids.len() > 10_000 {
        return Err(ApiError::bad_request("Send between 1 and 10000 items."));
    }
    Ok(())
}

/// An entry as `viewer` sees it: the folder it came from is shown beneath the viewer's own scope.
pub fn entry_json(s: &AppState, e: &Entry, via: &Via, viewer: &User) -> Value {
    let name = String::from_utf8_lossy(&e.name).into_owned();
    let from = RelPath::parse(&e.path)
        .ok()
        .and_then(|p| p.parent())
        .map(|p| format!("/{p}"))
        .map(|p| match crate::shares::split_scope(&p, &viewer.scope) {
            Some(rel) => format!("/{rel}"),
            None => p,
        })
        .unwrap_or_else(|| "/".into());
    let by = e
        .deleted_by
        .and_then(|id| users::by_id(&s.db, id).ok().flatten())
        .map(|u| json!({ "id": u.id, "name": u.name }));
    let mut v = json!({
        "id": e.id,
        "name": name,
        "kind": e.kind,
        "size": e.size,
        "from": from,
        "deletedBy": by,
        "deletedAt": e.deleted_at,
        "expiresAt": e.expires_at,
        "canDelete": *via == Via::Own,
    });
    if std::str::from_utf8(&e.name).is_err() {
        v["rawName"] = json!(RelPath::segment_for(&e.name));
    }
    v
}

async fn list(State(s): State<AppState>, fu: FileUser) -> Result<Json<Value>, ApiError> {
    let user = fu.user.clone();
    let st = s.clone();
    let rows = blocking(move || Ok(trash::visible(&st.db, &user))).await?;
    let entries: Vec<Value> = rows
        .iter()
        .map(|(e, via)| entry_json(&s, e, via, &fu.user))
        .collect();
    Ok(Json(json!({ "entries": entries })))
}

/// The visible entry with this id, and how the person reaches it.
fn find(s: &AppState, user: &User, id: &str) -> Option<(Entry, Via)> {
    trash::visible(&s.db, user)
        .into_iter()
        .find(|(e, _)| e.id == id)
}

async fn restore(
    State(s): State<AppState>,
    fu: FileUser,
    StrictJson(b): StrictJson<Ids>,
) -> Result<Json<Value>, ApiError> {
    check_ids(&b.ids)?;
    let (restored, failed) = blocking(move || {
        let (mut restored, mut failed) = (Vec::new(), Vec::new());
        for id in &b.ids {
            let result = find(&s, &fu.user, id)
                .ok_or(FsError::NotFound)
                .and_then(|(e, via)| match via {
                    Via::Own => trash::ops::restore(&s.trash, &s.db, &fu.scope, &fu.user.scope, &e),
                    Via::Grant(_) => Err(FsError::NotFound),
                });
            match result {
                Ok(at) => restored.push(json!({
                    "id": id,
                    "path": at.to_string(),
                    "name": at.name_lossy().map(|n| n.into_owned()),
                })),
                Err(e) => failed.push(failure("id", id, e)),
            }
        }
        Ok((restored, failed))
    })
    .await?;
    Ok(Json(json!({ "restored": restored, "failed": failed })))
}

async fn delete_forever(
    State(s): State<AppState>,
    fu: FileUser,
    StrictJson(b): StrictJson<Ids>,
) -> Result<Json<Value>, ApiError> {
    check_ids(&b.ids)?;
    let (deleted, failed) = blocking(move || {
        let (mut deleted, mut failed) = (0, Vec::new());
        for id in &b.ids {
            let result = match find(&s, &fu.user, id) {
                Some((e, Via::Own)) => trash::ops::remove(&s.trash, &s.db, &e),
                _ => Err(FsError::NotFound),
            };
            match result {
                Ok(()) => deleted += 1,
                Err(e) => failed.push(failure("id", id, e)),
            }
        }
        Ok((deleted, failed))
    })
    .await?;
    Ok(Json(json!({ "deleted": deleted, "failed": failed })))
}

async fn empty(State(s): State<AppState>, fu: FileUser) -> Result<Json<Value>, ApiError> {
    let deleted = blocking(move || {
        let mut deleted = 0;
        for (e, via) in trash::visible(&s.db, &fu.user) {
            if via == Via::Own && trash::ops::remove(&s.trash, &s.db, &e).is_ok() {
                deleted += 1;
            }
        }
        Ok(deleted)
    })
    .await?;
    Ok(Json(json!({ "deleted": deleted })))
}
