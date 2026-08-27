//! SQLite persistence adapter.

use std::{str::FromStr, sync::Arc};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::{Map, Value};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use thiserror::Error;

use crate::domain::{
    models::{Metrics, ModelRelease},
    policy::PolicyEvaluation,
    states::ReleaseStatus,
};

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("invalid persisted release: {0}")]
    InvalidData(String),
}

#[async_trait]
pub trait ReleaseRepository: Send + Sync {
    async fn create(&self, release: &ModelRelease) -> Result<ModelRelease, RepositoryError>;
    async fn get(&self, release_id: &str) -> Result<Option<ModelRelease>, RepositoryError>;
    async fn list(&self) -> Result<Vec<ModelRelease>, RepositoryError>;
    async fn update(&self, release: &ModelRelease) -> Result<ModelRelease, RepositoryError>;
}

#[derive(Clone)]
pub struct SqliteReleaseRepository {
    pool: SqlitePool,
}

impl SqliteReleaseRepository {
    /// # Errors
    ///
    /// Returns [`RepositoryError`] when the database cannot be opened or migrations cannot run.
    pub async fn connect(database_url: &str) -> Result<Self, RepositoryError> {
        let normalized = normalize_sqlite_url(database_url);
        let options = SqliteConnectOptions::from_str(&normalized)
            .map_err(RepositoryError::Database)?
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    #[must_use]
    pub fn from_pool(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

/// Convert `SQLAlchemy`'s `sqlite:///` URL notation to `SQLx`'s equivalent.
#[must_use]
pub fn normalize_sqlite_url(url: &str) -> String {
    if let Some(path) = url.strip_prefix("sqlite:////") {
        format!("sqlite:///{path}")
    } else if let Some(path) = url.strip_prefix("sqlite:///") {
        format!("sqlite://{path}")
    } else {
        url.to_owned()
    }
}

fn timestamp_to_database(value: DateTime<Utc>) -> String {
    value.to_rfc3339()
}

fn timestamp_from_database(value: &str) -> Result<DateTime<Utc>, RepositoryError> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .or_else(|_| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
                .map(|timestamp| timestamp.and_utc())
        })
        .map_err(|error| {
            RepositoryError::InvalidData(format!("invalid timestamp '{value}': {error}"))
        })
}

fn json_from_database(value: &str, field: &str) -> Result<Value, RepositoryError> {
    serde_json::from_str(value)
        .map_err(|error| RepositoryError::InvalidData(format!("invalid {field} JSON: {error}")))
}

fn value_as_object(value: &Value, field: &str) -> Result<Map<String, Value>, RepositoryError> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| RepositoryError::InvalidData(format!("{field} must be an object")))
}

fn decode_release(row: &sqlx::sqlite::SqliteRow) -> Result<ModelRelease, RepositoryError> {
    let status: String = row.try_get("status")?;
    let metadata = value_as_object(
        &json_from_database(&row.try_get::<String, _>("metadata")?, "metadata")?,
        "metadata",
    )?;
    let raw_metrics = value_as_object(
        &json_from_database(&row.try_get::<String, _>("metrics")?, "metrics")?,
        "metrics",
    )?;
    let metrics = raw_metrics
        .into_iter()
        .map(|(name, value)| {
            value.as_f64().map(|number| (name, number)).ok_or_else(|| {
                RepositoryError::InvalidData("metrics must contain numbers".to_owned())
            })
        })
        .collect::<Result<Metrics, _>>()?;
    let evaluation = row
        .try_get::<Option<String>, _>("evaluation")?
        .map(|value| {
            serde_json::from_str::<PolicyEvaluation>(&value).map_err(|error| {
                RepositoryError::InvalidData(format!("invalid evaluation JSON: {error}"))
            })
        })
        .transpose()?;
    Ok(ModelRelease {
        release_id: row.try_get("release_id")?,
        model_name: row.try_get("model_name")?,
        version: row.try_get("version")?,
        image_uri: row.try_get("image_uri")?,
        artifact_uri: row.try_get("artifact_uri")?,
        status: ReleaseStatus::from_str(&status).map_err(RepositoryError::InvalidData)?,
        created_at: timestamp_from_database(&row.try_get::<String, _>("created_at")?)?,
        updated_at: timestamp_from_database(&row.try_get::<String, _>("updated_at")?)?,
        metadata,
        metrics,
        evaluation,
        failure_reason: row.try_get("failure_reason")?,
    })
}

async fn write_release(
    pool: &SqlitePool,
    release: &ModelRelease,
    update: bool,
) -> Result<ModelRelease, RepositoryError> {
    let metadata = serde_json::to_string(&release.metadata)
        .map_err(|error| RepositoryError::InvalidData(error.to_string()))?;
    let metrics = serde_json::to_string(&release.metrics)
        .map_err(|error| RepositoryError::InvalidData(error.to_string()))?;
    let evaluation = release
        .evaluation
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| RepositoryError::InvalidData(error.to_string()))?;
    if update {
        sqlx::query("UPDATE releases SET model_name = ?, version = ?, image_uri = ?, artifact_uri = ?, status = ?, created_at = ?, updated_at = ?, metadata = ?, metrics = ?, evaluation = ?, failure_reason = ? WHERE release_id = ?")
            .bind(&release.model_name).bind(&release.version).bind(&release.image_uri).bind(&release.artifact_uri).bind(release.status.as_str()).bind(timestamp_to_database(release.created_at)).bind(timestamp_to_database(release.updated_at)).bind(metadata).bind(metrics).bind(evaluation).bind(&release.failure_reason).bind(&release.release_id).execute(pool).await?;
    } else {
        sqlx::query("INSERT INTO releases (release_id, model_name, version, image_uri, artifact_uri, status, created_at, updated_at, metadata, metrics, evaluation, failure_reason) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&release.release_id).bind(&release.model_name).bind(&release.version).bind(&release.image_uri).bind(&release.artifact_uri).bind(release.status.as_str()).bind(timestamp_to_database(release.created_at)).bind(timestamp_to_database(release.updated_at)).bind(metadata).bind(metrics).bind(evaluation).bind(&release.failure_reason).execute(pool).await?;
    }
    Ok(release.clone())
}

#[async_trait]
impl ReleaseRepository for SqliteReleaseRepository {
    async fn create(&self, release: &ModelRelease) -> Result<ModelRelease, RepositoryError> {
        write_release(&self.pool, release, false).await
    }
    async fn get(&self, release_id: &str) -> Result<Option<ModelRelease>, RepositoryError> {
        sqlx::query("SELECT release_id, model_name, version, image_uri, artifact_uri, status, created_at, updated_at, metadata, metrics, evaluation, failure_reason FROM releases WHERE release_id = ?").bind(release_id).fetch_optional(&self.pool).await?.map(|row| decode_release(&row)).transpose()
    }
    async fn list(&self) -> Result<Vec<ModelRelease>, RepositoryError> {
        sqlx::query("SELECT release_id, model_name, version, image_uri, artifact_uri, status, created_at, updated_at, metadata, metrics, evaluation, failure_reason FROM releases ORDER BY created_at DESC").fetch_all(&self.pool).await?.iter().map(decode_release).collect()
    }
    async fn update(&self, release: &ModelRelease) -> Result<ModelRelease, RepositoryError> {
        write_release(&self.pool, release, true).await
    }
}

pub type SharedRepository = Arc<dyn ReleaseRepository>;
