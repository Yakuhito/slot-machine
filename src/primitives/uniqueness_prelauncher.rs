use chia::{clvm_utils::TreeHash, protocol::{Bytes32, Coin}};
use chia_wallet_sdk::{Conditions, DriverError, Launcher, SpendContext};
use hex_literal::hex;


#[derive(Debug, Clone)]
#[must_use]
pub struct UniquenessPrelauncher<V> {
    pub coin: Coin,
    pub value: V,
}

impl<V> UniquenessPrelauncher<V> {
    pub fn from_coin(coin: Coin, value: V) -> Self {
        Self { coin, value }
    }

    pub fn new(parent_coin_id: Bytes32, amount: u64, value: V) -> Self {
        Self::from_coin(
            Coin::new(
                parent_coin_id,
                SINGLETON_LAUNCHER_PUZZLE_HASH.into(),
                amount,
            ),
            value,
        )
    }

    pub fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: SINGLETON_LAUNCHER_PUZZLE.to_vec(),
            args: UniquenessPrelauncherArgs::new(self.coin.coin_id(), self.value),
        }.to_clvm(&mut ctx.allocator)?)
    }

    pub fn spend<T>(
        self,
        ctx: &mut SpendContext,
    ) -> Result<Launcher, DriverError>
    where
        T: ToClvm<Allocator>,
    {
        let singleton_puzzle_hash =
            SingletonArgs::curry_tree_hash(self.coin.coin_id(), singleton_inner_puzzle_hash.into())
                .into();

        let solution_ptr = ctx.alloc(&LauncherSolution {
            singleton_puzzle_hash,
            amount: self.coin.amount,
            key_value_list,
        })?;

        let solution = ctx.serialize(&solution_ptr)?;

        ctx.insert(CoinSpend::new(
            self.coin,
            Program::from(SINGLETON_LAUNCHER_PUZZLE.to_vec()),
            solution,
        ));

        let singleton_coin =
            Coin::new(self.coin.coin_id(), singleton_puzzle_hash, self.coin.amount);

        Ok((
            self.conditions.assert_coin_announcement(announcement_id(
                self.coin.coin_id(),
                ctx.tree_hash(solution_ptr),
            )),
            singleton_coin,
        ))
    }
}

pub const UNIQUENESS_PRELAUNCHER_PUZZLE: [u8; 59] = hex!("ff02ffff01ff04ffff04ff04ffff04ff05ffff01ff01808080ffff04ffff04ff06ffff04ff0bff808080ff808080ffff04ffff01ff333eff018080");

pub const  UNIQUENESS_PRELAUNCHER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    1e5759069429397243b808748e5bd5270ea0891953ea06df9a46b87ce4ade466
    "
));