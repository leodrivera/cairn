//! Grant management: `/api/grants`. Creating needs `share`; owners and admins manage given grants, a recipient may
//! leave one.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::fs::path::RelPath;
use crate::grants::{self, GrantRow, LiveGrant};
use crate::http::AppState;
use crate::http::client::Client;
use crate::http::current_user::CurrentUser;
use crate::http::error::ApiError;
use crate::http::files_routes::{FileUser, blocking};
use crate::http::json::{StrictJson, some};
use crate::users::{self, User};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/grants", get(list_given).post(create))
        .route("/grants/received", get(list_received))
        .route("/grants/{id}", patch(update).delete(delete))
}

/// Most e-mails one request may name.
const EMAILS_MAX: usize = 20;

fn forbidden() -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "forbidden",
        "You do not have permission to do this here.",
    )
}

fn no_such_grant() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "not_found", "No such shared item.")
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Permission {
    Read,
    Write,
}

fn perm(write: bool) -> &'static str {
    if write { "write" } else { "read" }
}

fn name_of(path: &str) -> String {
    RelPath::parse(path)
        .ok()
        .and_then(|p| p.name_lossy().map(|n| n.into_owned()))
        .unwrap_or_else(|| "/".into())
}

/// A grant as its owner (or an admin) sees it. `kind` is null while the item is not where the grant points.
fn given_json(
    s: &AppState,
    row: &GrantRow,
    owner: &User,
    recipient: &User,
    viewer: &User,
) -> Value {
    let path = match crate::shares::split_scope(&row.path, &viewer.scope) {
        Some(rel) => format!("/{rel}"),
        None => row.path.clone(),
    };
    let live = grants::live(
        &s.db,
        &s.config.root,
        s.reserved(),
        s.force_fallback,
        row.clone(),
        row.recipient_id,
        crate::now(),
    );
    let kind = live
        .as_ref()
        .map(|l| if l.is_folder() { "folder" } else { "file" });
    json!({
        "id": row.id,
        "name": name_of(&row.path),
        "kind": kind,
        "path": path,
        "permission": perm(row.write),
        "effective": perm(row.write && owner.perms.write),
        "expiresAt": row.expires_at,
        "createdAt": row.created_at,
        "owner": { "id": owner.id, "name": owner.name },
        "recipient": { "id": recipient.id, "name": recipient.name, "email": recipient.email },
    })
}

/// A live grant as its recipient sees it: never the owner's path.
pub fn received_json(g: &LiveGrant, recipient: &User) -> Value {
    json!({
        "id": g.row.id,
        "name": name_of(&g.row.path),
        "kind": if g.is_folder() { "folder" } else { "file" },
        "permission": perm(g.row.write),
        "effective": perm(g.write),
        "expiresAt": g.row.expires_at,
        "createdAt": g.row.created_at,
        "owner": { "id": g.owner.id, "name": g.owner.name },
        "recipient": { "id": recipient.id, "name": recipient.name, "email": recipient.email },
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Create {
    path: String,
    emails: Vec<String>,
    permission: Permission,
    #[serde(default)]
    expires_days: Option<i64>,
}

/// Trimmed, lower-cased, blanks dropped, each once, in the order typed.
fn normalize(emails: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in emails {
        let e = e.trim().to_lowercase();
        if !e.is_empty() && !out.contains(&e) {
            out.push(e);
        }
    }
    out
}

enum Found {
    Added(User),
    NoAccount,
    Myself,
}

async fn create(
    State(s): State<AppState>,
    client: Client,
    fu: FileUser,
    StrictJson(b): StrictJson<Create>,
) -> Result<Response, ApiError> {
    let owner = fu.user.clone();
    if fu.in_grant() || !owner.perms.share || !owner.perms.read {
        return Err(forbidden());
    }
    if b.permission == Permission::Write && !owner.perms.write {
        return Err(forbidden());
    }
    let emails = normalize(&b.emails);
    if b.emails.len() > EMAILS_MAX || emails.is_empty() {
        return Err(ApiError::bad_request(
            "Name between 1 and 20 e-mail addresses.",
        ));
    }
    let now = crate::now();
    let expires_at = crate::shares::expiry(b.expires_days, now)
        .map_err(|m| ApiError::new(StatusCode::BAD_REQUEST, "invalid", m))?;
    let rel = RelPath::parse(&b.path)?;
    let scope = fu.scope;
    let check = rel.clone();
    blocking(move || scope.open(&check, rustix::fs::OFlags::PATH).map(|_| ())).await?;
    let path = crate::shares::root_path(&owner.scope, &rel);

    // Every e-mail counts before it is looked up; one with an account gives its try back. Over the limit, the whole
    // request is refused and nothing is saved.
    let limiter = &s.limiters.grant_lookup;
    let key = format!("user:{}", owner.id);
    let mut found = Vec::with_capacity(emails.len());
    for email in &emails {
        if let Some(wait) = limiter.try_count(client.ip, &key, now) {
            tracing::warn!(
                event = "http.rate_limited",
                route = "grant_lookup",
                user = owner.id
            );
            return Err(ApiError::too_many(wait));
        }
        let user = users::by_email(&s.db, email)?.map(|(u, _)| u);
        if user.is_some() {
            limiter.undo(client.ip, &key);
        }
        found.push(match user {
            Some(u) if u.id == owner.id => Found::Myself,
            Some(u) => Found::Added(u),
            None => Found::NoAccount,
        });
    }
    let write = b.permission == Permission::Write;
    let rows = s.db.with(|c| {
        let tx = c.unchecked_transaction()?;
        let mut rows = Vec::new();
        for f in &found {
            if let Found::Added(u) = f {
                rows.push(grants::upsert(
                    &tx, owner.id, u.id, &path, write, expires_at, now,
                )?);
            }
        }
        tx.commit()?;
        Ok(rows)
    })?;
    let unknown = found
        .iter()
        .filter(|f| matches!(f, Found::NoAccount))
        .count();
    if unknown > 0 {
        tracing::info!(
            event = "grant.unknown_email",
            user = owner.id,
            count = unknown
        );
    }
    let mut rows = rows.into_iter();
    let mut results = Vec::with_capacity(emails.len());
    for (email, f) in emails.iter().zip(found) {
        results.push(match f {
            Found::Added(u) => {
                let row = rows.next().expect("one row per added");
                tracing::info!(event = "grant.create", owner = owner.id, recipient = u.id, path = %row.path, permission = perm(write));
                json!({ "email": email, "status": "added", "grant": given_json(&s, &row, &owner, &u, &owner) })
            }
            Found::NoAccount => json!({ "email": email, "status": "no_account", "message": format!("No account for {email}.") }),
            Found::Myself => json!({ "email": email, "status": "self", "message": "That is you." }),
        });
    }
    Ok(Json(json!({ "results": results })).into_response())
}

async fn list_given(State(s): State<AppState>, me: CurrentUser) -> Result<Json<Value>, ApiError> {
    let viewer = me.session.user;
    let who = (!viewer.perms.admin).then_some(viewer.id);
    let st = s.clone();
    let out = blocking(move || {
        let rows = grants::list_given(&st.db, who, crate::now()).map_err(db)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let (Ok(Some(owner)), Ok(Some(recipient))) = (
                users::by_id(&st.db, row.owner_id),
                users::by_id(&st.db, row.recipient_id),
            ) else {
                continue;
            };
            out.push(given_json(&st, row, &owner, &recipient, &viewer));
        }
        Ok(out)
    })
    .await?;
    Ok(Json(json!({ "grants": out })))
}

fn db(e: rusqlite::Error) -> crate::fs::FsError {
    crate::fs::FsError::Io(std::io::Error::other(e))
}

async fn list_received(
    State(s): State<AppState>,
    me: CurrentUser,
) -> Result<Json<Value>, ApiError> {
    let viewer = me.session.user;
    let out = blocking(move || {
        let now = crate::now();
        let rows = grants::list_received(&s.db, viewer.id, now).map_err(db)?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                grants::live(
                    &s.db,
                    &s.config.root,
                    s.reserved(),
                    s.force_fallback,
                    row,
                    viewer.id,
                    now,
                )
            })
            .map(|g| received_json(&g, &viewer))
            .collect::<Vec<_>>())
    })
    .await?;
    Ok(Json(json!({ "grants": out })))
}

/// The grant, when the viewer gave it or is an admin. Others get the same 404 as a missing one.
fn owned(s: &AppState, viewer: &User, id: &str) -> Result<GrantRow, ApiError> {
    grants::get(&s.db, id)?
        .filter(|r| r.expires_at.is_none_or(|at| at > crate::now()))
        .filter(|r| viewer.perms.admin || r.owner_id == viewer.id)
        .ok_or_else(no_such_grant)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Update {
    #[serde(default)]
    permission: Option<Permission>,
    #[serde(default, deserialize_with = "some")]
    expires_days: Option<Option<i64>>,
}

async fn update(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<String>,
    StrictJson(b): StrictJson<Update>,
) -> Result<Json<Value>, ApiError> {
    let viewer = me.session.user;
    let row = owned(&s, &viewer, &id)?;
    let owner = users::by_id(&s.db, row.owner_id)?.ok_or_else(no_such_grant)?;
    // Without `share` a person may still revoke their grants, but not keep them going.
    if !viewer.perms.admin && !viewer.perms.share {
        return Err(forbidden());
    }
    if b.permission == Some(Permission::Write) && !owner.perms.write {
        return Err(forbidden());
    }
    let now = crate::now();
    let expires_at = match b.expires_days {
        Some(days) => Some(
            crate::shares::expiry(days, now)
                .map_err(|m| ApiError::new(StatusCode::BAD_REQUEST, "invalid", m))?,
        ),
        None => None,
    };
    grants::update(
        &s.db,
        &id,
        b.permission.map(|p| p == Permission::Write),
        expires_at,
        now,
    )?;
    tracing::info!(event = "grant.update", owner = owner.id, recipient = row.recipient_id, path = %row.path);
    let row = grants::get(&s.db, &id)?.ok_or_else(no_such_grant)?;
    let recipient = users::by_id(&s.db, row.recipient_id)?.ok_or_else(no_such_grant)?;
    Ok(Json(
        json!({ "grant": given_json(&s, &row, &owner, &recipient, &viewer) }),
    ))
}

async fn delete(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let viewer = me.session.user;
    let row = grants::get(&s.db, &id)?
        .filter(|r| viewer.perms.admin || r.owner_id == viewer.id || r.recipient_id == viewer.id)
        .ok_or_else(no_such_grant)?;
    grants::delete(&s.db, &id, crate::now())?;
    let left = row.recipient_id == viewer.id;
    tracing::info!(
        event = "grant.delete",
        owner = row.owner_id,
        recipient = row.recipient_id,
        left
    );
    Ok(StatusCode::NO_CONTENT)
}
