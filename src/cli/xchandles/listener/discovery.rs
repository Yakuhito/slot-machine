use chia_protocol::{Bytes32, Coin, CoinSpend};
use chia_puzzle_types::singleton::SingletonArgs;
use chia_wallet_sdk::driver::{DriverError, Layer, Nft, NftInfo, Puzzle, SingletonInfo, SingletonLayer};
use chia_wallet_sdk::types::{run_puzzle, Condition};
use clvm_traits::FromClvm;
use clvm_utils::ToTreeHash;
use clvmr::serde::{node_from_bytes, node_to_bytes};
use clvmr::{Allocator, NodePtr};
use serde::{Deserialize, Serialize};

use super::types::SingletonNftDetails;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSingleton {
    pub coin: Coin,
    pub inner_puzzle_hash: Bytes32,
    /// True when the matched spend created no odd child (terminal melt at discovery).
    pub melted: bool,
    pub nft: Option<ParsedNftState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedNftState {
    pub metadata_treehash: Bytes32,
    pub metadata_updater_puzzle_hash: Bytes32,
    pub current_owner: Option<Bytes32>,
    pub royalty_puzzle_hash: Bytes32,
    pub royalty_basis_points: u16,
    pub p2_puzzle_hash: Bytes32,
    pub metadata_clvm: Option<Vec<u8>>,
}

impl ParsedNftState {
    pub fn from_nft_info(allocator: &Allocator, info: &NftInfo) -> Result<Self, DriverError> {
        let metadata_clvm = node_to_bytes(allocator, info.metadata.ptr())
            .map_err(|e| DriverError::Custom(e.to_string()))?;
        Ok(Self {
            metadata_treehash: info.metadata.tree_hash().into(),
            metadata_updater_puzzle_hash: info.metadata_updater_puzzle_hash,
            current_owner: info.current_owner,
            royalty_puzzle_hash: info.royalty_puzzle_hash,
            royalty_basis_points: info.royalty_basis_points,
            p2_puzzle_hash: info.p2_puzzle_hash,
            metadata_clvm: Some(metadata_clvm),
        })
    }

    pub fn to_public(&self, include_metadata: bool) -> SingletonNftDetails {
        SingletonNftDetails {
            metadata_treehash: hex::encode(self.metadata_treehash.to_bytes()),
            metadata_updater_puzzle_hash: hex::encode(self.metadata_updater_puzzle_hash.to_bytes()),
            current_owner: self.current_owner.map(|id| hex::encode(id.to_bytes())),
            royalty_puzzle_hash: hex::encode(self.royalty_puzzle_hash.to_bytes()),
            royalty_basis_points: self.royalty_basis_points,
            p2_puzzle_hash: hex::encode(self.p2_puzzle_hash.to_bytes()),
            metadata: if include_metadata {
                self.metadata_clvm.as_ref().map(hex::encode)
            } else {
                None
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryResult {
    /// Zero matching odd singleton children — retryable incomplete state.
    Incomplete,
    /// Exactly one match.
    Found(DiscoveredSingleton),
    /// Multiple matches — integrity failure.
    Mismatch,
}

/// Discover a newly referenced singleton only from spends in the registry transition block.
///
/// Never traverses arbitrary launcher history. Inspects each spend, parses the singleton layer,
/// requires launcher ID + expected full puzzle hash to match, then takes the odd child (if any)
/// and attempts NFT parsing.
pub fn discover_singleton_in_block(
    allocator: &mut Allocator,
    spends: &[CoinSpend],
    launcher_id: Bytes32,
    expected_full_puzzle_hash: Bytes32,
    expected_inner_puzzle_hash: Bytes32,
) -> Result<DiscoveryResult, DriverError> {
    let mut matches = Vec::new();

    for spend in spends {
        if let Some(found) = try_match_spend(
            allocator,
            spend,
            launcher_id,
            expected_full_puzzle_hash,
            expected_inner_puzzle_hash,
        )? {
            matches.push(found);
        }
    }

    Ok(match matches.len() {
        0 => DiscoveryResult::Incomplete,
        1 => DiscoveryResult::Found(matches.remove(0)),
        _ => DiscoveryResult::Mismatch,
    })
}

fn try_match_spend(
    allocator: &mut Allocator,
    spend: &CoinSpend,
    launcher_id: Bytes32,
    expected_full_puzzle_hash: Bytes32,
    expected_inner_puzzle_hash: Bytes32,
) -> Result<Option<DiscoveredSingleton>, DriverError> {
    if spend.coin.puzzle_hash != expected_full_puzzle_hash {
        return Ok(None);
    }

    let puzzle_ptr = node_from_program(allocator, &spend.puzzle_reveal)?;
    let puzzle = Puzzle::parse(allocator, puzzle_ptr);
    let Some(singleton) = SingletonLayer::<Puzzle>::parse_puzzle(allocator, puzzle)? else {
        return Ok(None);
    };
    if singleton.launcher_id != launcher_id {
        return Ok(None);
    }

    let solution_ptr = node_from_program(allocator, &spend.solution)?;

    // NFT lineage: derive the odd child directly from the parent spend.
    if let Some(child_nft) = Nft::parse_child(allocator, spend.coin, puzzle, solution_ptr)? {
        let nft = ParsedNftState::from_nft_info(allocator, &child_nft.info)?;
        return Ok(Some(DiscoveredSingleton {
            coin: child_nft.coin,
            inner_puzzle_hash: child_nft.info.inner_puzzle_hash().into(),
            melted: false,
            nft: Some(nft),
        }));
    }

    let output = run_puzzle(allocator, puzzle_ptr, solution_ptr)?;
    let conditions =
        Vec::<Condition<NodePtr>>::from_clvm(allocator, output).map_err(DriverError::FromClvm)?;
    let odd_children: Vec<_> = conditions
        .into_iter()
        .filter_map(|c| match c {
            Condition::CreateCoin(cc) if cc.amount % 2 == 1 => Some(cc),
            _ => None,
        })
        .collect();

    if let Some(child) = odd_children.first() {
        let child_coin = Coin::new(spend.coin.coin_id(), child.puzzle_hash, child.amount);
        let expected_full: Bytes32 =
            SingletonArgs::curry_tree_hash(launcher_id, expected_inner_puzzle_hash.into()).into();
        let parent_inner: Bytes32 = singleton.inner_puzzle.tree_hash().into();
        let parent_full: Bytes32 =
            SingletonArgs::curry_tree_hash(launcher_id, parent_inner.into()).into();
        let inner_puzzle_hash = if expected_full == child.puzzle_hash {
            expected_inner_puzzle_hash
        } else if parent_full == child.puzzle_hash {
            parent_inner
        } else {
            expected_inner_puzzle_hash
        };
        return Ok(Some(DiscoveredSingleton {
            coin: child_coin,
            inner_puzzle_hash,
            melted: false,
            nft: None,
        }));
    }

    // Spent with no odd child → melted terminal. Last known coin is the spent one.
    let nft = match NftInfo::parse(allocator, puzzle)? {
        Some((info, _)) => Some(ParsedNftState::from_nft_info(allocator, &info)?),
        None => None,
    };
    Ok(Some(DiscoveredSingleton {
        coin: spend.coin,
        inner_puzzle_hash: singleton.inner_puzzle.tree_hash().into(),
        melted: true,
        nft,
    }))
}

fn node_from_program(
    allocator: &mut Allocator,
    program: &chia_protocol::Program,
) -> Result<NodePtr, DriverError> {
    node_from_bytes(allocator, program).map_err(|e| DriverError::Custom(e.to_string()))
}

/// Follow one subsequent singleton spend from the current coin.
/// Returns the next live coin state, or `Melted` when there is no odd child.
pub fn follow_singleton_spend(
    allocator: &mut Allocator,
    spend: &CoinSpend,
    launcher_id: Bytes32,
) -> Result<FollowSpendResult, DriverError> {
    let puzzle_ptr = node_from_program(allocator, &spend.puzzle_reveal)?;
    let puzzle = Puzzle::parse(allocator, puzzle_ptr);
    let Some(singleton) = SingletonLayer::<Puzzle>::parse_puzzle(allocator, puzzle)? else {
        return Err(DriverError::Custom(
            "current singleton spend is not a singleton".into(),
        ));
    };
    if singleton.launcher_id != launcher_id {
        return Err(DriverError::Custom(
            "singleton spend launcher ID mismatch".into(),
        ));
    }

    let solution_ptr = node_from_program(allocator, &spend.solution)?;

    if let Some(child_nft) = Nft::parse_child(allocator, spend.coin, puzzle, solution_ptr)? {
        let nft = ParsedNftState::from_nft_info(allocator, &child_nft.info)?;
        return Ok(FollowSpendResult::Next(DiscoveredSingleton {
            coin: child_nft.coin,
            inner_puzzle_hash: child_nft.info.inner_puzzle_hash().into(),
            melted: false,
            nft: Some(nft),
        }));
    }

    let output = run_puzzle(allocator, puzzle_ptr, solution_ptr)?;
    let conditions =
        Vec::<Condition<NodePtr>>::from_clvm(allocator, output).map_err(DriverError::FromClvm)?;
    let odd_children: Vec<_> = conditions
        .into_iter()
        .filter_map(|c| match c {
            Condition::CreateCoin(cc) if cc.amount % 2 == 1 => Some(cc),
            _ => None,
        })
        .collect();

    if let Some(child) = odd_children.first() {
        let child_coin = Coin::new(spend.coin.coin_id(), child.puzzle_hash, child.amount);
        return Ok(FollowSpendResult::Next(DiscoveredSingleton {
            coin: child_coin,
            inner_puzzle_hash: singleton.inner_puzzle.tree_hash().into(),
            melted: false,
            nft: None,
        }));
    }

    Ok(FollowSpendResult::Melted {
        last_coin: spend.coin,
        inner_puzzle_hash: singleton.inner_puzzle.tree_hash().into(),
        nft: match NftInfo::parse(allocator, puzzle)? {
            Some((info, _)) => Some(ParsedNftState::from_nft_info(allocator, &info)?),
            None => None,
        },
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowSpendResult {
    Next(DiscoveredSingleton),
    Melted {
        last_coin: Coin,
        inner_puzzle_hash: Bytes32,
        nft: Option<ParsedNftState>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_spends_is_incomplete() {
        let mut a = Allocator::new();
        let result = discover_singleton_in_block(
            &mut a,
            &[],
            Bytes32::default(),
            Bytes32::default(),
            Bytes32::default(),
        )
        .unwrap();
        assert_eq!(result, DiscoveryResult::Incomplete);
    }

    #[test]
    fn wrong_puzzle_hash_is_skipped() {
        let mut a = Allocator::new();
        let spend = CoinSpend::new(
            Coin::new(Bytes32::default(), Bytes32::new([1; 32]), 1),
            vec![0xff_u8].into(),
            vec![0xff_u8].into(),
        );
        let result = discover_singleton_in_block(
            &mut a,
            &[spend],
            Bytes32::new([3; 32]),
            Bytes32::new([2; 32]),
            Bytes32::new([4; 32]),
        )
        .unwrap();
        assert_eq!(result, DiscoveryResult::Incomplete);
    }
}
