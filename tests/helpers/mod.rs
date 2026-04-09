use std::sync::Arc;

use dashmap::DashMap;
use reqwest::Client;
use tempfile::TempDir;
use tokio::net::TcpListener;

use rustyfile::api::build_router;
use rustyfile::config::AppConfig;
use rustyfile::db::{create_pool, get_or_create_jwt_secret, run_migrations};
use rustyfile::services::search_index::SearchIndexer;
use rustyfile::services::SearchIndex;
use rustyfile::state::{AppState, SetupGuard};

#[allow(dead_code)]
pub struct TestApp {
    pub addr: String,
    pub client: Client,
    pub root_dir: TempDir,
    pub data_dir: TempDir,
    pub search_indexer: SearchIndexer,
}

#[allow(dead_code)]
impl TestApp {
    pub async fn spawn() -> Self {
        let root_dir = TempDir::new().expect("Failed to create root temp dir");
        let data_dir = TempDir::new().expect("Failed to create data temp dir");

        let config = AppConfig {
            host: "127.0.0.1".into(),
            port: 0,
            root: root_dir.path().to_string_lossy().to_string(),
            data_dir: data_dir.path().to_string_lossy().to_string(),
            log_level: "warn".into(),
            log_format: "pretty".into(),
            jwt_expiry_hours: 2,
            min_password_length: 10,
            setup_timeout_minutes: 5,
            cors_origins: "*".into(),
            max_upload_bytes: 50 * 1024 * 1024,
            max_password_length: 128,
            max_listing_items: 10_000,
            trusted_proxies: "".into(),
            cache_dir: data_dir.path().join("cache").to_string_lossy().to_string(),
            tus_expiry_hours: 24,
            secure_cookie: false,
        };

        let pool = create_pool(&config).expect("Failed to create pool");
        run_migrations(&pool)
            .await
            .expect("Failed to run migrations");

        let setup_guard = Arc::new(SetupGuard::new(config.setup_timeout_minutes));
        let jwt_secret = get_or_create_jwt_secret(&pool)
            .await
            .expect("Failed to get JWT secret");

        let canonical_root = std::path::PathBuf::from(&config.root)
            .canonicalize()
            .expect("Root temp dir must be canonicalizable");

        let login_limiter =
            rustyfile::state::new_login_limiter(std::num::NonZeroU32::new(100).unwrap(), 60);

        let dummy_hash = {
            use argon2::password_hash::SaltString;
            use argon2::PasswordHasher;
            let salt = SaltString::generate(&mut rand::rngs::OsRng);
            argon2::Argon2::default()
                .hash_password(b"rustyfile_dummy_timing_password", &salt)
                .expect("Failed to hash dummy password")
                .to_string()
        };

        let dir_cache = rustyfile::services::cache::DirCache::new(100, 30);

        let thumb_cache_dir = data_dir.path().join("cache").join("thumbs");
        std::fs::create_dir_all(&thumb_cache_dir).expect("Failed to create thumb cache dir");
        let thumb_worker =
            rustyfile::services::thumbnail::ThumbWorker::new(2, thumb_cache_dir, 300);

        let hls_dir = data_dir.path().join("cache").join("hls");
        std::fs::create_dir_all(&hls_dir).expect("Failed to create HLS cache dir");
        let transcoder = rustyfile::services::transcoder::HlsTranscoder::new(hls_dir, 2, 10);
        let hls_sources: Arc<DashMap<String, std::path::PathBuf>> = Arc::new(DashMap::new());

        let search_indexer = SearchIndexer::new(pool.clone(), canonical_root.clone());
        let search_indexer_for_test = search_indexer.clone();

        let state = AppState {
            db: pool,
            config,
            setup_guard,
            jwt_secret,
            canonical_root,
            login_limiter,
            dummy_hash,
            dir_cache,
            thumb_worker,
            transcoder,
            hls_sources,
            search_indexer,
        };

        let app = build_router(state);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind to ephemeral port");
        let addr = listener.local_addr().unwrap().to_string();

        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });

        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("Failed to build reqwest client");

        Self {
            addr,
            client,
            root_dir,
            data_dir,
            search_indexer: search_indexer_for_test,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub async fn create_admin(&self) -> String {
        let body = serde_json::json!({
            "username": "admin",
            "password": "supersecure1",
            "password_confirm": "supersecure1"
        });

        let resp = self
            .client
            .post(self.url("/api/setup/admin"))
            .json(&body)
            .send()
            .await
            .expect("Failed to create admin");

        assert_eq!(resp.status(), 201, "create_admin helper did not get 201");

        let json: serde_json::Value = resp.json().await.expect("Failed to parse admin response");
        json["token"]
            .as_str()
            .expect("No token in create_admin response")
            .to_string()
    }

    pub async fn reindex(&self) {
        self.search_indexer
            .full_reindex()
            .await
            .expect("Reindex failed in test");
    }

    pub fn write_file(&self, path: &str, content: &[u8]) {
        let full = self.root_dir.path().join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("Failed to create parent dirs");
        }
        std::fs::write(&full, content).expect("Failed to write file");
    }
}
