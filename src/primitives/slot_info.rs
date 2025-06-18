use std::cmp::Ordering;

use chia::{clvm_utils::ToTreeHash, protocol::Bytes32};
use clvm_traits::{FromClvm, ToClvm};
use hex_literal::hex;

// comparison is >s, not >
// previous min was 0x8000000000000000000000000000000000000000000000000000000000000000
// and previous max was 0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
pub static SLOT32_MIN_VALUE: [u8; 32] =
    hex!("0000000000000000000000000000000000000000000000000000000000000000");
// the maximum possible value of a slot - will be contained by the other end of the list
pub static SLOT32_MAX_VALUE: [u8; 32] =
    hex!("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct SlotInfo<V>
where
    V: Copy,
{
    pub nonce: u64,
    pub launcher_id: Bytes32,

    pub value_hash: Bytes32,
    pub value: V,
}

impl<V> SlotInfo<V>
where
    V: Copy,
{
    pub fn new(launcher_id: Bytes32, nonce: u64, value_hash: Bytes32, value: V) -> Self {
        Self {
            launcher_id,
            nonce,
            value_hash,
            value,
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
            value,
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

    pub fn initial_left_end() -> Self {
        Self::new(
            SLOT32_MIN_VALUE.into(),
            SLOT32_MIN_VALUE.into(),
            SLOT32_MAX_VALUE.into(),
        )
    }

    pub fn initial_right_end() -> Self {
        Self::new(
            SLOT32_MAX_VALUE.into(),
            SLOT32_MIN_VALUE.into(),
            SLOT32_MAX_VALUE.into(),
        )
    }
}

impl Ord for CatalogSlotValue {
    fn cmp(&self, other: &Self) -> Ordering {
        self.asset_id.cmp(&other.asset_id)
    }
}

impl PartialOrd for CatalogSlotValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct XchandlesSlotFirstPart {
    pub handle_hash: Bytes32,
    #[clvm(rest)]
    pub neighbors: SlotNeigborsInfo,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct XchandlesSlotLauncherIds {
    pub owner_launcher_id: Bytes32,
    #[clvm(rest)]
    pub resolved_launcher_id: Bytes32,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct XchandlesSlotSecondPart {
    pub expiration: u64,
    #[clvm(rest)]
    pub launcher_ids: XchandlesSlotLauncherIds,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct XchandlesSlotValue {
    pub first_part: XchandlesSlotFirstPart,
    pub second_part: XchandlesSlotSecondPart,
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
            first_part: XchandlesSlotFirstPart {
                handle_hash,
                neighbors: SlotNeigborsInfo {
                    left_value: left_handle_hash,
                    right_value: right_handle_hash,
                },
            },
            second_part: XchandlesSlotSecondPart {
                expiration,
                launcher_ids: XchandlesSlotLauncherIds {
                    owner_launcher_id,
                    resolved_launcher_id,
                },
            },
        }
    }

    pub fn initial_left_end() -> Self {
        XchandlesSlotValue::new(
            SLOT32_MIN_VALUE.into(),
            SLOT32_MIN_VALUE.into(),
            SLOT32_MAX_VALUE.into(),
            u64::MAX,
            Bytes32::default(),
            Bytes32::default(),
        )
    }

    pub fn initial_right_end() -> Self {
        XchandlesSlotValue::new(
            SLOT32_MAX_VALUE.into(),
            SLOT32_MIN_VALUE.into(),
            SLOT32_MAX_VALUE.into(),
            u64::MAX,
            Bytes32::default(),
            Bytes32::default(),
        )
    }

    pub fn with_neighbors(&self, left_handle_hash: Bytes32, right_handle_hash: Bytes32) -> Self {
        Self {
            first_part: XchandlesSlotFirstPart {
                handle_hash: self.first_part.handle_hash,
                neighbors: SlotNeigborsInfo {
                    left_value: left_handle_hash,
                    right_value: right_handle_hash,
                },
            },
            second_part: self.second_part,
        }
    }

    pub fn with_expiration(self, expiration: u64) -> Self {
        Self {
            first_part: self.first_part,
            second_part: XchandlesSlotSecondPart {
                expiration,
                launcher_ids: self.second_part.launcher_ids,
            },
        }
    }

    pub fn with_launcher_ids(
        self,
        owner_launcher_id: Bytes32,
        resolved_launcher_id: Bytes32,
    ) -> Self {
        Self {
            first_part: self.first_part,
            second_part: XchandlesSlotSecondPart {
                expiration: self.second_part.expiration,
                launcher_ids: XchandlesSlotLauncherIds {
                    owner_launcher_id,
                    resolved_launcher_id,
                },
            },
        }
    }
}

impl Ord for XchandlesSlotValue {
    fn cmp(&self, other: &Self) -> Ordering {
        self.first_part
            .handle_hash
            .cmp(&other.first_part.handle_hash)
    }
}

impl PartialOrd for XchandlesSlotValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewardDistributorSlotNonce {
    REWARD = 1,
    COMMITMENT = 2,
    ENTRY = 3,
}

impl RewardDistributorSlotNonce {
    pub fn from_u64(value: u64) -> Option<Self> {
        match value {
            1 => Some(Self::REWARD),
            2 => Some(Self::COMMITMENT),
            3 => Some(Self::ENTRY),
            _ => None,
        }
    }

    pub fn to_u64(self) -> u64 {
        match self {
            Self::REWARD => 1,
            Self::COMMITMENT => 2,
            Self::ENTRY => 3,
        }
    }
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct RewardDistributorRewardSlotValue {
    pub epoch_start: u64,
    pub next_epoch_initialized: bool,
    #[clvm(rest)]
    pub rewards: u64,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct RewardDistributorCommitmentSlotValue {
    pub epoch_start: u64,
    pub clawback_ph: Bytes32,
    #[clvm(rest)]
    pub rewards: u64,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct RewardDistributorEntrySlotValue {
    pub payout_puzzle_hash: Bytes32,
    pub initial_cumulative_payout: u64,
    #[clvm(rest)]
    pub shares: u64,
}
