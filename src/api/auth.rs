use std::net::SocketAddr;

use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::db::user_repo;
use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct Claims {
    pub sub: i64,
    pub role: String,
    pub exp: u64,
    pub iat: u64,
    pub iss: String,
    pub aud: String,
}

pub(crate) fn create_token(
    user_id: i64,
    role: &str,
    secret: &[u8],
    expiry_hours: u64,
) -> Result<String, AppError> {
    let now = chrono::Utc::now().timestamp() as u64;
    let exp = now + expiry_hours * 3600;

    let claims = Claims {
        sub: user_id,
        role: role.to_string(),
        exp,
        iat: now,
        iss: "rustyfile".to_string(),
        aud: "rustyfile".to_string(),
    };

    let header = Header::new(jsonwebtoken::Algorithm::HS256);
    let token = encode(&header, &claims, &EncodingKey::from_secret(secret))
        .map_err(|e| AppError::Internal(format!("Token creation error: {e}")))?;

    Ok(token)
}

pub(crate) fn validate_token(
    token: &str,
    secret: &[u8],
    blocklist: Option<&crate::state::TokenBlocklist>,
) -> Result<Claims, AppError> {
    if let Some(bl) = blocklist {
        if bl.contains_key(token) {
            return Err(AppError::Unauthorized("Token has been revoked".into()));
        }
    }

    let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.set_issuer(&["rustyfile"]);
    validation.set_audience(&["rustyfile"]);

    let token_data = decode::<Claims>(token, &DecodingKey::from_secret(secret), &validation)
        .map_err(|_| AppError::Unauthorized("Invalid or expired token".into()))?;

    Ok(token_data.claims)
}

pub(crate) fn extract_token(headers: &HeaderMap) -> Result<String, AppError> {
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

pub(crate) fn build_auth_cookie(token: &str, jwt_expiry_hours: u64, secure: bool) -> String {
    let mut cookie = format!(
        "rustyfile_token={}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
        token,
        jwt_expiry_hours * 3600
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct AuthResponse {
    user: user_repo::User,
}

#[derive(Debug, Serialize)]
struct RefreshResponse {
    user: user_repo::User,
}

#[derive(Debug, Serialize)]
struct LogoutResponse {
    message: String,
}

async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let client_ip =
        crate::api::extract_client_ip(&headers, Some(peer_addr), &state.config.trusted_proxies);

    if state.login_limiter.check_key(&client_ip).is_err() {
        tracing::warn!(client_ip = %client_ip, "Login rate limit exceeded");
        return Err(AppError::TooManyRequests(
            "Too many login attempts. Please try again later.".into(),
        ));
    }

    let maybe_user = user_repo::find_by_username(&state.db, &body.username).await?;

    match maybe_user {
        Some(user) => {
            let parsed_hash = PasswordHash::new(&user.password_hash)
                .map_err(|e| AppError::Internal(format!("Password hash parse error: {e}")))?;

            if Argon2::default()
                .verify_password(body.password.as_bytes(), &parsed_hash)
                .is_err()
            {
                return Err(AppError::Unauthorized(
                    "Invalid username or password".into(),
                ));
            }

            let token = create_token(
                user.id,
                &user.role,
                &state.jwt_secret,
                state.config.jwt_expiry_hours,
            )?;

            let cookie = build_auth_cookie(
                &token,
                state.config.jwt_expiry_hours,
                state.config.secure_cookie,
            );

            Ok((
                StatusCode::OK,
                [(axum::http::header::SET_COOKIE, cookie)],
                Json(AuthResponse { user }),
            ))
        }
        None => {
            // Constant-time failure: verify against pre-hashed dummy.
            let parsed = PasswordHash::new(&state.dummy_hash).expect("Dummy hash is valid PHC");
            let _ = Argon2::default().verify_password(body.password.as_bytes(), &parsed);

            Err(AppError::Unauthorized(
                "Invalid username or password".into(),
            ))
        }
    }
}

async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl axum::response::IntoResponse {
    if let Ok(token) = extract_token(&headers) {
        state.token_blocklist.insert(token, ()).await;
    }

    let mut clear_cookie =
        "rustyfile_token=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0".to_string();
    if state.config.secure_cookie {
        clear_cookie.push_str("; Secure");
    }
    (
        [(axum::http::header::SET_COOKIE, clear_cookie)],
        Json(LogoutResponse {
            message: "Logged out".into(),
        }),
    )
}

async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let token = extract_token(&headers)?;
    let claims = validate_token(&token, &state.jwt_secret, Some(&state.token_blocklist))?;

    let user = user_repo::find_by_id(&state.db, claims.sub)
        .await?
        .ok_or_else(|| AppError::Unauthorized("User no longer exists".into()))?;

    let new_token = create_token(
        user.id,
        &user.role,
        &state.jwt_secret,
        state.config.jwt_expiry_hours,
    )?;

    state.token_blocklist.insert(token, ()).await;

    let cookie = build_auth_cookie(
        &new_token,
        state.config.jwt_expiry_hours,
        state.config.secure_cookie,
    );

    Ok((
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(RefreshResponse { user }),
    ))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/refresh", post(refresh))
}
