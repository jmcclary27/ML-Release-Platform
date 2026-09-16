CREATE TABLE IF NOT EXISTS release_events (
    event_id VARCHAR(36) PRIMARY KEY NOT NULL,
    release_id VARCHAR(36) NOT NULL,
    event_type VARCHAR(64) NOT NULL,
    from_status VARCHAR(32),
    to_status VARCHAR(32),
    detail TEXT,
    occurred_at DATETIME NOT NULL,
    FOREIGN KEY (release_id) REFERENCES releases(release_id)
);
CREATE INDEX IF NOT EXISTS ix_release_events_release_occurred
    ON release_events (release_id, occurred_at, event_id);

CREATE TABLE IF NOT EXISTS deployment_attempts (
    attempt_id VARCHAR(36) PRIMARY KEY NOT NULL,
    release_id VARCHAR(36) NOT NULL,
    container_id VARCHAR(255),
    container_name VARCHAR(255),
    endpoint VARCHAR(2048),
    succeeded BOOLEAN NOT NULL,
    detail TEXT,
    started_at DATETIME NOT NULL,
    finished_at DATETIME NOT NULL,
    FOREIGN KEY (release_id) REFERENCES releases(release_id)
);
CREATE INDEX IF NOT EXISTS ix_deployment_attempts_release_started
    ON deployment_attempts (release_id, started_at, attempt_id);

CREATE TABLE IF NOT EXISTS verification_results (
    result_id VARCHAR(36) PRIMARY KEY NOT NULL,
    release_id VARCHAR(36) NOT NULL,
    passed BOOLEAN NOT NULL,
    detail TEXT,
    checked_at DATETIME NOT NULL,
    FOREIGN KEY (release_id) REFERENCES releases(release_id)
);
CREATE INDEX IF NOT EXISTS ix_verification_results_release_checked
    ON verification_results (release_id, checked_at, result_id);

CREATE TABLE IF NOT EXISTS active_releases (
    model_name VARCHAR(255) PRIMARY KEY NOT NULL,
    release_id VARCHAR(36) NOT NULL UNIQUE,
    updated_at DATETIME NOT NULL,
    FOREIGN KEY (release_id) REFERENCES releases(release_id)
);

-- Preserve data created by the pre-MVP local lifecycle while adopting the explicit
-- verification and released semantics.
UPDATE releases SET status = 'VERIFYING' WHERE status = 'CANARY';
UPDATE releases SET status = 'RELEASED' WHERE status = 'PROMOTED';
