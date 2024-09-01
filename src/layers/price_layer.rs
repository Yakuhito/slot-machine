use chia::{
    protocol::{Bytes, Bytes32},
    puzzles::standard::{DEFAULT_HIDDEN_PUZZLE, DEFAULT_HIDDEN_PUZZLE_HASH},
};
use chia_wallet_sdk::{Conditions, DriverError, Layer, Puzzle, SpendContext};
use clvm_traits::{clvm_quote, ToClvm};
use clvmr::{Allocator, NodePtr};
use num_bigint::BigInt;
use once_cell::sync::Lazy;

// https://docs.chia.net/block-rewards/#rewards-schedule
pub static BLOCK_REWARD_SCHEDULE: Lazy<Vec<(u32, u64)>> = Lazy::new(|| {
    vec![
        (10_091_520, 500_000_000_000),
        (15_137_280, 250_000_000_000),
        (20_183_040, 125_000_000_000),
    ]
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceLayer {
    pub launcher_id: Bytes32,
    pub price_schedule: Vec<(u32, u64)>,
    pub generation: u32, // 0 -> waiting for block price_scedule[0].0
    // when generation = price_schedule.len(), the puzzle is DEFAULT_HIDDEN_PUZZLE
    pub other_singleton_puzzle_hash: Bytes32,
}

impl PriceLayer {
    pub fn new(
        launcher_id: Bytes32,
        price_schedule: Vec<(u32, u64)>,
        generation: u32,
        other_singleton_puzzle_hash: Bytes32,
    ) -> Option<Self> {
        if generation > price_schedule.len() as u32 {
            return None;
        }

        Some(Self {
            launcher_id,
            price_schedule,
            generation,
            other_singleton_puzzle_hash,
        })
    }

    pub fn construct_generation_puzzle(
        allocator: &mut Allocator,
        my_launcher_id: Bytes32,
        next_block_height: u32,
        next_price: u64,
        next_puzzle_hash: Bytes32,
        other_singleton_puzzle_hash: Bytes32,
    ) -> Result<NodePtr, DriverError> {
        let next_price: BigInt = next_price.into();
        let next_price_bytes: Vec<u8> = next_price.to_signed_bytes_be();
        let mut next_price = next_price_bytes.as_slice();

        // make number minimal by removing leading zeros
        while (!next_price.is_empty()) && (next_price[0] == 0) {
            if next_price.len() > 1 && (next_price[1] & 0x80 == 0x80) {
                break;
            }
            next_price = &next_price[1..];
        }

        let other_singleton_puzzle_hash = other_singleton_puzzle_hash.to_clvm(allocator)?;

        clvm_quote!(Conditions::new()
            .assert_height_absolute(next_block_height)
            .create_coin(next_puzzle_hash, 1, vec![my_launcher_id.into()])
            .send_message(
                0b010010,
                Bytes::from(next_price),
                vec![other_singleton_puzzle_hash]
            ))
        .to_clvm(allocator)
        .map_err(DriverError::ToClvm)
    }
}

impl Layer for PriceLayer {
    type Solution = ();

    fn parse_puzzle(_: &Allocator, _: Puzzle) -> Result<Option<Self>, DriverError> {
        todo!()
    }

    fn parse_solution(_: &Allocator, _: NodePtr) -> Result<Self::Solution, DriverError> {
        Ok(())
    }

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        if self.generation >= self.price_schedule.len() as u32 {
            return ctx.alloc(&DEFAULT_HIDDEN_PUZZLE);
        }

        let mut current_gen = self.generation as usize;
        let mut next_puzzle_hash: Bytes32 = DEFAULT_HIDDEN_PUZZLE_HASH.into();
        while current_gen > self.price_schedule.len() {
            let next_puzze = PriceLayer::construct_generation_puzzle(
                &mut ctx.allocator,
                self.launcher_id,
                self.price_schedule[current_gen].0,
                self.price_schedule[current_gen].1,
                next_puzzle_hash,
                self.other_singleton_puzzle_hash,
            )?;
            next_puzzle_hash = ctx.tree_hash(next_puzze).into();
            current_gen -= 1;
        }

        PriceLayer::construct_generation_puzzle(
            &mut ctx.allocator,
            self.launcher_id,
            self.price_schedule[current_gen].0,
            self.price_schedule[current_gen].1,
            next_puzzle_hash,
            self.other_singleton_puzzle_hash,
        )
    }

    fn construct_solution(
        &self,
        _: &mut SpendContext,
        _: Self::Solution,
    ) -> Result<NodePtr, DriverError> {
        Ok(NodePtr::NIL)
    }
}
