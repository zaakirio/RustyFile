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

    /// Trusted proxy IPs for X-Forwarded-For (comma-separated; default: 127.0.0.1)
    #[arg(long, env = "RUSTYFILE_TRUSTED_PROXIES")]
    trusted_proxies: Option<String>,

    /// Cache directory for TUS temp files, thumbnails, etc.
    #[arg(long, env = "RUSTYFILE_CACHE_DIR")]
    cache_dir: Option<String>,

    /// Hours before incomplete TUS uploads expire
    #[arg(long, env = "RUSTYFILE_TUS_EXPIRY_HOURS")]
    tus_expiry_hours: Option<u64>,

    /// Set cookie Secure flag (disable for local dev without HTTPS)
    #[arg(long, env = "RUSTYFILE_SECURE_COOKIE")]
    secure_cookie: Option<bool>,

    /// Blocked file extensions for uploads (comma-separated, e.g. ".php,.sh,.exe")
    #[arg(long, env = "RUSTYFILE_BLOCKED_UPLOAD_EXTENSIONS")]
    blocked_upload_extensions: Option<String>,

    /// Max API requests per IP per minute for expensive endpoints
    #[arg(long, env = "RUSTYFILE_API_RATE_LIMIT")]
    api_rate_limit: Option<u32>,
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

    /// Prevents Argon2 DoS.
    #[serde(default = "default_max_password_length")]
    pub max_password_length: usize,

    #[serde(default = "default_max_listing_items")]
    pub max_listing_items: usize,

    #[serde(default = "default_trusted_proxies")]
    pub trusted_proxies: String,

    #[serde(default = "default_cache_dir")]
    pub cache_dir: String,

    #[serde(default = "default_tus_expiry_hours")]
    pub tus_expiry_hours: u64,

    #[serde(default = "default_secure_cookie")]
    pub secure_cookie: bool,

    /// Comma-separated list of blocked file extensions for uploads (e.g. ".php,.sh,.exe")
    #[serde(default = "default_blocked_upload_extensions")]
    pub blocked_upload_extensions: String,

    /// Max API requests per IP per minute for expensive endpoints (search, thumbnails, HLS)
    #[serde(default = "default_api_rate_limit")]
    pub api_rate_limit: u32,
}

// ── Default value constants (single source of truth) ──────────────────────────
const DEFAULT_HOST: &str = "0.0.0.0";
const DEFAULT_PORT: u16 = 8080;
const DEFAULT_ROOT: &str = "./data";
const DEFAULT_DATA_DIR: &str = "./rustyfile-data";
const DEFAULT_LOG_LEVEL: &str = "info";
const DEFAULT_LOG_FORMAT: &str = "pretty";
const DEFAULT_JWT_EXPIRY_HOURS: u64 = 2;
const DEFAULT_MIN_PASSWORD_LENGTH: usize = 10;
const DEFAULT_SETUP_TIMEOUT_MINUTES: u64 = 5;
const DEFAULT_CORS_ORIGINS: &str = "same-origin";
const DEFAULT_MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024; // 50 MB
const DEFAULT_MAX_PASSWORD_LENGTH: usize = 128;
const DEFAULT_MAX_LISTING_ITEMS: usize = 10_000;
const DEFAULT_TRUSTED_PROXIES: &str = "127.0.0.1";
const DEFAULT_CACHE_DIR: &str = "./rustyfile-data/cache";
const DEFAULT_TUS_EXPIRY_HOURS: u64 = 24;
const DEFAULT_SECURE_COOKIE: bool = true;
const DEFAULT_BLOCKED_UPLOAD_EXTENSIONS: &str =
    ".php,.phtml,.php5,.sh,.bash,.cgi,.pl,.py,.rb,.exe,.bat,.cmd,.ps1,.msi,.dll,.so,.com,.scr,.vbs,.vbe,.wsf,.wsh,.jar";
const DEFAULT_API_RATE_LIMIT: u32 = 60;

fn default_host() -> String {
    DEFAULT_HOST.into()
}
fn default_port() -> u16 {
    DEFAULT_PORT
}
fn default_root() -> String {
    DEFAULT_ROOT.into()
}
fn default_data_dir() -> String {
    DEFAULT_DATA_DIR.into()
}
fn default_log_level() -> String {
    DEFAULT_LOG_LEVEL.into()
}
fn default_log_format() -> String {
    DEFAULT_LOG_FORMAT.into()
}
fn default_jwt_expiry_hours() -> u64 {
    DEFAULT_JWT_EXPIRY_HOURS
}
fn default_min_password_length() -> usize {
    DEFAULT_MIN_PASSWORD_LENGTH
}
fn default_setup_timeout_minutes() -> u64 {
    DEFAULT_SETUP_TIMEOUT_MINUTES
}
fn default_cors_origins() -> String {
    DEFAULT_CORS_ORIGINS.into()
}
fn default_max_upload_bytes() -> usize {
    DEFAULT_MAX_UPLOAD_BYTES
}
fn default_max_password_length() -> usize {
    DEFAULT_MAX_PASSWORD_LENGTH
}
fn default_max_listing_items() -> usize {
    DEFAULT_MAX_LISTING_ITEMS
}
fn default_trusted_proxies() -> String {
    DEFAULT_TRUSTED_PROXIES.into()
}
fn default_cache_dir() -> String {
    DEFAULT_CACHE_DIR.into()
}
fn default_tus_expiry_hours() -> u64 {
    DEFAULT_TUS_EXPIRY_HOURS
}
fn default_secure_cookie() -> bool {
    DEFAULT_SECURE_COOKIE
}
fn default_blocked_upload_extensions() -> String {
    DEFAULT_BLOCKED_UPLOAD_EXTENSIONS.into()
}
fn default_api_rate_limit() -> u32 {
    DEFAULT_API_RATE_LIMIT
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            host: DEFAULT_HOST.into(),
            port: DEFAULT_PORT,
            root: DEFAULT_ROOT.into(),
            data_dir: DEFAULT_DATA_DIR.into(),
            log_level: DEFAULT_LOG_LEVEL.into(),
            log_format: DEFAULT_LOG_FORMAT.into(),
            jwt_expiry_hours: DEFAULT_JWT_EXPIRY_HOURS,
            min_password_length: DEFAULT_MIN_PASSWORD_LENGTH,
            setup_timeout_minutes: DEFAULT_SETUP_TIMEOUT_MINUTES,
            cors_origins: DEFAULT_CORS_ORIGINS.into(),
            max_upload_bytes: DEFAULT_MAX_UPLOAD_BYTES,
            max_password_length: DEFAULT_MAX_PASSWORD_LENGTH,
            max_listing_items: DEFAULT_MAX_LISTING_ITEMS,
            trusted_proxies: DEFAULT_TRUSTED_PROXIES.into(),
            cache_dir: DEFAULT_CACHE_DIR.into(),
            tus_expiry_hours: DEFAULT_TUS_EXPIRY_HOURS,
            secure_cookie: DEFAULT_SECURE_COOKIE,
            blocked_upload_extensions: DEFAULT_BLOCKED_UPLOAD_EXTENSIONS.into(),
            api_rate_limit: DEFAULT_API_RATE_LIMIT,
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

        macro_rules! merge_opt {
            ($figment:expr, $cli:expr, $($field:ident),* $(,)?) => {
                $(
                    if let Some(ref v) = $cli.$field {
                        $figment = $figment.merge(Serialized::default(stringify!($field), v));
                    }
                )*
            };
        }

        merge_opt!(
            figment,
            cli,
            host,
            port,
            root,
            data_dir,
            log_level,
            log_format,
            jwt_expiry_hours,
            min_password_length,
            setup_timeout_minutes,
            cors_origins,
            max_upload_bytes,
            max_password_length,
            max_listing_items,
            trusted_proxies,
            cache_dir,
            tus_expiry_hours,
            secure_cookie,
            blocked_upload_extensions,
            api_rate_limit,
        );

        let config: Self = figment.extract()?;
        Ok(config)
    }

    pub fn db_path(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(&self.data_dir).join("rustyfile.db")
    }

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
