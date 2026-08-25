"""Application configuration sourced from environment variables."""

from __future__ import annotations

import os
from dataclasses import dataclass

DEFAULT_DATABASE_URL = "sqlite:///./ml_release_platform.db"


@dataclass(frozen=True, slots=True)
class Settings:
    """Runtime configuration with safe local defaults."""

    database_url: str = DEFAULT_DATABASE_URL

    @classmethod
    def from_environment(cls) -> Settings:
        return cls(database_url=os.getenv("ML_RELEASE_DATABASE_URL", DEFAULT_DATABASE_URL))
