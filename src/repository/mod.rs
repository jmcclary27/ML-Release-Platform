//! SQLite persistence adapter and immutable release execution history.

use std::{str::FromStr, sync::Arc};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::{
    Row, Sqlite, SqlitePool, Transaction,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use thiserror::Error;
use uuid::Uuid;

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
    #[error("concurrent release update for '{0}'")]
    Conflict(String),
}

/// An append-only fact explaining a lifecycle or execution action.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ReleaseEvent {
    pub event_id: String,
    pub release_id: String,
    pub event_type: String,
    pub from_status: Option<ReleaseStatus>,
    pub to_status: Option<ReleaseStatus>,
    pub detail: Option<String>,
    pub occurred_at: DateTime<Utc>,
}

/// Persisted result of one candidate start/health-check attempt.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DeploymentAttempt {
    pub attempt_id: String,
    pub release_id: String,
    pub container_id: Option<String>,
    pub container_name: Option<String>,
    pub endpoint: Option<String>,
    pub succeeded: bool,
    pub detail: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}

/// Immutable result from one runtime inference verification.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VerificationResult {
    pub result_id: String,
    pub release_id: String,
    pub passed: bool,
    pub detail: Option<String>,
    pub checked_at: DateTime<Utc>,
}

/// The model-scoped champion pointer. Previous successful releases remain immutable records.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ActiveRelease {
    pub model_name: String,
    pub release_id: String,
    pub updated_at: DateTime<Utc>,
}

#[async_trait]
pub trait ReleaseRepository: Send + Sync {
    async fn create(&self, release: &ModelRelease) -> Result<ModelRelease, RepositoryError>;
    async fn get(&self, release_id: &str) -> Result<Option<ModelRelease>, RepositoryError>;
    async fn list(&self) -> Result<Vec<ModelRelease>, RepositoryError>;
    /// Retained for updating release evidence; state transitions must use [`Self::transition`].
    async fn update(&self, release: &ModelRelease) -> Result<ModelRelease, RepositoryError>;
    async fn transition(
        &self,
        release: &ModelRelease,
        target: ReleaseStatus,
        detail: Option<&str>,
    ) -> Result<ModelRelease, RepositoryError>;
    async fn record_event(
        &self,
        release_id: &str,
        event_type: &str,
        detail: Option<&str>,
    ) -> Result<ReleaseEvent, RepositoryError>;
    async fn events(&self, release_id: &str) -> Result<Vec<ReleaseEvent>, RepositoryError>;
    async fn events_for_model(
        &self,
        model_name: &str,
    ) -> Result<Vec<ReleaseEvent>, RepositoryError>;
    async fn record_deployment(
        &self,
        attempt: DeploymentAttempt,
    ) -> Result<DeploymentAttempt, RepositoryError>;
    async fn latest_deployment(
        &self,
        release_id: &str,
    ) -> Result<Option<DeploymentAttempt>, RepositoryError>;
    async fn record_verification(
        &self,
        result: VerificationResult,
    ) -> Result<VerificationResult, RepositoryError>;
    async fn latest_verification(
        &self,
        release_id: &str,
    ) -> Result<Option<VerificationResult>, RepositoryError>;
    async fn active(&self, model_name: &str) -> Result<Option<ActiveRelease>, RepositoryError>;
    /// Atomically marks a verified candidate released and makes it the model's active version.
    async fn promote(
        &self,
        release: &ModelRelease,
        detail: Option<&str>,
    ) -> Result<ModelRelease, RepositoryError>;
    /// Atomically completes a rollback and restores (or clears) the active pointer.
    async fn complete_rollback(
        &self,
        release: &ModelRelease,
        previous_active_release_id: Option<&str>,
        detail: Option<&str>,
    ) -> Result<ModelRelease, RepositoryError>;
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

    /// Explicitly close SQLite connections before deleting a database file on Windows.
    pub async fn close(&self) {
        self.pool.close().await;
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

fn decode_status(
    value: Option<String>,
    field: &str,
) -> Result<Option<ReleaseStatus>, RepositoryError> {
    value
        .map(|status| {
            ReleaseStatus::from_str(&status).map_err(|error| {
                RepositoryError::InvalidData(format!("invalid {field} '{status}': {error}"))
            })
        })
        .transpose()
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

fn decode_event(row: &sqlx::sqlite::SqliteRow) -> Result<ReleaseEvent, RepositoryError> {
    Ok(ReleaseEvent {
        event_id: row.try_get("event_id")?,
        release_id: row.try_get("release_id")?,
        event_type: row.try_get("event_type")?,
        from_status: decode_status(row.try_get("from_status")?, "event from_status")?,
        to_status: decode_status(row.try_get("to_status")?, "event to_status")?,
        detail: row.try_get("detail")?,
        occurred_at: timestamp_from_database(&row.try_get::<String, _>("occurred_at")?)?,
    })
}

fn decode_deployment(row: &sqlx::sqlite::SqliteRow) -> Result<DeploymentAttempt, RepositoryError> {
    Ok(DeploymentAttempt {
        attempt_id: row.try_get("attempt_id")?,
        release_id: row.try_get("release_id")?,
        container_id: row.try_get("container_id")?,
        container_name: row.try_get("container_name")?,
        endpoint: row.try_get("endpoint")?,
        succeeded: row.try_get("succeeded")?,
        detail: row.try_get("detail")?,
        started_at: timestamp_from_database(&row.try_get::<String, _>("started_at")?)?,
        finished_at: timestamp_from_database(&row.try_get::<String, _>("finished_at")?)?,
    })
}

fn decode_verification(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<VerificationResult, RepositoryError> {
    Ok(VerificationResult {
        result_id: row.try_get("result_id")?,
        release_id: row.try_get("release_id")?,
        passed: row.try_get("passed")?,
        detail: row.try_get("detail")?,
        checked_at: timestamp_from_database(&row.try_get::<String, _>("checked_at")?)?,
    })
}

fn decode_active(row: &sqlx::sqlite::SqliteRow) -> Result<ActiveRelease, RepositoryError> {
    Ok(ActiveRelease {
        model_name: row.try_get("model_name")?,
        release_id: row.try_get("release_id")?,
        updated_at: timestamp_from_database(&row.try_get::<String, _>("updated_at")?)?,
    })
}

async fn write_release_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    release: &ModelRelease,
    expected_status: Option<ReleaseStatus>,
) -> Result<(), RepositoryError> {
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
    let query = if expected_status.is_some() {
        "UPDATE releases SET model_name = ?, version = ?, image_uri = ?, artifact_uri = ?, status = ?, created_at = ?, updated_at = ?, metadata = ?, metrics = ?, evaluation = ?, failure_reason = ? WHERE release_id = ? AND status = ?"
    } else {
        "UPDATE releases SET model_name = ?, version = ?, image_uri = ?, artifact_uri = ?, status = ?, created_at = ?, updated_at = ?, metadata = ?, metrics = ?, evaluation = ?, failure_reason = ? WHERE release_id = ?"
    };
    let mut bound = sqlx::query(query)
        .bind(&release.model_name)
        .bind(&release.version)
        .bind(&release.image_uri)
        .bind(&release.artifact_uri)
        .bind(release.status.as_str())
        .bind(timestamp_to_database(release.created_at))
        .bind(timestamp_to_database(release.updated_at))
        .bind(metadata)
        .bind(metrics)
        .bind(evaluation)
        .bind(&release.failure_reason)
        .bind(&release.release_id);
    if let Some(status) = expected_status {
        bound = bound.bind(status.as_str());
    }
    let result = bound.execute(&mut **transaction).await?;
    if result.rows_affected() != 1 {
        return Err(RepositoryError::Conflict(release.release_id.clone()));
    }
    Ok(())
}

async fn insert_event(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &ReleaseEvent,
) -> Result<(), RepositoryError> {
    sqlx::query("INSERT INTO release_events (event_id, release_id, event_type, from_status, to_status, detail, occurred_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(&event.event_id)
        .bind(&event.release_id)
        .bind(&event.event_type)
        .bind(event.from_status.map(ReleaseStatus::as_str))
        .bind(event.to_status.map(ReleaseStatus::as_str))
        .bind(&event.detail)
        .bind(timestamp_to_database(event.occurred_at))
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn event(
    release_id: &str,
    event_type: &str,
    from_status: Option<ReleaseStatus>,
    to_status: Option<ReleaseStatus>,
    detail: Option<&str>,
) -> ReleaseEvent {
    ReleaseEvent {
        event_id: Uuid::new_v4().to_string(),
        release_id: release_id.to_owned(),
        event_type: event_type.to_owned(),
        from_status,
        to_status,
        detail: detail.map(str::to_owned),
        occurred_at: Utc::now(),
    }
}

#[async_trait]
impl ReleaseRepository for SqliteReleaseRepository {
    async fn create(&self, release: &ModelRelease) -> Result<ModelRelease, RepositoryError> {
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
        let mut transaction = self.pool.begin().await?;
        sqlx::query("INSERT INTO releases (release_id, model_name, version, image_uri, artifact_uri, status, created_at, updated_at, metadata, metrics, evaluation, failure_reason) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&release.release_id).bind(&release.model_name).bind(&release.version).bind(&release.image_uri).bind(&release.artifact_uri).bind(release.status.as_str()).bind(timestamp_to_database(release.created_at)).bind(timestamp_to_database(release.updated_at)).bind(metadata).bind(metrics).bind(evaluation).bind(&release.failure_reason).execute(&mut *transaction).await?;
        insert_event(
            &mut transaction,
            &event(
                &release.release_id,
                "CREATED",
                None,
                Some(release.status),
                None,
            ),
        )
        .await?;
        transaction.commit().await?;
        Ok(release.clone())
    }

    async fn get(&self, release_id: &str) -> Result<Option<ModelRelease>, RepositoryError> {
        sqlx::query("SELECT release_id, model_name, version, image_uri, artifact_uri, status, created_at, updated_at, metadata, metrics, evaluation, failure_reason FROM releases WHERE release_id = ?").bind(release_id).fetch_optional(&self.pool).await?.map(|row| decode_release(&row)).transpose()
    }

    async fn list(&self) -> Result<Vec<ModelRelease>, RepositoryError> {
        sqlx::query("SELECT release_id, model_name, version, image_uri, artifact_uri, status, created_at, updated_at, metadata, metrics, evaluation, failure_reason FROM releases ORDER BY created_at DESC, release_id DESC").fetch_all(&self.pool).await?.iter().map(decode_release).collect()
    }

    async fn update(&self, release: &ModelRelease) -> Result<ModelRelease, RepositoryError> {
        let mut transaction = self.pool.begin().await?;
        write_release_in_transaction(&mut transaction, release, None).await?;
        transaction.commit().await?;
        Ok(release.clone())
    }

    async fn transition(
        &self,
        release: &ModelRelease,
        target: ReleaseStatus,
        detail: Option<&str>,
    ) -> Result<ModelRelease, RepositoryError> {
        let transitioned = release
            .transition_to(target)
            .map_err(|error| RepositoryError::InvalidData(error.to_string()))?;
        let mut transaction = self.pool.begin().await?;
        write_release_in_transaction(&mut transaction, &transitioned, Some(release.status)).await?;
        insert_event(
            &mut transaction,
            &event(
                &release.release_id,
                "STATE_TRANSITION",
                Some(release.status),
                Some(target),
                detail,
            ),
        )
        .await?;
        transaction.commit().await?;
        Ok(transitioned)
    }

    async fn record_event(
        &self,
        release_id: &str,
        event_type: &str,
        detail: Option<&str>,
    ) -> Result<ReleaseEvent, RepositoryError> {
        let event = event(release_id, event_type, None, None, detail);
        let mut transaction = self.pool.begin().await?;
        insert_event(&mut transaction, &event).await?;
        transaction.commit().await?;
        Ok(event)
    }

    async fn events(&self, release_id: &str) -> Result<Vec<ReleaseEvent>, RepositoryError> {
        sqlx::query("SELECT event_id, release_id, event_type, from_status, to_status, detail, occurred_at FROM release_events WHERE release_id = ? ORDER BY occurred_at, event_id")
            .bind(release_id)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(decode_event)
            .collect()
    }

    async fn events_for_model(
        &self,
        model_name: &str,
    ) -> Result<Vec<ReleaseEvent>, RepositoryError> {
        sqlx::query("SELECT e.event_id, e.release_id, e.event_type, e.from_status, e.to_status, e.detail, e.occurred_at FROM release_events e INNER JOIN releases r ON r.release_id = e.release_id WHERE r.model_name = ? ORDER BY e.occurred_at, e.event_id")
            .bind(model_name)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(decode_event)
            .collect()
    }

    async fn record_deployment(
        &self,
        mut attempt: DeploymentAttempt,
    ) -> Result<DeploymentAttempt, RepositoryError> {
        if attempt.attempt_id.is_empty() {
            attempt.attempt_id = Uuid::new_v4().to_string();
        }
        let mut transaction = self.pool.begin().await?;
        sqlx::query("INSERT INTO deployment_attempts (attempt_id, release_id, container_id, container_name, endpoint, succeeded, detail, started_at, finished_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&attempt.attempt_id).bind(&attempt.release_id).bind(&attempt.container_id).bind(&attempt.container_name).bind(&attempt.endpoint).bind(attempt.succeeded).bind(&attempt.detail).bind(timestamp_to_database(attempt.started_at)).bind(timestamp_to_database(attempt.finished_at)).execute(&mut *transaction).await?;
        insert_event(
            &mut transaction,
            &event(
                &attempt.release_id,
                if attempt.succeeded {
                    "DEPLOYMENT_SUCCEEDED"
                } else {
                    "DEPLOYMENT_FAILED"
                },
                None,
                None,
                attempt.detail.as_deref(),
            ),
        )
        .await?;
        transaction.commit().await?;
        Ok(attempt)
    }

    async fn latest_deployment(
        &self,
        release_id: &str,
    ) -> Result<Option<DeploymentAttempt>, RepositoryError> {
        sqlx::query("SELECT attempt_id, release_id, container_id, container_name, endpoint, succeeded, detail, started_at, finished_at FROM deployment_attempts WHERE release_id = ? ORDER BY started_at DESC, attempt_id DESC LIMIT 1")
            .bind(release_id).fetch_optional(&self.pool).await?.map(|row| decode_deployment(&row)).transpose()
    }

    async fn record_verification(
        &self,
        mut result: VerificationResult,
    ) -> Result<VerificationResult, RepositoryError> {
        if result.result_id.is_empty() {
            result.result_id = Uuid::new_v4().to_string();
        }
        let mut transaction = self.pool.begin().await?;
        sqlx::query("INSERT INTO verification_results (result_id, release_id, passed, detail, checked_at) VALUES (?, ?, ?, ?, ?)")
            .bind(&result.result_id).bind(&result.release_id).bind(result.passed).bind(&result.detail).bind(timestamp_to_database(result.checked_at)).execute(&mut *transaction).await?;
        insert_event(
            &mut transaction,
            &event(
                &result.release_id,
                if result.passed {
                    "VERIFICATION_PASSED"
                } else {
                    "VERIFICATION_FAILED"
                },
                None,
                None,
                result.detail.as_deref(),
            ),
        )
        .await?;
        transaction.commit().await?;
        Ok(result)
    }

    async fn latest_verification(
        &self,
        release_id: &str,
    ) -> Result<Option<VerificationResult>, RepositoryError> {
        sqlx::query("SELECT result_id, release_id, passed, detail, checked_at FROM verification_results WHERE release_id = ? ORDER BY checked_at DESC, result_id DESC LIMIT 1")
            .bind(release_id).fetch_optional(&self.pool).await?.map(|row| decode_verification(&row)).transpose()
    }

    async fn active(&self, model_name: &str) -> Result<Option<ActiveRelease>, RepositoryError> {
        sqlx::query(
            "SELECT model_name, release_id, updated_at FROM active_releases WHERE model_name = ?",
        )
        .bind(model_name)
        .fetch_optional(&self.pool)
        .await?
        .map(|row| decode_active(&row))
        .transpose()
    }

    async fn promote(
        &self,
        release: &ModelRelease,
        detail: Option<&str>,
    ) -> Result<ModelRelease, RepositoryError> {
        let promoted = release
            .transition_to(ReleaseStatus::Released)
            .map_err(|error| RepositoryError::InvalidData(error.to_string()))?;
        let mut transaction = self.pool.begin().await?;
        write_release_in_transaction(&mut transaction, &promoted, Some(release.status)).await?;
        let now = Utc::now();
        sqlx::query("INSERT INTO active_releases (model_name, release_id, updated_at) VALUES (?, ?, ?) ON CONFLICT(model_name) DO UPDATE SET release_id = excluded.release_id, updated_at = excluded.updated_at")
            .bind(&promoted.model_name).bind(&promoted.release_id).bind(timestamp_to_database(now)).execute(&mut *transaction).await?;
        insert_event(
            &mut transaction,
            &event(
                &release.release_id,
                "STATE_TRANSITION",
                Some(release.status),
                Some(ReleaseStatus::Released),
                detail,
            ),
        )
        .await?;
        insert_event(
            &mut transaction,
            &event(
                &release.release_id,
                "ACTIVE_VERSION_SET",
                None,
                None,
                detail,
            ),
        )
        .await?;
        transaction.commit().await?;
        Ok(promoted)
    }

    async fn complete_rollback(
        &self,
        release: &ModelRelease,
        previous_active_release_id: Option<&str>,
        detail: Option<&str>,
    ) -> Result<ModelRelease, RepositoryError> {
        let rolled_back = release
            .with_failure(
                detail
                    .unwrap_or("Runtime verification failed; candidate rolled back.")
                    .to_owned(),
            )
            .transition_to(ReleaseStatus::RolledBack)
            .map_err(|error| RepositoryError::InvalidData(error.to_string()))?;
        let mut transaction = self.pool.begin().await?;
        write_release_in_transaction(&mut transaction, &rolled_back, Some(release.status)).await?;
        if let Some(previous_release_id) = previous_active_release_id {
            sqlx::query("INSERT INTO active_releases (model_name, release_id, updated_at) VALUES (?, ?, ?) ON CONFLICT(model_name) DO UPDATE SET release_id = excluded.release_id, updated_at = excluded.updated_at")
                .bind(&rolled_back.model_name).bind(previous_release_id).bind(timestamp_to_database(Utc::now())).execute(&mut *transaction).await?;
        } else {
            sqlx::query("DELETE FROM active_releases WHERE model_name = ?")
                .bind(&rolled_back.model_name)
                .execute(&mut *transaction)
                .await?;
        }
        insert_event(
            &mut transaction,
            &event(
                &release.release_id,
                "STATE_TRANSITION",
                Some(release.status),
                Some(ReleaseStatus::RolledBack),
                detail,
            ),
        )
        .await?;
        insert_event(
            &mut transaction,
            &event(
                &release.release_id,
                "ACTIVE_VERSION_RESTORED",
                None,
                None,
                detail,
            ),
        )
        .await?;
        transaction.commit().await?;
        Ok(rolled_back)
    }
}

pub type SharedRepository = Arc<dyn ReleaseRepository>;

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::Map;

    use super::*;
    use crate::domain::models::ModelRelease;

    fn database_url(name: &str) -> (String, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "ml-release-platform-repository-{name}-{}.sqlite",
            Uuid::new_v4()
        ));
        (format!("sqlite:///{}", path.display()), path)
    }

    fn release(id: &str, version: &str) -> ModelRelease {
        let now = Utc::now();
        ModelRelease {
            release_id: id.to_owned(),
            model_name: "example".to_owned(),
            version: version.to_owned(),
            image_uri: format!("example:{version}"),
            artifact_uri: None,
            status: ReleaseStatus::Submitted,
            created_at: now,
            updated_at: now,
            metadata: Map::new(),
            metrics: Metrics::new(),
            evaluation: None,
            failure_reason: None,
        }
    }

    async fn ready(repository: &SqliteReleaseRepository, release: &ModelRelease) -> ModelRelease {
        let validating = repository
            .transition(
                release,
                ReleaseStatus::Validating,
                Some("evaluation started"),
            )
            .await
            .unwrap();
        repository
            .transition(&validating, ReleaseStatus::Ready, Some("policy passed"))
            .await
            .unwrap()
    }

    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn promotion_and_rollback_preserve_immutable_history_and_active_pointer() {
        let (url, path) = database_url("execution");
        let repository = SqliteReleaseRepository::connect(&url).await.unwrap();
        let first = release("first", "v1");
        repository.create(&first).await.unwrap();
        let first = ready(&repository, &first).await;
        let first = repository
            .transition(&first, ReleaseStatus::Deploying, None)
            .await
            .unwrap();
        let first = repository
            .transition(&first, ReleaseStatus::Verifying, None)
            .await
            .unwrap();
        let first = repository
            .promote(&first, Some("v1 verified"))
            .await
            .unwrap();
        assert_eq!(first.status, ReleaseStatus::Released);
        assert_eq!(
            repository
                .active("example")
                .await
                .unwrap()
                .unwrap()
                .release_id,
            "first"
        );

        let second = release("second", "v2");
        repository.create(&second).await.unwrap();
        let second = ready(&repository, &second).await;
        let second = repository
            .transition(&second, ReleaseStatus::Deploying, None)
            .await
            .unwrap();
        repository
            .record_deployment(DeploymentAttempt {
                attempt_id: String::new(),
                release_id: second.release_id.clone(),
                container_id: Some("candidate-container".to_owned()),
                container_name: Some("mlrp-example-v2".to_owned()),
                endpoint: Some("http://127.0.0.1:12345".to_owned()),
                succeeded: true,
                detail: None,
                started_at: Utc::now(),
                finished_at: Utc::now(),
            })
            .await
            .unwrap();
        let second = repository
            .transition(&second, ReleaseStatus::Verifying, None)
            .await
            .unwrap();
        repository
            .record_verification(VerificationResult {
                result_id: String::new(),
                release_id: second.release_id.clone(),
                passed: false,
                detail: Some("inference returned 503".to_owned()),
                checked_at: Utc::now(),
            })
            .await
            .unwrap();
        let second = repository
            .transition(&second, ReleaseStatus::RollingBack, None)
            .await
            .unwrap();
        let second = repository
            .complete_rollback(
                &second,
                Some(&first.release_id),
                Some("verification failed"),
            )
            .await
            .unwrap();

        assert_eq!(second.status, ReleaseStatus::RolledBack);
        assert_eq!(
            repository
                .active("example")
                .await
                .unwrap()
                .unwrap()
                .release_id,
            first.release_id
        );
        assert_eq!(
            repository
                .latest_deployment(&second.release_id)
                .await
                .unwrap()
                .unwrap()
                .container_id
                .as_deref(),
            Some("candidate-container")
        );
        assert!(
            !repository
                .latest_verification(&second.release_id)
                .await
                .unwrap()
                .unwrap()
                .passed
        );
        let events = repository.events(&second.release_id).await.unwrap();
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "VERIFICATION_FAILED")
        );
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "ACTIVE_VERSION_RESTORED")
        );

        repository.close().await;
        std::fs::remove_file(path).unwrap();
    }
}
