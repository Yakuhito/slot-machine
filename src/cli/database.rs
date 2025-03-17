use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite};
use std::time::Duration;

use super::CliError;

pub static CATALOG_LAUNCH_LAUNCHER_ID_KEY: &str = "catalog-launch_launcher-id";

pub struct Db {
    pool: Pool<Sqlite>,
}

impl Db {
    pub async fn new() -> Result<Self, CliError> {
        let pool = SqlitePoolOptions::new()
            .idle_timeout(Duration::from_secs(5))
            .acquire_timeout(Duration::from_secs(5))
            .connect("sqlite://data.db")
            .await?;

        sqlx::query(
            "
            CREATE TABLE IF NOT EXISTS key_value_store (
                id INTEGER PRIMARY KEY,
                key TEXT UNIQUE NOT NULL,
                value TEXT NOT NULL
            )
            ",
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
    }

    pub async fn save_key_value(&self, key: &str, value: &str) -> Result<(), CliError> {
        sqlx::query(
            "
            INSERT INTO key_value_store (key, value) 
            VALUES (?1, ?2)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            ",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }

    pub async fn get_value_by_key(&self, key: &str) -> Result<Option<String>, CliError> {
        let row = sqlx::query(
            "
            SELECT value FROM key_value_store WHERE key = ?1
            ",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(row.map(|r| r.get(0)))
    }

    pub async fn remove_key(&self, key: &str) -> Result<(), CliError> {
        sqlx::query(
            "
            DELETE FROM key_value_store WHERE key = ?1
            ",
        )
        .bind(key)
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }
}
