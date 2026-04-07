use axum::extract::{DefaultBodyLimit, Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::api::middleware::auth::require_auth;
use crate::db::user_repo;
use crate::error::AppError;
use crate::services::file_ops;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct BrowseQuery {
    pub content: Option<bool>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Deserialize)]
pub struct RenameBody {
    pub destination: String,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Serialize)]
struct FileInfoResponse {
    #[serde(flatten)]
    entry: file_ops::FileEntry,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subtitles: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct MutationResponse {
    message: String,
}

async fn browse(
    State(state): State<AppState>,
    path: Option<Path<String>>,
    Query(query): Query<BrowseQuery>,
    Extension(_user): Extension<user_repo::User>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_path = path.map(|Path(p)| p).unwrap_or_default();
    let resolved = file_ops::safe_resolve(&state.canonical_root, &user_path)?;

    let metadata = tokio::fs::metadata(&resolved)
        .await
        .map_err(|_| AppError::NotFound("Path not found".into()))?;

    if metadata.is_dir() {
        let mut listing = file_ops::list_directory(
            &state.canonical_root,
            &resolved,
            state.config.max_listing_items,
        ).await?;

        let sort_field = query.sort.as_deref().unwrap_or("name");
        let ascending = query.order.as_deref().unwrap_or("asc") != "desc";

        listing.items.sort_by(|a, b| {
            match (a.is_dir, b.is_dir) {
                (true, false) => return std::cmp::Ordering::Less,
                (false, true) => return std::cmp::Ordering::Greater,
                _ => {}
            }

            let ord = match sort_field {
                "size" => a.size.cmp(&b.size),
                "modified" => a.modified.cmp(&b.modified),
                "type" => {
                    let ext_a = a.extension.as_deref().unwrap_or("");
                    let ext_b = b.extension.as_deref().unwrap_or("");
                    ext_a.to_lowercase().cmp(&ext_b.to_lowercase())
                }
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            };

            if ascending { ord } else { ord.reverse() }
        });

        Ok(Json(serde_json::to_value(&listing).map_err(AppError::Json)?))
    } else {
        let entry = file_ops::file_info(&state.canonical_root, &resolved).await?;

        let content = if query.content.unwrap_or(false) {
            let is_text = entry
                .mime_type
                .as_deref()
                .map(|m| {
                    m.starts_with("text/")
                        || m.contains("json")
                        || m.contains("xml")
                        || m.contains("javascript")
                        || m.contains("yaml")
                        || m.contains("toml")
                })
                .unwrap_or(false);

            if is_text {
                file_ops::read_text_content(&resolved).await.ok()
            } else {
                None
            }
        } else {
            None
        };

        let subtitles = if entry
            .mime_type
            .as_deref()
            .map(|m| m.starts_with("video/"))
            .unwrap_or(false)
        {
            let subs = file_ops::detect_subtitles(resolved.clone()).await;
            if subs.is_empty() { None } else { Some(subs) }
        } else {
            None
        };

        let response = FileInfoResponse {
            entry,
            content,
            subtitles,
        };

        Ok(Json(serde_json::to_value(&response).map_err(AppError::Json)?))
    }
}

async fn save_file(
    State(state): State<AppState>,
    Path(user_path): Path<String>,
    Extension(_user): Extension<user_repo::User>,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<MutationResponse>), AppError> {
    let resolved = file_ops::safe_resolve(&state.canonical_root, &user_path)?;

    file_ops::write_file(&resolved, &body).await?;

    Ok((
        StatusCode::OK,
        Json(MutationResponse {
            message: format!("File saved: {user_path}"),
        }),
    ))
}

async fn create(
    State(state): State<AppState>,
    Path(user_path): Path<String>,
    Extension(_user): Extension<user_repo::User>,
    Json(body): Json<CreateBody>,
) -> Result<(StatusCode, Json<MutationResponse>), AppError> {
    if body.kind != "directory" {
        return Err(AppError::BadRequest(
            "Only type \"directory\" is supported for creation".into(),
        ));
    }

    let resolved = file_ops::safe_resolve(&state.canonical_root, &user_path)?;

    file_ops::create_directory(&resolved).await?;

    Ok((
        StatusCode::CREATED,
        Json(MutationResponse {
            message: format!("Directory created: {user_path}"),
        }),
    ))
}

async fn remove(
    State(state): State<AppState>,
    Path(user_path): Path<String>,
    Extension(_user): Extension<user_repo::User>,
) -> Result<Json<MutationResponse>, AppError> {
    let resolved = file_ops::safe_resolve(&state.canonical_root, &user_path)?;

    file_ops::delete(&state.canonical_root, &resolved).await?;

    Ok(Json(MutationResponse {
        message: format!("Deleted: {user_path}"),
    }))
}

async fn rename_item(
    State(state): State<AppState>,
    Path(user_path): Path<String>,
    Extension(_user): Extension<user_repo::User>,
    Json(body): Json<RenameBody>,
) -> Result<Json<MutationResponse>, AppError> {
    let from = file_ops::safe_resolve(&state.canonical_root, &user_path)?;
    let to = file_ops::safe_resolve(&state.canonical_root, &body.destination)?;

    file_ops::rename(&from, &to, body.overwrite).await?;

    Ok(Json(MutationResponse {
        message: format!("Renamed {user_path} -> {}", body.destination),
    }))
}

pub fn routes(state: AppState) -> Router<AppState> {
    let max_upload = state.config.max_upload_bytes;

    Router::new()
        .route("/", get(browse))
        .route(
            "/{*path}",
            get(browse)
                .put(save_file)
                .post(create)
                .delete(remove)
                .patch(rename_item),
        )
        .route_layer(middleware::from_fn_with_state(state, require_auth))
        .layer(DefaultBodyLimit::max(max_upload))
}
