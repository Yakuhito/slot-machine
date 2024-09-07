use chia::{
    protocol::{Bytes32, Coin, CoinSpend, Program},
    puzzles::singleton::{
        LauncherSolution, SingletonArgs, SINGLETON_LAUNCHER_PUZZLE, SINGLETON_LAUNCHER_PUZZLE_HASH,
    },
};
use chia_wallet_sdk::{announcement_id, Conditions, DriverError, Launcher, SpendContext};
use clvm_traits::ToClvm;
use clvmr::Allocator;

pub trait LauncherExt {
    fn spend_with_target_amount<T>(
        self,
        ctx: &mut SpendContext,
        singleton_inner_puzzle_hash: Bytes32,
        key_value_list: T,
        target_singleton_amount: u64,
    ) -> Result<(Conditions, Coin), DriverError>
    where
        T: ToClvm<Allocator> + Clone;
}

impl LauncherExt for Launcher {
    fn spend_with_target_amount<T>(
        self,
        ctx: &mut SpendContext,
        singleton_inner_puzzle_hash: Bytes32,
        key_value_list: T,
        target_singleton_amount: u64,
    ) -> Result<(Conditions, Coin), DriverError>
    where
        T: ToClvm<Allocator> + Clone,
    {
        let coin = self.coin();
        let singleton_puzzle_hash =
            SingletonArgs::curry_tree_hash(coin.coin_id(), singleton_inner_puzzle_hash.into())
                .into();

        let solution_ptr = ctx.alloc(&LauncherSolution {
            singleton_puzzle_hash,
            amount: target_singleton_amount,
            key_value_list,
        })?;

        let solution = ctx.serialize(&solution_ptr)?;

        ctx.insert(CoinSpend::new(
            coin,
            Program::from(SINGLETON_LAUNCHER_PUZZLE.to_vec()),
            solution,
        ));

        let singleton_coin = Coin::new(
            coin.coin_id(),
            singleton_puzzle_hash,
            target_singleton_amount,
        );

        Ok((
            Conditions::new()
                .create_coin(
                    SINGLETON_LAUNCHER_PUZZLE_HASH.into(),
                    coin.amount,
                    Vec::new(),
                )
                .assert_coin_announcement(announcement_id(
                    coin.coin_id(),
                    ctx.tree_hash(solution_ptr),
                )),
            singleton_coin,
        ))
    }
}
