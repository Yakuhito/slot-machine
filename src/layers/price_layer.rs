use chia::{
    clvm_utils::tree_hash,
    protocol::{Bytes, Bytes32},
    puzzles::standard::{DEFAULT_HIDDEN_PUZZLE, DEFAULT_HIDDEN_PUZZLE_HASH},
};
use chia_wallet_sdk::{Conditions, DriverError, Layer, Puzzle, SpendContext};
use clvm_traits::{clvm_quote, ToClvm};
use clvmr::{Allocator, NodePtr};
use num_bigint::BigInt;

// TODO: ideally, this layer would only be aware of this generation
// so the primitive stores all this data and this puzzle is
// only aware of launcher_id, other_singleton_puzzle_hash, next_block_height, next_price
// this would allow the layer to implement parse_puzzle
// you could even modify it to be a more general delegated state layer - the primitive would
// make it specific to price schedules

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
    ) -> Self {
        Self {
            launcher_id,
            price_schedule,
            generation,
            other_singleton_puzzle_hash,
        }
    }

    pub fn construct_single_generation_puzzle(
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

    pub fn construct_generation_puzzle(
        allocator: &mut Allocator,
        launcher_id: Bytes32,
        price_schedule: Vec<(u32, u64)>,
        generation: u32,
        other_singleton_puzzle_hash: Bytes32,
    ) -> Result<NodePtr, DriverError> {
        if generation >= price_schedule.len() as u32 {
            return DEFAULT_HIDDEN_PUZZLE
                .to_clvm(allocator)
                .map_err(DriverError::ToClvm);
        }

        let mut current_gen = generation as usize;
        let mut next_puzzle_hash: Bytes32 = DEFAULT_HIDDEN_PUZZLE_HASH.into();
        while current_gen > price_schedule.len() {
            let next_puzze = Self::construct_single_generation_puzzle(
                allocator,
                launcher_id,
                price_schedule[current_gen].0,
                price_schedule[current_gen].1,
                next_puzzle_hash,
                other_singleton_puzzle_hash,
            )?;
            next_puzzle_hash = tree_hash(allocator, next_puzze).into();
            current_gen -= 1;
        }

        PriceLayer::construct_single_generation_puzzle(
            allocator,
            launcher_id,
            price_schedule[current_gen].0,
            price_schedule[current_gen].1,
            next_puzzle_hash,
            other_singleton_puzzle_hash,
        )
    }
}

impl Layer for PriceLayer {
    type Solution = ();

    fn parse_puzzle(_: &Allocator, _: Puzzle) -> Result<Option<Self>, DriverError> {
        Ok(None) // Can't infer price schedule from single puzzle
    }

    fn parse_solution(_: &Allocator, _: NodePtr) -> Result<Self::Solution, DriverError> {
        Ok(())
    }

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Self::construct_generation_puzzle(
            &mut ctx.allocator,
            self.launcher_id,
            self.price_schedule.clone(),
            self.generation,
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