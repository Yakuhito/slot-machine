use chia::{clvm_utils::ToTreeHash, protocol::Bytes32};
use chia_wallet_sdk::{coinset::CoinRecord, driver::DriverError};
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

use crate::{RewardDistributorConstants, Slot, SlotInfo, SlotProof};

use super::CliError;
pub struct Db {
    pool: Pool<Sqlite>,
}

impl Db {
    pub async fn new(skip_create_tables: bool) -> Result<Self, CliError> {
        let pool = SqlitePoolOptions::new()
            .idle_timeout(Duration::from_secs(5))
            .acquire_timeout(Duration::from_secs(5))
            .connect("sqlite://data.db?mode=rwc")
            .await?;

        if !skip_create_tables {
            sqlx::query(
                "
                CREATE TABLE IF NOT EXISTS slots (
                    singleton_launcher_id BLOB NOT NULL,
                    nonce INTEGER NOT NULL,
                    slot_value_hash BLOB NOT NULL,
                    spent_block_height INTEGER NOT NULL,
                    slot_value BLOB NOT NULL,
                    parent_parent_info BLOB NOT NULL,
                    parent_inner_puzzle_hash BLOB NOT NULL,
                    PRIMARY KEY (singleton_launcher_id, nonce, slot_value_hash, parent_parent_info)
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

            sqlx::query(
                "
                CREATE TABLE IF NOT EXISTS singleton_coins (
                    launcher_id BLOB NOT NULL,
                    coin_id BLOB NOT NULL PRIMARY KEY,
                    parent_coin_id BLOB,
                    spent_block_height NOT NULL
                )
                ",
            )
            .execute(&pool)
            .await?;

            sqlx::query(
                "
                CREATE INDEX IF NOT EXISTS idx_singleton_coins_launcher_spent 
                ON singleton_coins(launcher_id, spent_block_height)
                ",
            )
            .execute(&pool)
            .await?;

            sqlx::query(
                "
                CREATE INDEX IF NOT EXISTS idx_slots_neighbors 
                ON slots(singleton_launcher_id, nonce, spent_block_height, slot_value_hash)
                ",
            )
            .execute(&pool)
            .await?;

            sqlx::query(
                "
                CREATE TABLE IF NOT EXISTS reward_distributor_configurations (
                    launcher_id BLOB PRIMARY KEY,
                    constants BLOB NOT NULL
                )
                ",
            )
            .execute(&pool)
            .await?;

            sqlx::query(
                "
                CREATE TABLE IF NOT EXISTS dig_indexed_slot_values_by_epoch_start (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    epoch_start INTEGER NOT NULL,
                    nonce INTEGER NOT NULL,
                    slot_value_hash BLOB NOT NULL
                )
                ",
            )
            .execute(&pool)
            .await?;

            sqlx::query(
                "
                CREATE TABLE IF NOT EXISTS dig_indexed_slot_values_by_puzzle_hash (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    puzzle_hash BLOB NOT NULL,
                    nonce INTEGER NOT NULL,
                    slot_value_hash BLOB NOT NULL
                )
                ",
            )
            .execute(&pool)
            .await?;
        }

        Ok(Self { pool })
    }

    pub async fn save_slot<SV>(
        &self,
        allocator: &mut Allocator,
        slot: Slot<SV>,
        spent_block_height: u32,
    ) -> Result<(), CliError>
    where
        SV: ToClvm<Allocator> + FromClvm<Allocator> + Copy,
    {
        let slot_value_ptr = slot
            .info
            .value
            .to_clvm(allocator)
            .map_err(|err| CliError::Driver(DriverError::ToClvm(err)))?;
        let slot_value_bytes = node_to_bytes(allocator, slot_value_ptr)?;

        sqlx::query(
            "
            INSERT INTO slots (
                singleton_launcher_id, nonce, slot_value_hash, spent_block_height,
                slot_value, parent_parent_info, parent_inner_puzzle_hash
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(singleton_launcher_id, nonce, slot_value_hash, parent_parent_info) DO UPDATE SET spent_block_height = excluded.spent_block_height
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
            .map_err(|err| CliError::Driver(DriverError::FromClvm(err)))?;

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
        spent_block_height: u32,
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
        spent_block_height: u32,
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
        spent_block_height: u32,
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

    pub async fn delete_all_slots_for_singleton(
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

    pub async fn delete_all_catalog_indexed_slot_values(&self) -> Result<(), CliError> {
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
            UPDATE slots SET spent_block_height = ?1 WHERE singleton_launcher_id = ?2 AND nonce = ?3 AND slot_value_hash = ?4 AND spent_block_height=0
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

    pub async fn get_catalog_neighbors<SV>(
        &self,
        allocator: &mut Allocator,
        launcher_id: Bytes32,
        asset_id: Bytes32,
    ) -> Result<(Slot<SV>, Slot<SV>), CliError>
    where
        SV: FromClvm<Allocator> + Copy + ToTreeHash,
    {
        let query = sqlx::query(
            "
            SELECT 'left' AS side, s.* 
            FROM slots s
            WHERE s.singleton_launcher_id = ?1
              AND s.nonce = 0
              AND s.spent_block_height = 0
              AND s.slot_value_hash = (
                  SELECT slot_value_hash 
                  FROM catalog_indexed_slot_values 
                  WHERE asset_id < ?2 
                  ORDER BY asset_id DESC 
                  LIMIT 1
              )
            UNION ALL
            SELECT 'right' AS side, s.* 
            FROM slots s
            WHERE s.singleton_launcher_id = ?1
              AND s.nonce = 0
              AND s.spent_block_height = 0
              AND s.slot_value_hash = (
                  SELECT slot_value_hash 
                  FROM catalog_indexed_slot_values 
                  WHERE asset_id > ?2 
                  ORDER BY asset_id ASC 
                  LIMIT 1
              )
            ",
        )
        .bind(launcher_id.to_vec())
        .bind(asset_id.to_vec());

        let rows = query.fetch_all(&self.pool).await.map_err(CliError::Sqlx)?;

        let mut left_slots = Vec::new();
        let mut right_slots = Vec::new();

        for row in rows {
            let side: String = row.get("side");
            let slot = Self::row_to_slot(allocator, &row)?;
            if side == "left" {
                left_slots.push(slot);
            } else if side == "right" {
                right_slots.push(slot);
            }
        }

        if left_slots.is_empty() || right_slots.is_empty() {
            return Err(CliError::DbColumnNotFound());
        }

        Ok((left_slots[0], right_slots[0]))
    }

    pub async fn delete_slots_spent_before(
        &self,
        spent_block_height_threshold: u32,
    ) -> Result<(), CliError> {
        sqlx::query(
            "
            DELETE FROM slots WHERE spent_block_height>0 AND spent_block_height < ?1
            ",
        )
        .bind(spent_block_height_threshold)
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }

    pub async fn delete_all_singleton_coins(&self, launcher_id: Bytes32) -> Result<(), CliError> {
        sqlx::query(
            "
            DELETE FROM singleton_coins WHERE launcher_id = ?1
            ",
        )
        .bind(launcher_id.to_vec())
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }

    pub async fn delete_singleton_coins_spent_before(
        &self,
        spent_block_height_threshold: u32,
    ) -> Result<(), CliError> {
        sqlx::query(
            "
            DELETE FROM singleton_coins WHERE spent_block_height>0 AND spent_block_height < ?1
            ",
        )
        .bind(spent_block_height_threshold)
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }

    pub async fn save_singleton_coin(
        &self,
        launcher_id: Bytes32,
        coin_record: CoinRecord,
    ) -> Result<(), CliError> {
        sqlx::query(
            "
            UPDATE singleton_coins 
            SET spent_block_height = ?1 
            WHERE coin_id = ?2 AND spent_block_height=0
            ",
        )
        .bind(coin_record.confirmed_block_index)
        .bind(coin_record.coin.parent_coin_info.to_vec())
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        sqlx::query(
            "
            INSERT INTO singleton_coins (launcher_id, coin_id, parent_coin_id, spent_block_height) 
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(coin_id) DO NOTHING
            ",
        )
        .bind(launcher_id.to_vec())
        .bind(coin_record.coin.coin_id().to_vec())
        .bind(coin_record.coin.parent_coin_info.to_vec())
        .bind(coin_record.spent_block_index)
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }

    pub async fn get_last_unspent_singleton_coin(
        &self,
        launcher_id: Bytes32,
    ) -> Result<Option<(Bytes32, Bytes32)>, CliError> {
        let row = sqlx::query(
            "
            SELECT coin_id, parent_coin_id FROM singleton_coins WHERE launcher_id = ?1 AND spent_block_height=0 LIMIT 1
            ",
        )
        .bind(launcher_id.to_vec())
        .fetch_optional(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let coin_id = column_to_bytes32(row.get::<&[u8], _>("coin_id"))?;
        let parent_coin_id = column_to_bytes32(row.get::<&[u8], _>("parent_coin_id"))?;

        Ok(Some((coin_id, parent_coin_id)))
    }

    pub async fn save_reward_distributor_configuration(
        &self,
        allocator: &mut Allocator,
        launcher_id: Bytes32,
        constants: RewardDistributorConstants,
    ) -> Result<(), CliError> {
        let constants_ptr = constants.to_clvm(allocator).map_err(DriverError::ToClvm)?;
        let constants_bytes = node_to_bytes(allocator, constants_ptr)?;

        sqlx::query(
            "
            INSERT INTO reward_distributor_configurations (launcher_id, constants) VALUES (?1, ?2)
            ON CONFLICT(launcher_id) DO UPDATE SET constants = excluded.constants
            ",
        )
        .bind(launcher_id.to_vec())
        .bind(constants_bytes)
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }

    pub async fn get_reward_distributor_configuration(
        &self,
        allocator: &mut Allocator,
        launcher_id: Bytes32,
    ) -> Result<Option<RewardDistributorConstants>, CliError> {
        let row = sqlx::query(
            "
            SELECT constants FROM reward_distributor_configurations WHERE launcher_id = ?1
            ",
        )
        .bind(launcher_id.to_vec())
        .fetch_optional(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let constants = node_from_bytes(allocator, row.get::<&[u8], _>("constants"))?;
        let constants = RewardDistributorConstants::from_clvm(allocator, constants)
            .map_err(DriverError::FromClvm)?;

        Ok(Some(constants))
    }

    pub async fn save_dig_indexed_slot_value_by_epoch_start(
        &self,
        epoch_start: u64,
        nonce: u64,
        slot_value_hash: Bytes32,
    ) -> Result<(), CliError> {
        sqlx::query(
            "
            INSERT INTO dig_indexed_slot_values_by_epoch_start (epoch_start, nonce, slot_value_hash) VALUES (?1, ?2, ?3)
            ",
        )
        .bind(epoch_start as i64)
        .bind(nonce as i64)
        .bind(slot_value_hash.to_vec())
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }

    pub async fn save_dig_indexed_slot_value_by_puzzle_hash(
        &self,
        puzzle_hash: Bytes32,
        nonce: u64,
        slot_value_hash: Bytes32,
    ) -> Result<(), CliError> {
        sqlx::query(
            "
            INSERT INTO dig_indexed_slot_values_by_puzzle_hash (puzzle_hash, nonce, slot_value_hash) VALUES (?1, ?2, ?3)
            ",
        )
        .bind(puzzle_hash.to_vec())
        .bind(nonce as i64)
        .bind(slot_value_hash.to_vec())
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }

    pub async fn get_dig_indexed_slot_values_by_epoch_start(
        &self,
        epoch_start: u64,
        nonce: u64,
    ) -> Result<Vec<Bytes32>, CliError> {
        let row = sqlx::query(
            "
            SELECT slot_value_hash FROM dig_indexed_slot_values_by_epoch_start WHERE epoch_start = ?1 AND nonce = ?2
            ",
        )
        .bind(epoch_start as i64)
        .bind(nonce as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        row.into_iter()
            .map(|row| column_to_bytes32(row.get::<&[u8], _>("slot_value_hash")))
            .collect::<Result<Vec<_>, _>>()
    }

    pub async fn get_dig_indexed_slot_values_by_puzzle_hash(
        &self,
        puzzle_hash: Bytes32,
        nonce: u64,
    ) -> Result<Vec<Bytes32>, CliError> {
        let row = sqlx::query(
            "
            SELECT slot_value_hash FROM dig_indexed_slot_values_by_puzzle_hash WHERE puzzle_hash = ?1 AND nonce = ?2
            ",
        )
        .bind(puzzle_hash.to_vec())
        .bind(nonce as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        row.into_iter()
            .map(|row| column_to_bytes32(row.get::<&[u8], _>("slot_value_hash")))
            .collect::<Result<Vec<_>, _>>()
    }

    pub async fn delete_dig_indexed_slot_values_by_epoch_start_using_value_hash(
        &self,
        value_hash: Bytes32,
    ) -> Result<(), CliError> {
        sqlx::query(
            "
            DELETE FROM dig_indexed_slot_values_by_epoch_start WHERE slot_value_hash = ?1
            ",
        )
        .bind(value_hash.to_vec())
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }

    pub async fn delete_dig_indexed_slot_values_by_puzzle_hash_using_value_hash(
        &self,
        value_hash: Bytes32,
    ) -> Result<(), CliError> {
        sqlx::query(
            "
            DELETE FROM dig_indexed_slot_values_by_puzzle_hash WHERE slot_value_hash = ?1
            ",
        )
        .bind(value_hash.to_vec())
        .execute(&self.pool)
        .await
        .map_err(CliError::Sqlx)?;

        Ok(())
    }
}

pub fn column_to_bytes32(column_value: &[u8]) -> Result<Bytes32, CliError> {
    Ok(Bytes32::new(
        column_value
            .try_into()
            .map_err(|_| CliError::DbColumnParse())?,
    ))
}
