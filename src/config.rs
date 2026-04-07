use clap::Parser;
use figment::providers::{Env, Format, Serialized, Toml};
use figment::Figment;
use serde::{Deserialize, Serialize};

#[derive(Debug, Parser)]
#[command(name = "rustyfile", version, about = "Fast, self-hosted file browser")]
struct CliArgs {
    /// Host address to bind to
    #[arg(long, env = "RUSTYFILE_HOST")]
    host: Option<String>,

    /// Port to listen on
    #[arg(long, env = "RUSTYFILE_PORT")]
    port: Option<u16>,

    /// Root directory to serve files from
    #[arg(long, env = "RUSTYFILE_ROOT")]
    root: Option<String>,

    /// Internal data directory (for database, etc.)
    #[arg(long, env = "RUSTYFILE_DATA_DIR")]
    data_dir: Option<String>,

    /// Logging level
    #[arg(long, env = "RUSTYFILE_LOG_LEVEL")]
    log_level: Option<String>,

    /// Log format: "pretty" or "json"
    #[arg(long, env = "RUSTYFILE_LOG_FORMAT")]
    log_format: Option<String>,

    /// JWT token expiry in hours
    #[arg(long, env = "RUSTYFILE_JWT_EXPIRY_HOURS")]
    jwt_expiry_hours: Option<u64>,

    /// Minimum password length
    #[arg(long, env = "RUSTYFILE_MIN_PASSWORD_LENGTH")]
    min_password_length: Option<usize>,

    /// Setup wizard timeout in minutes
    #[arg(long, env = "RUSTYFILE_SETUP_TIMEOUT_MINUTES")]
    setup_timeout_minutes: Option<u64>,

    /// Allowed CORS origins (comma-separated, or "*" for any)
    #[arg(long, env = "RUSTYFILE_CORS_ORIGINS")]
    cors_origins: Option<String>,

    /// Maximum upload body size in bytes
    #[arg(long, env = "RUSTYFILE_MAX_UPLOAD_BYTES")]
    max_upload_bytes: Option<usize>,

    /// Maximum password length
    #[arg(long, env = "RUSTYFILE_MAX_PASSWORD_LENGTH")]
    max_password_length: Option<usize>,

    /// Maximum items in directory listing
    #[arg(long, env = "RUSTYFILE_MAX_LISTING_ITEMS")]
    max_listing_items: Option<usize>,

    /// Trusted proxy IPs for X-Forwarded-For (comma-separated, empty = trust all)
    #[arg(long, env = "RUSTYFILE_TRUSTED_PROXIES")]
    trusted_proxies: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_host")]
    pub host: String,

    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_root")]
    pub root: String,

    #[serde(default = "default_data_dir")]
    pub data_dir: String,

    #[serde(default = "default_log_level")]
    pub log_level: String,

    #[serde(default = "default_log_format")]
    pub log_format: String,

    #[serde(default = "default_jwt_expiry_hours")]
    pub jwt_expiry_hours: u64,

    #[serde(default = "default_min_password_length")]
    pub min_password_length: usize,

    #[serde(default = "default_setup_timeout_minutes")]
    pub setup_timeout_minutes: u64,

    #[serde(default = "default_cors_origins")]
    pub cors_origins: String,

    #[serde(default = "default_max_upload_bytes")]
    pub max_upload_bytes: usize,

    /// Max length prevents Argon2 DoS with extremely long passwords.
    #[serde(default = "default_max_password_length")]
    pub max_password_length: usize,

    #[serde(default = "default_max_listing_items")]
    pub max_listing_items: usize,

    /// Comma-separated list of trusted proxy IPs for X-Forwarded-For.
    /// Empty means trust all (development default).
    #[serde(default = "default_trusted_proxies")]
    pub trusted_proxies: String,
}

fn default_host() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    8080
}
fn default_root() -> String {
    "./data".into()
}
fn default_data_dir() -> String {
    "./rustyfile-data".into()
}
fn default_log_level() -> String {
    "info".into()
}
fn default_log_format() -> String {
    "pretty".into()
}
fn default_jwt_expiry_hours() -> u64 {
    2
}
fn default_min_password_length() -> usize {
    10
}
fn default_setup_timeout_minutes() -> u64 {
    5
}
fn default_cors_origins() -> String {
    "*".into()
}
fn default_max_upload_bytes() -> usize {
    50 * 1024 * 1024 // 50 MB
}
fn default_max_password_length() -> usize {
    128
}
fn default_max_listing_items() -> usize {
    10_000
}
fn default_trusted_proxies() -> String {
    "".into() // Empty = trust all (backwards compatible)
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            root: default_root(),
            data_dir: default_data_dir(),
            log_level: default_log_level(),
            log_format: default_log_format(),
            jwt_expiry_hours: default_jwt_expiry_hours(),
            min_password_length: default_min_password_length(),
            setup_timeout_minutes: default_setup_timeout_minutes(),
            cors_origins: default_cors_origins(),
            max_upload_bytes: default_max_upload_bytes(),
            max_password_length: default_max_password_length(),
            max_listing_items: default_max_listing_items(),
            trusted_proxies: default_trusted_proxies(),
        }
    }
}

impl AppConfig {
    /// Precedence: defaults < config.toml < RUSTYFILE_* env < CLI args
    pub fn load() -> anyhow::Result<Self> {
        let cli = CliArgs::parse();

        let mut figment = Figment::new()
            .merge(Serialized::defaults(Self::default()))
            .merge(Toml::file("config.toml").nested())
            .merge(Env::prefixed("RUSTYFILE_").lowercase(false));

        if let Some(v) = &cli.host {
            figment = figment.merge(Serialized::default("host", v));
        }
        if let Some(v) = cli.port {
            figment = figment.merge(Serialized::default("port", v));
        }
        if let Some(v) = &cli.root {
            figment = figment.merge(Serialized::default("root", v));
        }
        if let Some(v) = &cli.data_dir {
            figment = figment.merge(Serialized::default("data_dir", v));
        }
        if let Some(v) = &cli.log_level {
            figment = figment.merge(Serialized::default("log_level", v));
        }
        if let Some(v) = &cli.log_format {
            figment = figment.merge(Serialized::default("log_format", v));
        }
        if let Some(v) = cli.jwt_expiry_hours {
            figment = figment.merge(Serialized::default("jwt_expiry_hours", v));
        }
        if let Some(v) = cli.min_password_length {
            figment = figment.merge(Serialized::default("min_password_length", v));
        }
        if let Some(v) = cli.setup_timeout_minutes {
            figment = figment.merge(Serialized::default("setup_timeout_minutes", v));
        }
        if let Some(v) = &cli.cors_origins {
            figment = figment.merge(Serialized::default("cors_origins", v));
        }
        if let Some(v) = cli.max_upload_bytes {
            figment = figment.merge(Serialized::default("max_upload_bytes", v));
        }
        if let Some(v) = cli.max_password_length {
            figment = figment.merge(Serialized::default("max_password_length", v));
        }
        if let Some(v) = cli.max_listing_items {
            figment = figment.merge(Serialized::default("max_listing_items", v));
        }
        if let Some(v) = &cli.trusted_proxies {
            figment = figment.merge(Serialized::default("trusted_proxies", v));
        }

        let config: Self = figment.extract()?;
        Ok(config)
    }

    pub fn db_path(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(&self.data_dir).join("rustyfile.db")
    }

    /// Log warnings for security-sensitive configuration defaults.
    /// Call once at startup after logging is initialized.
    pub fn log_security_warnings(&self) {
        if self.cors_origins.trim() == "*" || self.cors_origins.trim().is_empty() {
            tracing::warn!(
                "CORS allows all origins (*). Set RUSTYFILE_CORS_ORIGINS for production."
            );
        }
        if self.trusted_proxies.trim().is_empty() {
            tracing::warn!(
                "X-Forwarded-For trusted from all sources. \
                 Set RUSTYFILE_TRUSTED_PROXIES for production."
            );
        }
    }
}
