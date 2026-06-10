pub mod archive;
pub mod auth;
pub mod download;
pub mod events;
pub mod files;
pub mod health;
pub mod hls;
pub mod middleware;
pub mod public_shares;
pub mod search;
pub mod setup;
pub mod shares;
pub mod thumbs;
pub mod tus;

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Redirect};
use axum::routing::get;
use axum::{middleware as axum_mw, Router};
use tower::timeout::TimeoutLayer;
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

const REQUEST_TIMEOUT_SECS: u64 = 30;
/// HLS segments wait on full ffmpeg transcodes and TUS PATCH reads whole
/// chunks from slow links, so those routes get a much longer timeout.
const SLOW_REQUEST_TIMEOUT_SECS: u64 = 300;

fn parse_trusted_proxies(config_value: &str) -> Option<Vec<IpAddr>> {
    let trimmed = config_value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let addrs: Vec<IpAddr> = trimmed
        .split(',')
        .filter_map(|s| s.trim().parse::<IpAddr>().ok())
        .collect();
    if addrs.is_empty() {
        None
    } else {
        Some(addrs)
    }
}

pub(crate) fn extract_client_ip(
    headers: &axum::http::HeaderMap,
    peer_addr: Option<SocketAddr>,
    trusted_proxies: &str,
) -> String {
    let peer_ip = peer_addr.map(|a| a.ip());
    let trusted = parse_trusted_proxies(trusted_proxies);

    let should_trust_headers = match (&trusted, peer_ip) {
        (None, _) => true,
        (Some(list), Some(ip)) => list.contains(&ip),
        (Some(_), None) => false,
    };

    if should_trust_headers {
        if let Some(forwarded) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|s| s.trim().to_string())
        {
            return forwarded;
        }
        if let Some(real_ip) = headers
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string())
        {
            return real_ip;
        }
    }

    peer_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub fn build_router(state: AppState) -> Router {
    let max_upload = state.config.max_upload_bytes;

    let cached_api_routes = Router::new()
        .nest("/health", health::routes())
        .nest("/setup", setup::routes())
        .nest("/auth", auth::routes())
        .nest(
            "/fs/search",
            search::routes(state.clone()).route_layer(axum_mw::from_fn_with_state(
                state.clone(),
                middleware::rate_limit::api_rate_limit,
            )),
        )
        .nest("/fs", files::routes(state.clone()))
        .nest("/shares", shares::routes(state.clone()))
        // Axum nest doesn't match trailing slash.
        .route("/fs/", get(|| async { Redirect::permanent("/api/fs") }))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache, no-store, must-revalidate"),
        ));

    // Download routes set their own Cache-Control, so they're layered separately.
    let download_routes = Router::new().nest("/fs/download", download::routes(state.clone()));

    let tus_routes = Router::new()
        .nest("/tus", tus::routes(state.clone()))
        .layer(DefaultBodyLimit::max(max_upload));

    let thumb_routes = Router::new()
        .nest("/thumbs", thumbs::routes(state.clone()))
        .route_layer(axum_mw::from_fn_with_state(
            state.clone(),
            middleware::rate_limit::api_rate_limit,
        ));

    let hls_routes = Router::new()
        .nest("/hls", hls::routes(state.clone()))
        .route_layer(axum_mw::from_fn_with_state(
            state.clone(),
            middleware::rate_limit::api_rate_limit,
        ));

    let cors = build_cors_layer(&state.config.cors_origins);

    let trusted_proxies = state.config.trusted_proxies.clone();
    let trace_layer =
        TraceLayer::new_for_http().make_span_with(move |request: &axum::http::Request<_>| {
            let peer_addr = request
                .extensions()
                .get::<axum::extract::ConnectInfo<SocketAddr>>()
                .map(|ci| ci.0);
            let client_ip = extract_client_ip(request.headers(), peer_addr, &trusted_proxies);
            tracing::info_span!(
                "request",
                method = %request.method(),
                uri = %request.uri(),
                client_ip = %client_ip,
            )
        });

    let secure_cookie = state.config.secure_cookie;
    let security_headers = move |r: Router| {
        let r = r
            .layer(SetResponseHeaderLayer::overriding(
                header::HeaderName::from_static("x-frame-options"),
                HeaderValue::from_static("DENY"),
            ))
            .layer(SetResponseHeaderLayer::overriding(
                header::HeaderName::from_static("x-content-type-options"),
                HeaderValue::from_static("nosniff"),
            ))
            .layer(SetResponseHeaderLayer::overriding(
                header::HeaderName::from_static("referrer-policy"),
                HeaderValue::from_static("strict-origin-when-cross-origin"),
            ))
            .layer(SetResponseHeaderLayer::overriding(
                header::HeaderName::from_static("permissions-policy"),
                HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
            ))
            .layer(SetResponseHeaderLayer::if_not_present(
                header::HeaderName::from_static("content-security-policy"),
                HeaderValue::from_static(
                    "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; img-src 'self' blob: data:; media-src 'self' blob:; connect-src 'self'; font-src 'self' https://fonts.gstatic.com; object-src 'none'; frame-ancestors 'none'",
                ),
            ));
        if secure_cookie {
            r.layer(SetResponseHeaderLayer::overriding(
                header::HeaderName::from_static("strict-transport-security"),
                HeaderValue::from_static("max-age=63072000; includeSubDomains"),
            ))
        } else {
            r
        }
    };

    let default_timeout = ServiceBuilder::new()
        .layer(axum::error_handling::HandleErrorLayer::new(
            |_: tower::BoxError| async { StatusCode::REQUEST_TIMEOUT.into_response() },
        ))
        .layer(TimeoutLayer::new(Duration::from_secs(REQUEST_TIMEOUT_SECS)));

    let slow_timeout = ServiceBuilder::new()
        .layer(axum::error_handling::HandleErrorLayer::new(
            |_: tower::BoxError| async { StatusCode::REQUEST_TIMEOUT.into_response() },
        ))
        .layer(TimeoutLayer::new(Duration::from_secs(
            SLOW_REQUEST_TIMEOUT_SECS,
        )));

    // Long-running routes (ffmpeg transcodes, slow-link uploads, archive
    // zip/extract) get their own timeout; everything else keeps the 30s
    // default.
    let slow_routes = Router::new()
        .nest("/api", tus_routes)
        .nest("/api", hls_routes)
        .nest("/api/archive", archive::routes(state.clone()))
        // Anonymous share links: downloads stream whole files/dir-zips and
        // drop uploads read large bodies, so the whole group gets the slow
        // timeout. NO auth middleware — access control is the share token
        // (+ optional password), rate-limited per IP inside.
        .nest("/api/public/shares", public_shares::routes(state.clone()))
        .layer(slow_timeout);

    // SSE stream is long-lived by design: no timeout layer at all (it still
    // gets the outer auth/CSRF/trace layers applied to the merged app).
    let event_routes = Router::new().nest("/api/events", events::routes(state.clone()));

    let standard_routes = Router::new()
        .nest("/api", download_routes)
        .nest("/api", thumb_routes)
        .nest("/api", cached_api_routes)
        .fallback(crate::frontend::static_handler)
        .layer(default_timeout);

    let app = standard_routes
        .merge(slow_routes)
        .merge(event_routes)
        .layer(axum_mw::from_fn_with_state(
            state.clone(),
            middleware::csrf::verify_origin,
        ))
        .layer(trace_layer)
        .layer(CompressionLayer::new())
        .layer(cors)
        .layer(DefaultBodyLimit::max(max_upload))
        .with_state(state);

    security_headers(app)
}

fn build_cors_layer(origins_config: &str) -> CorsLayer {
    let base = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
            Method::HEAD,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::RANGE,
            header::ACCEPT,
            header::HeaderName::from_static("upload-offset"),
            header::HeaderName::from_static("upload-length"),
            header::HeaderName::from_static("upload-metadata"),
            header::HeaderName::from_static("tus-resumable"),
        ])
        .expose_headers([
            header::HeaderName::from_static("upload-offset"),
            header::HeaderName::from_static("upload-length"),
            header::HeaderName::from_static("tus-resumable"),
            header::HeaderName::from_static("upload-expires"),
            header::LOCATION,
        ]);

    let trimmed = origins_config.trim();
    if trimmed == "*" {
        base.allow_origin(tower_http::cors::Any)
    } else if trimmed.is_empty() || trimmed == "same-origin" {
        base.allow_origin(AllowOrigin::list(Vec::<HeaderValue>::new()))
    } else {
        let origins: Vec<HeaderValue> = trimmed
            .split(',')
            .filter_map(|s| {
                let s = s.trim();
                if s.is_empty() {
                    None
                } else {
                    HeaderValue::from_str(s).ok()
                }
            })
            .collect();
        base.allow_origin(AllowOrigin::list(origins))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    fn peer(addr: &str) -> Option<SocketAddr> {
        Some(addr.parse().unwrap())
    }

    fn xff(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn no_trusted_proxies_trusts_headers() {
        let headers = xff("1.2.3.4");
        let ip = extract_client_ip(&headers, peer("10.0.0.1:9999"), "");
        assert_eq!(ip, "1.2.3.4");
    }

    #[test]
    fn xff_chain_uses_first_hop() {
        let headers = xff("1.2.3.4, 5.6.7.8, 9.10.11.12");
        let ip = extract_client_ip(&headers, peer("10.0.0.1:9999"), "10.0.0.1");
        assert_eq!(ip, "1.2.3.4");
    }

    #[test]
    fn untrusted_peer_ignores_headers() {
        let headers = xff("1.2.3.4");
        let ip = extract_client_ip(&headers, peer("192.168.1.5:9999"), "10.0.0.1");
        assert_eq!(ip, "192.168.1.5");
    }

    #[test]
    fn x_real_ip_used_when_no_xff() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", HeaderValue::from_static("1.2.3.4"));
        let ip = extract_client_ip(&headers, peer("10.0.0.1:9999"), "10.0.0.1");
        assert_eq!(ip, "1.2.3.4");
    }

    #[test]
    fn missing_headers_falls_back_to_peer() {
        let headers = HeaderMap::new();
        let ip = extract_client_ip(&headers, peer("203.0.113.7:1234"), "10.0.0.1");
        assert_eq!(ip, "203.0.113.7");
    }

    #[test]
    fn no_peer_with_trusted_proxies_is_unknown() {
        let headers = xff("1.2.3.4");
        let ip = extract_client_ip(&headers, None, "10.0.0.1");
        assert_eq!(ip, "unknown");
    }
}
