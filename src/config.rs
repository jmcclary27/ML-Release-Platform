//! Runtime configuration.

pub const DEFAULT_DATABASE_URL: &str = "sqlite:///./ml_release_platform.db";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settings {
    pub database_url: String,
}

impl Settings {
    #[must_use]
    pub fn from_environment() -> Self {
        Self {
            database_url: std::env::var("ML_RELEASE_DATABASE_URL")
                .unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_owned()),
        }
    }
}
