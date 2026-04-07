use std::path::PathBuf;

use axum::extract::{Extension, Path, Query, State};
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

// ---------------------------------------------------------------------------
// Query / body types
// ---------------------------------------------------------------------------

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
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET / or GET /*path -- Browse a directory or inspect a file.
async fn browse(
    State(state): State<AppState>,
    path: Option<Path<String>>,
    Query(query): Query<BrowseQuery>,
    Extension(_user): Extension<user_repo::User>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_path = path.map(|Path(p)| p).unwrap_or_default();
    let root = PathBuf::from(&state.config.root);
    let resolved = file_ops::safe_resolve(&root, &user_path)?;

    let metadata = tokio::fs::metadata(&resolved)
        .await
        .map_err(|_| AppError::NotFound(format!("Path not found: {user_path}")))?;

    if metadata.is_dir() {
        let mut listing = file_ops::list_directory(&root, &resolved).await?;

        // Sort items: directories first, then by the requested field.
        let sort_field = query.sort.as_deref().unwrap_or("name");
        let ascending = query.order.as_deref().unwrap_or("asc") != "desc";

        listing.items.sort_by(|a, b| {
            // Directories always come first.
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
                // Default: sort by name, case-insensitive.
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            };

            if ascending { ord } else { ord.reverse() }
        });

        Ok(Json(serde_json::to_value(&listing).unwrap()))
    } else {
        // Single file info.
        let entry = file_ops::file_info(&root, &resolved).await?;

        let content = if query.content.unwrap_or(false) {
            // Only try text content for text-like MIME types.
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

        // Detect subtitles for video files.
        let subtitles = entry
            .mime_type
            .as_deref()
            .filter(|m| m.starts_with("video/"))
            .map(|_| file_ops::detect_subtitles(&resolved))
            .filter(|s| !s.is_empty());

        let response = FileInfoResponse {
            entry,
            content,
            subtitles,
        };

        Ok(Json(serde_json::to_value(&response).unwrap()))
    }
}

/// PUT /*path -- Save file content (body = raw bytes).
async fn save_file(
    State(state): State<AppState>,
    Path(user_path): Path<String>,
    Extension(_user): Extension<user_repo::User>,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<MutationResponse>), AppError> {
    let root = PathBuf::from(&state.config.root);
    let resolved = file_ops::safe_resolve(&root, &user_path)?;

    file_ops::write_file(&resolved, &body).await?;

    Ok((
        StatusCode::OK,
        Json(MutationResponse {
            message: format!("File saved: {user_path}"),
        }),
    ))
}

/// POST /*path -- Create a directory.
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

    let root = PathBuf::from(&state.config.root);
    let resolved = file_ops::safe_resolve(&root, &user_path)?;

    file_ops::create_directory(&resolved).await?;

    Ok((
        StatusCode::CREATED,
        Json(MutationResponse {
            message: format!("Directory created: {user_path}"),
        }),
    ))
}

/// DELETE /*path -- Delete a file or directory.
async fn remove(
    State(state): State<AppState>,
    Path(user_path): Path<String>,
    Extension(_user): Extension<user_repo::User>,
) -> Result<Json<MutationResponse>, AppError> {
    let root = PathBuf::from(&state.config.root);
    let resolved = file_ops::safe_resolve(&root, &user_path)?;

    file_ops::delete(&resolved).await?;

    Ok(Json(MutationResponse {
        message: format!("Deleted: {user_path}"),
    }))
}

/// PATCH /*path -- Rename / move a file or directory.
async fn rename_item(
    State(state): State<AppState>,
    Path(user_path): Path<String>,
    Extension(_user): Extension<user_repo::User>,
    Json(body): Json<RenameBody>,
) -> Result<Json<MutationResponse>, AppError> {
    let root = PathBuf::from(&state.config.root);
    let from = file_ops::safe_resolve(&root, &user_path)?;
    let to = file_ops::safe_resolve(&root, &body.destination)?;

    file_ops::rename(&from, &to).await?;

    Ok(Json(MutationResponse {
        message: format!("Renamed {user_path} -> {}", body.destination),
    }))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn routes(state: AppState) -> Router<AppState> {
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
}
