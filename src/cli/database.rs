use chia::protocol::{Bytes, Bytes32};
use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite};
use std::time::Duration;

use super::CliError;

pub static CATALOG_LAUNCH_LAUNCHER_ID_KEY: &str = "catalog-launch_launcher-id";
pub static SLOTS_TABLE: &str = "slots";

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

        sqlx::query(
            "
            CREATE TABLE IF NOT EXISTS slots (
                id INTEGER PRIMARY KEY,
                singleton_launcher_id BLOB NOT NULL,
                nonce INTEGER NOT NULL,
                value_hash BLOB NOT NULL,
                value BLOB NOT NULL,
                parent_parent_info BLOB NOT NULL,
                parent_inner_puzzle_hash BLOB NOT NULL,
                UNIQUE(singleton_launcher_id, nonce, value_hash)
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

    pub async fn save_slot(
        &self,
        singleton_launcher_id: Bytes32,
        nonce: u64,
        value_hash: Bytes32,
        value: Bytes,
        parent_parent_info: Bytes32,
        parent_inner_puzzle_hash: Bytes32,
    ) -> Result<(), CliError> {
        sqlx::query(
            "
            INSERT INTO slots (
                singleton_launcher_id, nonce, value_hash, value, 
                parent_parent_info, parent_inner_puzzle_hash
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(singleton_launcher_id, nonce) 
            DO UPDATE SET 
                value_hash = excluded.value_hash,
                value = excluded.value,
                parent_parent_info = excluded.parent_parent_info,
                parent_inner_puzzle_hash = excluded.parent_inner_puzzle_hash
            ",
        )
        .bind(singleton_launcher_id.to_vec())
        .bind(nonce as i64)
        .bind(value_hash.to_vec())
        .bind(value.to_vec())
        .bind(parent_parent_info.to_vec())
        .bind(parent_inner_puzzle_hash.to_vec())
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }

    pub async fn get_slot(
        &self,
        singleton_launcher_id: Bytes32,
        nonce: u64,
        value_hash: Bytes32,
    ) -> Result<Option<(Bytes, Bytes, Bytes32, Bytes32)>, CliError> {
        let row = sqlx::query(
            "
            SELECT value_hash, value, parent_parent_info, parent_inner_puzzle_hash 
            FROM slots 
            WHERE singleton_launcher_id = ?1 AND nonce = ?2 AND value_hash = ?3
            ",
        )
        .bind(singleton_launcher_id.to_vec())
        .bind(nonce as i64)
        .bind(value_hash.to_vec())
        .fetch_optional(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(row.map(|r| {
            (
                r.get::<Vec<u8>, _>("value_hash"),
                r.get::<Vec<u8>, _>("value"),
                r.get::<Vec<u8>, _>("parent_parent_info"),
                r.get::<Vec<u8>, _>("parent_inner_puzzle_hash"),
            )
        }))
    }
}
