use chia::{clvm_utils::ToTreeHash, protocol::Bytes32};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{
    serde::{node_from_bytes, node_to_bytes},
    Allocator,
};
use sqlx::{
    sqlite::{SqlitePoolOptions, SqliteRow},
    Pool, Row, Sqlite,
};
use std::time::Duration;

use crate::{Slot, SlotInfo, SlotProof};

use super::CliError;
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
            CREATE TABLE IF NOT EXISTS slots (
                id INTEGER PRIMARY KEY,
                singleton_launcher_id BLOB NOT NULL,
                nonce INTEGER NOT NULL,
                slot_value_hash BLOB NOT NULL,
                spent_block_height INTEGER,
                slot_value BLOB NOT NULL,
                parent_parent_info BLOB NOT NULL,
                parent_inner_puzzle_hash BLOB NOT NULL,
                UNIQUE(singleton_launcher_id, nonce, slot_value_hash, spent_block_height)
            )
            ",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "
            CREATE TABLE IF NOT EXISTS catalog_indexed_slot_values (
                asset_id BLOB PRIMARY KEY,
                slot_value_hash BLOB NOT NULL
            )
            ",
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
    }

    pub async fn save_slot<SV>(
        &self,
        allocator: &mut Allocator,
        slot: Slot<SV>,
        spent_block_height: Option<u32>,
    ) -> Result<(), CliError>
    where
        SV: ToClvm<Allocator> + FromClvm<Allocator> + Copy,
    {
        let slot_value_ptr = slot
            .info
            .value
            .to_clvm(allocator)
            .map_err(|err| CliError::Driver(chia_wallet_sdk::DriverError::ToClvm(err)))?;
        let slot_value_bytes = node_to_bytes(allocator, slot_value_ptr)?;

        sqlx::query(
            "
            INSERT INTO slots (
                singleton_launcher_id, nonce, slot_value_hash, spent_block_height,
                slot_value, parent_parent_info, parent_inner_puzzle_hash
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ",
        )
        .bind(slot.info.launcher_id.to_vec())
        .bind(slot.info.nonce as i64)
        .bind(slot.info.value_hash.to_vec())
        .bind(spent_block_height)
        .bind(slot_value_bytes)
        .bind(slot.proof.parent_parent_info.to_vec())
        .bind(slot.proof.parent_inner_puzzle_hash.to_vec())
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }

    fn row_to_slot<SV>(allocator: &mut Allocator, row: &SqliteRow) -> Result<Slot<SV>, CliError>
    where
        SV: FromClvm<Allocator> + Copy + ToTreeHash,
    {
        let launcher_id = column_to_bytes32(row.get::<&[u8], _>("singleton_launcher_id"))?;
        let nonce = row.get::<i64, _>("nonce") as u64;
        let parent_parent_info = column_to_bytes32(row.get::<&[u8], _>("parent_parent_info"))?;
        let parent_inner_puzzle_hash =
            column_to_bytes32(row.get::<&[u8], _>("parent_inner_puzzle_hash"))?;

        let value = node_from_bytes(allocator, row.get::<&[u8], _>("slot_value"))?;
        let value = SV::from_clvm(allocator, value)
            .map_err(|err| CliError::Driver(chia_wallet_sdk::DriverError::FromClvm(err)))?;

        Ok(Slot::new(
            SlotProof {
                parent_parent_info,
                parent_inner_puzzle_hash,
            },
            SlotInfo::<SV>::from_value(launcher_id, nonce, value),
        ))
    }

    pub async fn get_slot<SV>(
        &self,
        allocator: &mut Allocator,
        singleton_launcher_id: Bytes32,
        nonce: u64,
        slot_value_hash: Bytes32,
        spent_block_height: Option<u32>,
    ) -> Result<Option<Slot<SV>>, CliError>
    where
        SV: FromClvm<Allocator> + Copy + ToTreeHash,
    {
        let row = sqlx::query(
            "
            SELECT * FROM slots 
            WHERE singleton_launcher_id = ?1 AND nonce = ?2 AND slot_value_hash = ?3 AND spent_block_height = ?4
            ",
        )
        .bind(singleton_launcher_id.to_vec())
        .bind(nonce as i64)
        .bind(slot_value_hash.to_vec())
        .bind(spent_block_height)
        .fetch_optional(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        let Some(row) = row else {
            return Ok(None);
        };

        Ok(Some(Self::row_to_slot(allocator, &row)?))
    }

    pub async fn get_slots<SV>(
        &self,
        allocator: &mut Allocator,
        singleton_launcher_id: Bytes32,
        nonce: u64,
        spent_block_height: Option<u32>,
    ) -> Result<Vec<Slot<SV>>, CliError>
    where
        SV: FromClvm<Allocator> + Copy + ToTreeHash,
    {
        let rows =
            sqlx::query("SELECT * FROM slots WHERE singleton_launcher_id = ?1 AND nonce = ?2 AND spent_block_height = ?3")
                .bind(singleton_launcher_id.to_vec())
                .bind(nonce as i64)
                .bind(spent_block_height)
                .fetch_all(&self.pool)
                .await
                .map_err(CliError::Sqlx)?;

        rows.into_iter()
            .map(|row| Self::row_to_slot(allocator, &row))
            .collect::<Result<Vec<_>, _>>()
    }

    pub async fn remove_slot(
        &self,
        singleton_launcher_id: Bytes32,
        nonce: u64,
        slot_value_hash: Bytes32,
        spent_block_height: Option<u32>,
    ) -> Result<(), CliError> {
        sqlx::query(
            "
            DELETE FROM slots WHERE singleton_launcher_id = ?1 AND nonce = ?2 AND slot_value_hash = ?3 AND spent_block_height = ?4
            ",
        )
        .bind(singleton_launcher_id.to_vec())
        .bind(nonce as i64)
        .bind(slot_value_hash.to_vec())
        .bind(spent_block_height)
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }

    pub async fn clear_slots_for_singleton(
        &self,
        singleton_launcher_id: Bytes32,
    ) -> Result<(), CliError> {
        sqlx::query(
            "
            DELETE FROM slots WHERE singleton_launcher_id = ?1
            ",
        )
        .bind(singleton_launcher_id.to_vec())
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }

    pub async fn clear_catalog_indexed_slot_values(&self) -> Result<(), CliError> {
        sqlx::query(
            "
            DELETE FROM catalog_indexed_slot_values
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }

    pub async fn mark_slot_as_spent(
        &self,
        singleton_launcher_id: Bytes32,
        nonce: u64,
        slot_value_hash: Bytes32,
        spent_block_height: u32,
    ) -> Result<(), CliError> {
        sqlx::query(
            "
            UPDATE slots SET spent_block_height = ?1 WHERE singleton_launcher_id = ?2 AND nonce = ?3 AND slot_value_hash = ?4 AND spent_block_height IS NULL
            ",
        )
        .bind(spent_block_height)
        .bind(singleton_launcher_id.to_vec())
        .bind(nonce as i64)
        .bind(slot_value_hash.to_vec())
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }

    pub async fn save_catalog_indexed_slot_value(
        &self,
        asset_id: Bytes32,
        slot_value_hash: Bytes32,
    ) -> Result<(), CliError> {
        sqlx::query(
            "
            INSERT INTO catalog_indexed_slot_values (asset_id, slot_value_hash) 
            VALUES (?1, ?2)
            ON CONFLICT(asset_id) DO UPDATE SET slot_value_hash = excluded.slot_value_hash
            ",
        )
        .bind(asset_id.to_vec())
        .bind(slot_value_hash.to_vec())
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }

    pub async fn get_catalog_indexed_slot_value(
        &self,
        asset_id: Bytes32,
    ) -> Result<Option<Bytes32>, CliError> {
        let row = sqlx::query(
            "
            SELECT slot_value_hash FROM catalog_indexed_slot_values WHERE asset_id = ?1
            ",
        )
        .bind(asset_id.to_vec())
        .fetch_optional(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        let Some(row) = row else {
            return Ok(None);
        };

        Ok(Some(column_to_bytes32(
            row.get::<&[u8], _>("slot_value_hash"),
        )?))
    }

    pub async fn get_catalog_neighbor_value_hashes(
        &self,
        asset_id: Bytes32,
    ) -> Result<(Bytes32, Bytes32), CliError> {
        // First byte > 0x7F means negative number in signed representation
        let is_negative = asset_id.as_ref()[0] > 0x7F;

        // Get the previous (lower) neighbor
        let mut lower_row = if is_negative {
            sqlx::query(
                "
                SELECT slot_value_hash, asset_id FROM catalog_indexed_slot_values 
                WHERE asset_id < ?1 AND asset_id >= x'8000000000000000000000000000000000000000000000000000000000000000'
                ORDER BY asset_id ASC
                LIMIT 1
                ",
            )
        } else {
            sqlx::query(
                "
                SELECT slot_value_hash, asset_id FROM catalog_indexed_slot_values 
                WHERE asset_id < ?1 AND asset_id < x'8000000000000000000000000000000000000000000000000000000000000000'
                ORDER BY asset_id DESC
                LIMIT 1
                ",
            )
        }
        .bind(asset_id.to_vec())
        .fetch_optional(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        // If no lower neighbor in same sign range, wrap around to the maximum value of opposite sign
        if lower_row.is_none() {
            lower_row = if is_negative {
                // If we're negative and found no lower, fff
                eprintln!("negative and found no lower");
                return Err(CliError::DbColumnParse());
            } else {
                sqlx::query(
                    "
                    SELECT slot_value_hash, asset_id FROM catalog_indexed_slot_values 
                    WHERE asset_id >= x'8000000000000000000000000000000000000000000000000000000000000000'
                    ORDER BY asset_id DESC
                    LIMIT 1
                    ",
                )
            }
            .fetch_optional(&self.pool)
            .await
            .map_err(CliError::Sqlx)?;
        }

        // Get the next (higher) neighbor
        let mut higher_row = if is_negative {
            sqlx::query(
                "
                SELECT slot_value_hash, asset_id FROM catalog_indexed_slot_values 
                WHERE asset_id > ?1 AND asset_id >= x'8000000000000000000000000000000000000000000000000000000000000000'
                ORDER BY asset_id DESC
                LIMIT 1
                ",
            )
        } else {
            sqlx::query(
                "
                SELECT slot_value_hash, asset_id FROM catalog_indexed_slot_values 
                WHERE asset_id > ?1 AND asset_id < x'8000000000000000000000000000000000000000000000000000000000000000'
                ORDER BY asset_id ASC
                LIMIT 1
                ",
            )
        }
        .bind(asset_id.to_vec())
        .fetch_optional(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        if higher_row.is_none() {
            higher_row = if is_negative {
                sqlx::query(
                    "
                    SELECT slot_value_hash, asset_id FROM catalog_indexed_slot_values 
                    WHERE asset_id < x'8000000000000000000000000000000000000000000000000000000000000000'
                    ORDER BY asset_id ASC
                    LIMIT 1
                    ",
                )
            } else {
                eprintln!("positive and found no higher");
                return Err(CliError::DbColumnParse());
            }
            .fetch_optional(&self.pool)
            .await
            .map_err(CliError::Sqlx)?;
        }

        let Some(lower_row) = lower_row else {
            return Err(CliError::DbColumnParse());
        };
        let Some(higher_row) = higher_row else {
            return Err(CliError::DbColumnParse());
        };

        let lower_hash = column_to_bytes32(lower_row.get::<&[u8], _>("slot_value_hash"))?;
        let higher_hash = column_to_bytes32(higher_row.get::<&[u8], _>("slot_value_hash"))?;

        Ok((lower_hash, higher_hash))
    }
}

pub fn column_to_bytes32(column_value: &[u8]) -> Result<Bytes32, CliError> {
    Ok(Bytes32::new(
        column_value
            .try_into()
            .map_err(|_| CliError::DbColumnParse())?,
    ))
}
