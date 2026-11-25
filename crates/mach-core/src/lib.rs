use serde::Deserialize;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("unable to read mach.toml")]
    Read,
    #[error("invalid mach.toml (check keys, types and values)")]
    Invalid,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub project: Project,
    #[serde(default)]
    pub database: Database,
    #[serde(default)]
    pub http: Http,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub agent: Agent,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub name: String,
}

#[derive(Debug, Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Allow,
    #[default]
    Warn,
    Deny,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Agent {
    pub output: Output,
}
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Output {
    #[default]
    Compact,
    Json,
    Human,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exception {
    pub code: String,
    pub file: String,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub redundant_comments: Level,
    pub unnecessary_wrappers: Level,
    pub unsafe_code: Level,
    pub dead_code: Level,
    pub unnecessary_clones: Level,
    pub speculative_abstractions: Level,
    pub unbounded_public_queries: Level,
    pub unused_dependencies: Level,
    pub exceptions: Vec<Exception>,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            redundant_comments: Level::Warn,
            unnecessary_wrappers: Level::Warn,
            unsafe_code: Level::Deny,
            dead_code: Level::Deny,
            unnecessary_clones: Level::Warn,
            speculative_abstractions: Level::Warn,
            unbounded_public_queries: Level::Deny,
            unused_dependencies: Level::Warn,
            exceptions: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Database {
    pub provider: String,
    pub max_connections: u32,
}
impl Default for Database {
    fn default() -> Self {
        Self {
            provider: "postgres".into(),
            max_connections: 20,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Http {
    pub bind: std::net::SocketAddr,
    pub max_body_bytes: usize,
    pub request_timeout_ms: u64,
    pub max_concurrent_requests: usize,
    pub cors_origin: Option<String>,
}
impl Default for Http {
    fn default() -> Self {
        Self {
            bind: ([127, 0, 0, 1], 3000).into(),
            max_body_bytes: 1 << 20,
            request_timeout_ms: 10_000,
            max_concurrent_requests: 128,
            cors_origin: None,
        }
    }
}
impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|_| ConfigError::Read)?;
        let c: Self = toml::from_str(&text).map_err(|_| ConfigError::Invalid)?;
        if c.policy
            .exceptions
            .iter()
            .any(|e| e.reason.trim().is_empty() || e.file.is_empty() || e.code.is_empty())
            || c.project.name.is_empty()
            || c.database.provider != "postgres"
            || c.database.max_connections == 0
            || c.http.max_body_bytes == 0
            || c.http.request_timeout_ms == 0
            || !(1..=65535).contains(&c.http.max_concurrent_requests)
        {
            return Err(ConfigError::Invalid);
        }
        Ok(c)
    }
}
