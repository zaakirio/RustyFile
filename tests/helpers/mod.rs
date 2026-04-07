use std::sync::Arc;

use reqwest::Client;
use tempfile::TempDir;
use tokio::net::TcpListener;

use rustyfile::api::build_router;
use rustyfile::config::AppConfig;
use rustyfile::db::{create_pool, get_or_create_jwt_secret, run_migrations};
use rustyfile::state::{AppState, SetupGuard};

/// A self-contained test application.
///
/// Each test gets its own temp directories, database, and ephemeral port so that
/// tests can run in parallel without interfering with each other.
#[allow(dead_code)]
pub struct TestApp {
    pub addr: String,
    pub client: Client,
    pub root_dir: TempDir,
    pub data_dir: TempDir,
}

#[allow(dead_code)]
impl TestApp {
    /// Spin up a fully-initialised server on an OS-assigned port.
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
        };

        let pool = create_pool(&config).expect("Failed to create pool");
        run_migrations(&pool).await.expect("Failed to run migrations");

        let setup_guard = Arc::new(SetupGuard::new(config.setup_timeout_minutes));
        let jwt_secret = get_or_create_jwt_secret(&pool)
            .await
            .expect("Failed to get JWT secret");

        let canonical_root = std::path::PathBuf::from(&config.root)
            .canonicalize()
            .expect("Root temp dir must be canonicalizable");

        let state = AppState {
            db: pool,
            config,
            setup_guard,
            jwt_secret,
            canonical_root,
        };

        let app = build_router(state);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind to ephemeral port");
        let addr = listener.local_addr().unwrap().to_string();

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
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
        }
    }

    /// Build a full URL from a path like `/api/health`.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    /// Create the initial admin account and return the JWT token.
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

    /// Write a file into the test root directory.
    pub fn write_file(&self, path: &str, content: &[u8]) {
        let full = self.root_dir.path().join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("Failed to create parent dirs");
        }
        std::fs::write(&full, content).expect("Failed to write file");
    }
}
