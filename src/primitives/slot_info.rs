use std::cmp::Ordering;

use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use clvm_traits::{FromClvm, ToClvm};
use hex_literal::hex;
use num_bigint::BigInt;

// the values below are for slots organized into a double-linked ordered list
// the minimum possible value of an slot - this will be contained by one of the ends of the list
pub static SLOT32_MIN_VALUE: [u8; 32] =
    hex!("8000000000000000000000000000000000000000000000000000000000000000");
// the maximum possible value of a slot - will be contained by the other end of the list
pub static SLOT32_MAX_VALUE: [u8; 32] =
    hex!("7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct SlotInfo<V>
where
    V: Copy,
{
    pub launcher_id: Bytes32,

    pub value_hash: Bytes32,
    pub value: Option<V>,
}

impl<V> SlotInfo<V>
where
    V: Copy,
{
    pub fn new(launcher_id: Bytes32, value_hash: Bytes32) -> Self {
        Self {
            launcher_id,
            value_hash,
            value: None,
        }
    }

    pub fn from_value(launcher_id: Bytes32, value: V) -> Self
    where
        V: ToTreeHash,
    {
        Self {
            launcher_id,
            value_hash: value.tree_hash().into(),
            value: Some(value),
        }
    }
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct SlotNeigborsInfo {
    pub left_value: Bytes32,
    #[clvm(rest)]
    pub right_value: Bytes32,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct CatalogSlotValue {
    pub asset_id: Bytes32,
    #[clvm(rest)]
    pub neighbors: SlotNeigborsInfo,
}

impl CatalogSlotValue {
    pub fn new(asset_id: Bytes32, left_asset_id: Bytes32, right_asset_id: Bytes32) -> Self {
        Self {
            asset_id,
            neighbors: SlotNeigborsInfo {
                left_value: left_asset_id,
                right_value: right_asset_id,
            },
        }
    }
}

impl Ord for CatalogSlotValue {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_num = BigInt::from_signed_bytes_be(&self.asset_id);
        let other_num = BigInt::from_signed_bytes_be(&other.asset_id);

        self_num.cmp(&other_num)
    }
}

impl PartialOrd for CatalogSlotValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct CnsSlotValue {
    pub name_hash: Bytes32,
    pub neighbors: SlotNeigborsInfo,
    pub expiration: u64,
    pub version: u32,
    #[clvm(rest)]
    pub launcher_id: Bytes32,
}

impl CnsSlotValue {
    pub fn new(
        name_hash: Bytes32,
        left_name_hash: Bytes32,
        right_name_hash: Bytes32,
        expiration: u64,
        version: u32,
        launcher_id: Bytes32,
    ) -> Self {
        Self {
            name_hash,
            neighbors: SlotNeigborsInfo {
                left_value: left_name_hash,
                right_value: right_name_hash,
            },
            expiration,
            version,
            launcher_id,
        }
    }

    pub fn with_neighbors(&self, left_name_hash: Bytes32, right_name_hash: Bytes32) -> Self {
        Self {
            name_hash: self.name_hash,
            neighbors: SlotNeigborsInfo {
                left_value: left_name_hash,
                right_value: right_name_hash,
            },
            expiration: self.expiration,
            version: self.version,
            launcher_id: self.launcher_id,
        }
    }

    pub fn after_neigbors_data_hash(&self) -> TreeHash {
        CnsSlotValueWithoutNameAndNeighbors {
            expiration: self.expiration,
            version: self.version,
            launcher_id: self.launcher_id,
        }
        .tree_hash()
    }
}

impl Ord for CnsSlotValue {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_num = BigInt::from_signed_bytes_be(&self.name_hash);
        let other_num = BigInt::from_signed_bytes_be(&other.name_hash);

        self_num.cmp(&other_num)
    }
}

impl PartialOrd for CnsSlotValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct CnsSlotValueWithoutNameAndNeighbors {
    pub expiration: u64,
    pub version: u32,
    #[clvm(rest)]
    pub launcher_id: Bytes32,
}
