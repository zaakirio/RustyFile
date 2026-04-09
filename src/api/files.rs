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
use crate::services::search_index::SearchIndex;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CreateKind {
    Directory,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SortField {
    Name,
    Size,
    Modified,
    Type,
}

impl Default for SortField {
    fn default() -> Self {
        Self::Name
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct BrowseQuery {
    pub content: Option<bool>,
    #[serde(default)]
    pub sort: SortField,
    pub order: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateBody {
    #[serde(rename = "type")]
    pub kind: CreateKind,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RenameBody {
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
        let cache_key = resolved.to_string_lossy().into_owned();
        let root = state.canonical_root.clone();
        let max_items = state.config.max_listing_items;
        let resolved_clone = resolved.clone();

        let cached = state
            .dir_cache
            .get_or_insert(cache_key, || async {
                let listing = file_ops::list_directory(&root, &resolved_clone, max_items)
                    .await
                    .unwrap_or_else(|_| file_ops::DirListing {
                        path: String::new(),
                        items: Vec::new(),
                        num_dirs: 0,
                        num_files: 0,
                        total: None,
                        truncated: false,
                    });
                std::sync::Arc::new(listing)
            })
            .await;

        let mut listing = (*cached).clone();

        let sort_field = &query.sort;
        let ascending = query.order.as_deref().unwrap_or("asc") != "desc";

        listing.items.sort_by(|a, b| {
            match (a.is_dir, b.is_dir) {
                (true, false) => return std::cmp::Ordering::Less,
                (false, true) => return std::cmp::Ordering::Greater,
                _ => {}
            }

            let ord = match sort_field {
                SortField::Size => a.size.cmp(&b.size),
                SortField::Modified => a.modified.cmp(&b.modified),
                SortField::Type => {
                    let ext_a = a.extension.as_deref().unwrap_or("");
                    let ext_b = b.extension.as_deref().unwrap_or("");
                    ext_a.to_lowercase().cmp(&ext_b.to_lowercase())
                }
                SortField::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            };

            if ascending {
                ord
            } else {
                ord.reverse()
            }
        });

        Ok(Json(
            serde_json::to_value(&listing).map_err(AppError::Json)?,
        ))
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
            if subs.is_empty() {
                None
            } else {
                Some(subs)
            }
        } else {
            None
        };

        let response = FileInfoResponse {
            entry,
            content,
            subtitles,
        };

        Ok(Json(
            serde_json::to_value(&response).map_err(AppError::Json)?,
        ))
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

    if let Some(parent) = resolved.parent() {
        let key = parent.to_string_lossy().into_owned();
        state.dir_cache.invalidate(&key).await;
    }

    let indexer = state.search_indexer.clone();
    let idx_path = user_path.clone();
    tokio::spawn(async move {
        let _ = indexer.upsert(&idx_path).await;
    });

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
    let CreateKind::Directory = body.kind;

    let resolved = file_ops::safe_resolve(&state.canonical_root, &user_path)?;

    file_ops::create_directory(&resolved).await?;

    if let Some(parent) = resolved.parent() {
        let key = parent.to_string_lossy().into_owned();
        state.dir_cache.invalidate(&key).await;
    }

    let indexer = state.search_indexer.clone();
    let idx_path = user_path.clone();
    tokio::spawn(async move {
        let _ = indexer.upsert(&idx_path).await;
    });

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

    // Check before delete so we know whether to remove a prefix or single entry.
    let is_dir = tokio::fs::metadata(&resolved)
        .await
        .map(|m| m.is_dir())
        .unwrap_or(false);

    file_ops::delete(&state.canonical_root, &resolved).await?;

    if let Some(parent) = resolved.parent() {
        let key = parent.to_string_lossy().into_owned();
        state.dir_cache.invalidate(&key).await;
    }

    let indexer = state.search_indexer.clone();
    let idx_path = user_path.clone();
    tokio::spawn(async move {
        if is_dir {
            let _ = indexer.remove_prefix(&idx_path).await;
        } else {
            let _ = indexer.remove(&idx_path).await;
        }
    });

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

    if let Some(parent) = from.parent() {
        let key = parent.to_string_lossy().into_owned();
        state.dir_cache.invalidate(&key).await;
    }
    if let Some(parent) = to.parent() {
        let key = parent.to_string_lossy().into_owned();
        state.dir_cache.invalidate(&key).await;
    }

    let indexer = state.search_indexer.clone();
    let old_path = user_path.clone();
    let new_path = body.destination.clone();
    tokio::spawn(async move {
        let _ = indexer.rename_prefix(&old_path, &new_path).await;
    });

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
