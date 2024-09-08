use std::cmp::Ordering;

use chia::{clvm_utils::ToTreeHash, protocol::Bytes32};
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
pub struct CatalogSlotNeigborsInfo {
    pub left_asset_id: Bytes32,
    #[clvm(rest)]
    pub right_asset_id: Bytes32,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct CatalogSlotValue {
    pub asset_id: Bytes32,
    #[clvm(rest)]
    pub neighbors: CatalogSlotNeigborsInfo,
}

impl CatalogSlotValue {
    pub fn new(asset_id: Bytes32, left_asset_id: Bytes32, right_asset_id: Bytes32) -> Self {
        Self {
            asset_id,
            neighbors: CatalogSlotNeigborsInfo {
                left_asset_id,
                right_asset_id,
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
