use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes, Bytes32},
};
use clvm_traits::FromClvm;
use clvmr::{serde::node_from_bytes, Allocator};
use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite};
use std::time::Duration;

use crate::{Slot, SlotInfo, SlotProof};

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

    pub async fn get_slot<SV>(
        &self,
        allocator: &mut Allocator,
        singleton_launcher_id: Bytes32,
        nonce: u64,
        value_hash: Bytes32,
    ) -> Result<Option<Slot<SV>>, CliError>
    where
        SV: FromClvm<Allocator> + Copy + ToTreeHash,
    {
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

        let Some(row) = row else {
            return Ok(None);
        };

        let launcher_id = column_to_bytes32(row.get::<&[u8], _>("singleton_launcher_id"))?;
        let nonce = row.get::<i64, _>("nonce") as u64;
        let parent_parent_info = column_to_bytes32(row.get::<&[u8], _>("parent_parent_info"))?;
        let parent_inner_puzzle_hash =
            column_to_bytes32(row.get::<&[u8], _>("parent_inner_puzzle_hash"))?;

        let value = node_from_bytes(allocator, row.get::<&[u8], _>("value"))?;
        let value = SV::from_clvm(allocator, value)
            .map_err(|err| CliError::Driver(chia_wallet_sdk::DriverError::FromClvm(err)))?;

        Ok(Some(Slot::new(
            SlotProof {
                parent_parent_info,
                parent_inner_puzzle_hash,
            },
            SlotInfo::<SV>::from_value(launcher_id, nonce, value),
        )))
    }
}

pub fn column_to_bytes32(column_value: &[u8]) -> Result<Bytes32, CliError> {
    Ok(Bytes32::new(
        column_value
            .try_into()
            .map_err(|_| CliError::DbColumnParse())?,
    ))
}
