use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::api::auth;
use crate::db::user_repo;
use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Serialize)]
struct SetupStatusResponse {
    setup_required: bool,
}

#[derive(Debug, Deserialize)]
struct CreateAdminRequest {
    username: String,
    password: String,
    password_confirm: String,
}

#[derive(Debug, Serialize)]
struct CreateAdminResponse {
    token: String,
    user: user_repo::User,
}

async fn setup_status(State(state): State<AppState>) -> Json<SetupStatusResponse> {
    Json(SetupStatusResponse {
        setup_required: state.setup_guard.is_setup_required(),
    })
}

async fn create_admin(
    State(state): State<AppState>,
    Json(body): Json<CreateAdminRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    if !state.setup_guard.is_setup_allowed() {
        if !state.setup_guard.is_setup_required() {
            return Err(AppError::Conflict("Admin account already exists".into()));
        }
        return Err(AppError::SetupExpired);
    }

    // Double-check DB to guard against race conditions.
    if user_repo::admin_exists(&state.db).await? {
        state.setup_guard.mark_complete();
        return Err(AppError::Conflict("Admin account already exists".into()));
    }

    let username = body.username.trim();
    if username.len() < 3 || username.len() > 32 {
        return Err(AppError::BadRequest(
            "Username must be between 3 and 32 characters".into(),
        ));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(AppError::BadRequest(
            "Username may only contain letters, numbers, and underscores".into(),
        ));
    }

    // Max length prevents Argon2 DoS with extremely long passwords.
    if body.password.len() < state.config.min_password_length {
        return Err(AppError::BadRequest(format!(
            "Password must be at least {} characters",
            state.config.min_password_length
        )));
    }
    if body.password.len() > state.config.max_password_length {
        return Err(AppError::BadRequest(format!(
            "Password must not exceed {} characters",
            state.config.max_password_length
        )));
    }

    if body.password != body.password_confirm {
        return Err(AppError::BadRequest("Passwords do not match".into()));
    }

    let password_hash = user_repo::hash_password(&body.password)?;
    let user = match user_repo::create_user(&state.db, username, &password_hash, "admin").await {
        Ok(user) => user,
        Err(AppError::Database(ref e)) if e.to_string().contains("UNIQUE constraint failed") => {
            state.setup_guard.mark_complete();
            return Err(AppError::Conflict("Username already taken".into()));
        }
        Err(e) => return Err(e),
    };

    state.setup_guard.mark_complete();

    let token = auth::create_token(
        user.id,
        &user.role,
        &state.jwt_secret,
        state.config.jwt_expiry_hours,
    )?;

    let cookie = format!(
        "rustyfile_token={}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
        token,
        state.config.jwt_expiry_hours * 3600
    );

    Ok((
        StatusCode::CREATED,
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(CreateAdminResponse {
            token: token.clone(),
            user,
        }),
    ))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/status", get(setup_status))
        .route("/admin", post(create_admin))
}
