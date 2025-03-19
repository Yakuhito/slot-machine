use std::cmp::Ordering;

use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use clvm_traits::{clvm_tuple, FromClvm, ToClvm};
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
    pub nonce: u64,
    pub launcher_id: Bytes32,

    pub value_hash: Bytes32,
    pub value: Option<V>,
}

impl<V> SlotInfo<V>
where
    V: Copy,
{
    pub fn new(launcher_id: Bytes32, nonce: u64, value_hash: Bytes32) -> Self {
        Self {
            launcher_id,
            nonce,
            value_hash,
            value: None,
        }
    }

    pub fn from_value(launcher_id: Bytes32, nonce: u64, value: V) -> Self
    where
        V: ToTreeHash,
    {
        Self {
            launcher_id,
            nonce,
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

    pub fn left_end(right_asset_id: Bytes32) -> Self {
        Self::new(
            SLOT32_MIN_VALUE.into(),
            SLOT32_MIN_VALUE.into(),
            right_asset_id,
        )
    }

    pub fn right_end(left_asset_id: Bytes32) -> Self {
        Self::new(
            SLOT32_MAX_VALUE.into(),
            left_asset_id,
            SLOT32_MAX_VALUE.into(),
        )
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
pub struct XchandlesSlotValue {
    pub handle_hash: Bytes32,
    pub neighbors: SlotNeigborsInfo,
    pub expiration: u64,
    pub owner_launcher_id: Bytes32,
    #[clvm(rest)]
    pub resolved_launcher_id: Bytes32,
}

impl XchandlesSlotValue {
    pub fn new(
        handle_hash: Bytes32,
        left_handle_hash: Bytes32,
        right_handle_hash: Bytes32,
        expiration: u64,
        owner_launcher_id: Bytes32,
        resolved_launcher_id: Bytes32,
    ) -> Self {
        Self {
            handle_hash,
            neighbors: SlotNeigborsInfo {
                left_value: left_handle_hash,
                right_value: right_handle_hash,
            },
            expiration,
            owner_launcher_id,
            resolved_launcher_id,
        }
    }

    pub fn edge(
        handle_hash: Bytes32,
        left_handle_hash: Bytes32,
        right_handle_hash: Bytes32,
    ) -> Self {
        Self {
            handle_hash,
            neighbors: SlotNeigborsInfo {
                left_value: left_handle_hash,
                right_value: right_handle_hash,
            },
            expiration: 0,
            owner_launcher_id: Bytes32::default(),
            resolved_launcher_id: Bytes32::default(),
        }
    }

    pub fn with_neighbors(&self, left_handle_hash: Bytes32, right_handle_hash: Bytes32) -> Self {
        Self {
            handle_hash: self.handle_hash,
            neighbors: SlotNeigborsInfo {
                left_value: left_handle_hash,
                right_value: right_handle_hash,
            },
            expiration: self.expiration,
            owner_launcher_id: self.owner_launcher_id,
            resolved_launcher_id: self.resolved_launcher_id,
        }
    }

    pub fn after_handle_data_hash(&self) -> TreeHash {
        clvm_tuple!(
            self.neighbors,
            clvm_tuple!(
                self.expiration,
                clvm_tuple!(self.owner_launcher_id, self.resolved_launcher_id),
            )
        )
        .tree_hash()
    }

    pub fn after_neigbors_data_hash(&self) -> TreeHash {
        clvm_tuple!(
            self.expiration,
            clvm_tuple!(self.owner_launcher_id, self.resolved_launcher_id),
        )
        .tree_hash()
    }

    pub fn launcher_ids_data_hash(&self) -> TreeHash {
        clvm_tuple!(self.owner_launcher_id, self.resolved_launcher_id).tree_hash()
    }

    pub fn with_expiration(self, expiration: u64) -> Self {
        Self {
            handle_hash: self.handle_hash,
            neighbors: self.neighbors,
            expiration,
            owner_launcher_id: self.owner_launcher_id,
            resolved_launcher_id: self.resolved_launcher_id,
        }
    }

    pub fn with_launcher_ids(
        self,
        owner_launcher_id: Bytes32,
        resolved_launcher_id: Bytes32,
    ) -> Self {
        Self {
            handle_hash: self.handle_hash,
            neighbors: self.neighbors,
            expiration: self.expiration,
            owner_launcher_id,
            resolved_launcher_id,
        }
    }
}

impl Ord for XchandlesSlotValue {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_num = BigInt::from_signed_bytes_be(&self.handle_hash);
        let other_num = BigInt::from_signed_bytes_be(&other.handle_hash);

        self_num.cmp(&other_num)
    }
}

impl PartialOrd for XchandlesSlotValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigSlotNonce {
    REWARD = 1,
    COMMITMENT = 2,
    MIRROR = 3,
}

impl DigSlotNonce {
    pub fn from_u64(value: u64) -> Option<Self> {
        match value {
            1 => Some(Self::REWARD),
            2 => Some(Self::COMMITMENT),
            3 => Some(Self::MIRROR),
            _ => None,
        }
    }

    pub fn to_u64(self) -> u64 {
        match self {
            Self::REWARD => 1,
            Self::COMMITMENT => 2,
            Self::MIRROR => 3,
        }
    }
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct DigRewardSlotValue {
    pub epoch_start: u64,
    pub next_epoch_initialized: bool,
    #[clvm(rest)]
    pub rewards: u64,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct DigCommitmentSlotValue {
    pub epoch_start: u64,
    pub clawback_ph: Bytes32,
    #[clvm(rest)]
    pub rewards: u64,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct DigMirrorSlotValue {
    pub payout_puzzle_hash: Bytes32,
    pub initial_cumulative_payout: u64,
    #[clvm(rest)]
    pub shares: u64,
}
