use std::cmp::Ordering;

use chia::{clvm_utils::ToTreeHash, protocol::Bytes32};
use chia_wallet_sdk::DriverError;
use clvm_traits::{FromClvm, ToClvm};
use hex_literal::hex;

// the values below are for slots organized into a double-linked ordered list
// the minimum possible value of an slot - this will be contained by one of the ends of the list
pub static SLOT32_MIN_VALUE: [u8; 32] =
    hex!("8000000000000000000000000000000000000000000000000000000000000000");
// the maximum possible value of a slot - will be contained by the other end of the list
pub static SLOT32_MAX_VALUE: [u8; 32] =
    hex!("7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");

#[derive(Debug, Clone, Copy)]
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
    pub fn new(launcher_id: Bytes32, value_hash: Bytes32) -> Result<Self, DriverError> {
        Ok(Self {
            launcher_id,
            value_hash,
            value: None,
        })
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
        let self_is_negative = self.asset_id >= Bytes32::from(SLOT32_MIN_VALUE);
        let other_is_negative = other.asset_id >= Bytes32::from(SLOT32_MIN_VALUE);

        if self_is_negative && !other_is_negative {
            return Ordering::Less;
        }

        if !self_is_negative && other_is_negative {
            return Ordering::Greater;
        }

        if self_is_negative {
            return self.asset_id.cmp(&other.asset_id);
        }

        // invert
        match self.asset_id.cmp(&other.asset_id) {
            Ordering::Less => Ordering::Greater,
            Ordering::Equal => Ordering::Equal,
            Ordering::Greater => Ordering::Less,
        }
    }
}

impl PartialOrd for CatalogSlotValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
