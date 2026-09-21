//! Runtime configuration.

use std::time::Duration;

pub const DEFAULT_DATABASE_URL: &str = "sqlite:///./ml_release_platform.db";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settings {
    pub database_url: String,
    pub docker: DockerSettings,
}

/// Runtime settings for the local Docker serving adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockerSettings {
    pub container_port: u16,
    pub health_path: String,
    pub inference_path: String,
    pub startup_timeout: Duration,
    pub poll_interval: Duration,
    pub verification_timeout: Duration,
}

impl Settings {
    #[must_use]
    pub fn from_environment() -> Self {
        Self {
            database_url: std::env::var("ML_RELEASE_DATABASE_URL")
                .unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_owned()),
            docker: DockerSettings {
                container_port: environment_u16("ML_RELEASE_CONTAINER_PORT", 8080),
                health_path: environment_path("ML_RELEASE_HEALTH_PATH", "/health"),
                inference_path: environment_path("ML_RELEASE_INFERENCE_PATH", "/infer"),
                startup_timeout: Duration::from_secs(environment_u64(
                    "ML_RELEASE_STARTUP_TIMEOUT_SECONDS",
                    30,
                )),
                poll_interval: Duration::from_millis(environment_u64(
                    "ML_RELEASE_HEALTH_POLL_MILLISECONDS",
                    250,
                )),
                verification_timeout: Duration::from_secs(environment_u64(
                    "ML_RELEASE_VERIFICATION_TIMEOUT_SECONDS",
                    10,
                )),
            },
        }
    }
}

fn environment_u16(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn environment_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn environment_path(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| value.starts_with('/'))
        .unwrap_or_else(|| default.to_owned())
}
