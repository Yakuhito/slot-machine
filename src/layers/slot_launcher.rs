use chia::protocol::{Bytes, Bytes32};
use chia_wallet_sdk::{Condition, DriverError, Layer, Puzzle, Spend, SpendContext};
use clvm_traits::{clvm_quote, match_quote, FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};

use crate::Slot;

/// The Slot Launcher [`Layer`] allows singletons to launch initial slots before
/// the main logic singleton is created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotLauncherLayer {
    pub launcher_id: Bytes32,
    pub slot_value_hashes: Vec<Bytes32>,
    pub next_puzzle_hash: Bytes32,
}

impl SlotLauncherLayer {
    pub fn new(
        launcher_id: Bytes32,
        slot_value_hashes: Vec<Bytes32>,
        next_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            launcher_id,
            slot_value_hashes,
            next_puzzle_hash,
        }
    }
}

pub fn get_hint(memos: &[Bytes]) -> Option<Bytes32> {
    let hint = memos.first()?;

    let Ok(hint) = hint.try_into() else {
        return None;
    };

    Some(hint)
}

impl Layer for SlotLauncherLayer {
    type Solution = ();

    fn parse_puzzle(allocator: &Allocator, puzzle: Puzzle) -> Result<Option<Self>, DriverError> {
        let Some(puzzle) = puzzle.as_raw() else {
            return Ok(None);
        };

        let (_q, conditions) =
            <match_quote!(Vec<Condition<NodePtr>>)>::from_clvm(allocator, puzzle.ptr)?;

        let Some((next_puzzle_hash, launcher_id)) = conditions.iter().find_map(|cond| {
            let Condition::CreateCoin(create_coin) = cond else {
                return None;
            };

            if create_coin.amount % 2 == 1 {
                let launcher_id = get_hint(&create_coin.memos)?;

                return Some((create_coin.puzzle_hash, launcher_id));
            }

            None
        }) else {
            return Ok(None);
        };

        let slot_value_hashes: Vec<Bytes32> = conditions
            .into_iter()
            .filter_map(|condition| {
                let Condition::CreateCoin(create_coin) = condition else {
                    return None;
                };

                if create_coin.amount != 0 {
                    return None;
                }

                let hint = get_hint(&create_coin.memos)?;

                if Slot::puzzle_hash(launcher_id, hint) != create_coin.puzzle_hash.into() {
                    return None;
                }

                Some(hint)
            })
            .collect();

        if slot_value_hashes.is_empty() {
            return Ok(None);
        }

        Ok(Some(Self {
            launcher_id,
            slot_value_hashes,
            next_puzzle_hash,
        }))
    }

    fn parse_solution(_: &Allocator, _: NodePtr) -> Result<Self::Solution, DriverError> {
        Ok(())
    }

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        let conditions: Vec<Condition<()>> = self
            .slot_value_hashes
            .iter()
            .map(|value_hash| {
                Condition::create_coin(
                    Slot::puzzle_hash(self.launcher_id, *value_hash).into(),
                    0,
                    vec![(*value_hash).into()],
                )
            })
            .chain(vec![Condition::create_coin(
                self.next_puzzle_hash,
                1,
                vec![self.launcher_id.into()],
            )])
            .collect();

        Ok(clvm_quote!(conditions).to_clvm(&mut ctx.allocator)?)
    }

    fn construct_solution(
        &self,
        _: &mut SpendContext,
        (): Self::Solution,
    ) -> Result<NodePtr, DriverError> {
        Ok(NodePtr::NIL)
    }
}

impl SlotLauncherLayer {
    pub fn spend(self, ctx: &mut SpendContext) -> Result<Spend, DriverError> {
        let puzzle = self.construct_puzzle(ctx)?;
        let solution = self.construct_solution(ctx, ())?;

        Ok(Spend { puzzle, solution })
    }
}
