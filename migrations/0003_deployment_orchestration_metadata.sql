ALTER TABLE deployment_attempts
    ADD COLUMN orchestration_metadata JSON NOT NULL DEFAULT '{}';
