use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::db::user_repo;
use crate::error::AppError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// JWT Claims
// ---------------------------------------------------------------------------

/// JWT token claims embedded in every issued token.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    /// Subject -- the user's database ID.
    pub sub: i64,
    /// The user's role (e.g. "admin", "user").
    pub role: String,
    /// Expiration time as a UTC Unix timestamp.
    pub exp: usize,
    /// Issued-at time as a UTC Unix timestamp.
    pub iat: usize,
}

// ---------------------------------------------------------------------------
// Public helper functions (used by setup.rs and middleware)
// ---------------------------------------------------------------------------

/// Create a signed JWT for the given user.
pub fn create_token(
    user_id: i64,
    role: &str,
    secret: &[u8],
    expiry_hours: u64,
) -> Result<String, AppError> {
    let now = chrono::Utc::now().timestamp() as usize;
    let exp = now + (expiry_hours as usize) * 3600;

    let claims = Claims {
        sub: user_id,
        role: role.to_string(),
        exp,
        iat: now,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret),
    )
    .map_err(|e| AppError::Internal(format!("Token creation error: {e}")))?;

    Ok(token)
}

/// Validate a JWT and return the decoded claims.
pub fn validate_token(token: &str, secret: &[u8]) -> Result<Claims, AppError> {
    let validation = Validation::default();

    let token_data = decode::<Claims>(token, &DecodingKey::from_secret(secret), &validation)
        .map_err(|e| AppError::Unauthorized(format!("Invalid token: {e}")))?;

    Ok(token_data.claims)
}

/// Extract a bearer token from request headers.
///
/// Checks the `Authorization: Bearer <token>` header first, then falls back
/// to the `rustyfile_token` cookie.
pub fn extract_token(headers: &HeaderMap) -> Result<String, AppError> {
    // Try Authorization header first
    if let Some(auth_header) = headers.get("authorization") {
        let auth_str = auth_header
            .to_str()
            .map_err(|_| AppError::Unauthorized("Invalid Authorization header".into()))?;

        if let Some(token) = auth_str.strip_prefix("Bearer ") {
            let token = token.trim();
            if !token.is_empty() {
                return Ok(token.to_string());
            }
        }
    }

    // Fall back to cookie
    if let Some(cookie_header) = headers.get("cookie") {
        let cookie_str = cookie_header
            .to_str()
            .map_err(|_| AppError::Unauthorized("Invalid Cookie header".into()))?;

        for cookie in cookie_str.split(';') {
            let cookie = cookie.trim();
            if let Some(value) = cookie.strip_prefix("rustyfile_token=") {
                let value = value.trim();
                if !value.is_empty() {
                    return Ok(value.to_string());
                }
            }
        }
    }

    Err(AppError::Unauthorized(
        "No authentication token provided".into(),
    ))
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct AuthResponse {
    token: String,
    user: user_repo::User,
}

#[derive(Debug, Serialize)]
struct RefreshResponse {
    token: String,
}

#[derive(Debug, Serialize)]
struct LogoutResponse {
    message: String,
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

/// POST /auth/login -- authenticate with username and password.
async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<(StatusCode, Json<AuthResponse>), AppError> {
    let user = user_repo::find_by_username(&state.db, &body.username)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid username or password".into()))?;

    // Verify the password against the stored hash
    let parsed_hash = PasswordHash::new(&user.password_hash)
        .map_err(|e| AppError::Internal(format!("Password hash parse error: {e}")))?;

    Argon2::default()
        .verify_password(body.password.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::Unauthorized("Invalid username or password".into()))?;

    let token = create_token(
        user.id,
        &user.role,
        &state.jwt_secret,
        state.config.jwt_expiry_hours,
    )?;

    Ok((StatusCode::OK, Json(AuthResponse { token, user })))
}

/// POST /auth/logout -- placeholder that acknowledges logout.
async fn logout() -> Json<LogoutResponse> {
    Json(LogoutResponse {
        message: "Logged out".into(),
    })
}

/// POST /auth/refresh -- issue a fresh token from a valid existing token.
async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<RefreshResponse>, AppError> {
    let token = extract_token(&headers)?;
    let claims = validate_token(&token, &state.jwt_secret)?;

    let new_token = create_token(
        claims.sub,
        &claims.role,
        &state.jwt_secret,
        state.config.jwt_expiry_hours,
    )?;

    Ok(Json(RefreshResponse { token: new_token }))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/refresh", post(refresh))
}
