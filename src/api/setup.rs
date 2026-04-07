use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::api::auth;
use crate::db::user_repo;
use crate::error::AppError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

/// GET /setup/status -- report whether initial setup is still required.
async fn setup_status(State(state): State<AppState>) -> Json<SetupStatusResponse> {
    Json(SetupStatusResponse {
        setup_required: state.setup_guard.is_setup_required(),
    })
}

/// POST /setup/admin -- create the initial admin user during setup.
async fn create_admin(
    State(state): State<AppState>,
    Json(body): Json<CreateAdminRequest>,
) -> Result<(StatusCode, Json<CreateAdminResponse>), AppError> {
    // Check whether setup is still allowed
    if !state.setup_guard.is_setup_allowed() {
        // Distinguish between "already set up" and "timed out"
        if !state.setup_guard.is_setup_required() {
            return Err(AppError::Conflict("Admin account already exists".into()));
        }
        return Err(AppError::SetupExpired);
    }

    // Race-condition guard: double-check the database directly
    if user_repo::admin_exists(&state.db).await? {
        state.setup_guard.mark_complete();
        return Err(AppError::Conflict("Admin account already exists".into()));
    }

    // ---- Validate input ----

    // Username: 3-32 characters, alphanumeric + underscore only
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

    // Password: minimum and maximum length from config.
    // Max length prevents Argon2 DoS with extremely long passwords
    // (pattern from Portainer's password policy enforcement).
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

    // Confirm passwords match
    if body.password != body.password_confirm {
        return Err(AppError::BadRequest("Passwords do not match".into()));
    }

    // ---- Create user ----
    let password_hash = user_repo::hash_password(&body.password)?;
    let user = match user_repo::create_user(&state.db, username, &password_hash, "admin").await {
        Ok(user) => user,
        Err(AppError::Database(ref e))
            if e.to_string().contains("UNIQUE constraint failed") =>
        {
            state.setup_guard.mark_complete();
            return Err(AppError::Conflict(
                "Username already taken".into(),
            ));
        }
        Err(e) => return Err(e),
    };

    // Mark setup as complete so no further admin creation is allowed
    state.setup_guard.mark_complete();

    // Generate JWT for auto-login
    let token = auth::create_token(
        user.id,
        &user.role,
        &state.jwt_secret,
        state.config.jwt_expiry_hours,
    )?;

    Ok((
        StatusCode::CREATED,
        Json(CreateAdminResponse { token, user }),
    ))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/status", get(setup_status))
        .route("/admin", post(create_admin))
}
