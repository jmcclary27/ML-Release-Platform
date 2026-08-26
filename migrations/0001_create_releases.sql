CREATE TABLE IF NOT EXISTS releases (
    release_id VARCHAR(36) PRIMARY KEY NOT NULL,
    model_name VARCHAR(255) NOT NULL,
    version VARCHAR(255) NOT NULL,
    image_uri VARCHAR(2048) NOT NULL,
    artifact_uri VARCHAR(2048),
    status VARCHAR(32) NOT NULL,
    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL,
    metadata JSON NOT NULL,
    metrics JSON NOT NULL,
    evaluation JSON,
    failure_reason VARCHAR(2048)
);
CREATE INDEX IF NOT EXISTS ix_releases_status ON releases (status);
