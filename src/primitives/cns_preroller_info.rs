use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_wallet_sdk::{
    Condition, Conditions, DriverError, Layer, Puzzle, SingletonLayer, SpendContext,
};
use clvmr::Allocator;

use crate::ConditionsLayer;

use super::{
    get_hint, CnsSlotValue, Slot, SlotInfo, SlotProof, SLOT32_MAX_VALUE, SLOT32_MIN_VALUE,
};

pub type CnsPrerollerLayers = SingletonLayer<ConditionsLayer>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameToLaunch {
    pub name_hash: Bytes32,
    pub full_slot: Option<CnsSlotValue>,
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CnsPrerollerInfo {
    pub launcher_id: Bytes32,
    pub to_launch: Vec<NameToLaunch>,
    pub next_puzzle_hash: Bytes32,
}

impl CnsPrerollerInfo {
    pub fn new(
        launcher_id: Bytes32,
        to_launch: Vec<CnsSlotValue>,
        next_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            launcher_id,
            to_launch: to_launch
                .into_iter()
                .map(|v| NameToLaunch {
                    name_hash: v.name_hash,
                    full_slot: Some(v),
                })
                .collect(),
            next_puzzle_hash,
        }
    }

    pub fn parse(allocator: &Allocator, puzzle: Puzzle) -> Result<Option<Self>, DriverError> {
        let Some(layers) = CnsPrerollerLayers::parse_puzzle(allocator, puzzle)? else {
            return Ok(None);
        };

        Self::from_layers(layers)
    }

    pub fn from_layers(layers: CnsPrerollerLayers) -> Result<Option<Self>, DriverError> {
        let Some(Condition::CreateCoin(recreate_condition)) =
            layers.inner_puzzle.conditions.as_ref().iter().find(|c| {
                let Condition::CreateCoin(cc) = c else {
                    return false;
                };

                cc.amount % 2 == 1
            })
        else {
            return Ok(None);
        };

        let Some(launcher_id) = get_hint(&recreate_condition.memos) else {
            return Ok(None);
        };

        let next_puzzle_hash = recreate_condition.puzzle_hash;
        let to_launch = layers
            .inner_puzzle
            .conditions
            .into_iter()
            .filter_map(|cond| {
                let Condition::CreateCoin(create_coin) = cond else {
                    return None;
                };

                if create_coin.amount != 0 {
                    return None;
                }

                // we get the name_hash from slot launches
                let name_hash = get_hint(&create_coin.memos)?;

                Some(NameToLaunch {
                    name_hash,
                    full_slot: None,
                })
            })
            .collect();

        Ok(Some(Self {
            launcher_id,
            to_launch,
            next_puzzle_hash,
        }))
    }

    pub fn get_slots(
        to_launch: Vec<NameToLaunch>,
        my_launcher_id: Bytes32,
        slot_proof: SlotProof,
    ) -> Result<Vec<Slot<CnsSlotValue>>, DriverError> {
        let mut slots = Vec::with_capacity(to_launch.len());

        for add_name in to_launch {
            let Some(value) = add_name.full_slot else {
                return Err(DriverError::Custom(
                    "Missing name full launch info (required to build slot)".to_string(),
                ));
            };

            // slot
            let slot = Slot::new(slot_proof, SlotInfo::from_value(my_launcher_id, value));

            let min_value = Bytes32::new(SLOT32_MIN_VALUE);
            if value.neighbors.left_value == min_value {
                let left_slot_value = CnsSlotValue::edge(min_value, min_value, value.name_hash);
                let left_slot = Slot::new(
                    slot_proof,
                    SlotInfo::from_value(my_launcher_id, left_slot_value),
                );
                slots.push(left_slot);
            }

            slots.push(slot);

            let max_value = Bytes32::new(SLOT32_MAX_VALUE);
            if value.neighbors.right_value == max_value {
                let right_slot_value = CnsSlotValue::edge(max_value, value.name_hash, max_value);
                let right_slot = Slot::new(
                    slot_proof,
                    SlotInfo::from_value(my_launcher_id, right_slot_value),
                );
                slots.push(right_slot);
            }
        }

        Ok(slots)
    }

    pub fn into_layers(self) -> Result<CnsPrerollerLayers, DriverError> {
        let mut base_conditions =
            Conditions::new().create_coin(self.next_puzzle_hash, 1, vec![self.launcher_id.into()]);

        for add_name in self.to_launch {
            let Some(slot_value) = add_name.full_slot else {
                return Err(DriverError::Custom(
                    "Missing name full launch info (required to build slot)".to_string(),
                ));
            };

            // create slot
            let slot_value_hash: Bytes32 = slot_value.tree_hash().into();

            base_conditions = base_conditions.create_coin(
                Slot::<CnsSlotValue>::puzzle_hash(&SlotInfo::<CnsSlotValue>::new(
                    self.launcher_id,
                    slot_value_hash,
                ))
                .into(),
                0,
                vec![slot_value.name_hash.into()],
            );

            let min_value = Bytes32::new(SLOT32_MIN_VALUE);
            if slot_value.neighbors.left_value == min_value {
                // also launch min value slot
                base_conditions = base_conditions.create_coin(
                    Slot::<CnsSlotValue>::puzzle_hash(&SlotInfo::<CnsSlotValue>::from_value(
                        self.launcher_id,
                        CnsSlotValue::edge(min_value, min_value, slot_value.name_hash),
                    ))
                    .into(),
                    0,
                    vec![min_value.into()],
                );
            }

            let max_value = Bytes32::new(SLOT32_MAX_VALUE);
            if slot_value.neighbors.right_value == max_value {
                // also launch max value slot
                base_conditions = base_conditions.create_coin(
                    Slot::<CnsSlotValue>::puzzle_hash(&SlotInfo::<CnsSlotValue>::from_value(
                        self.launcher_id,
                        CnsSlotValue::edge(max_value, slot_value.name_hash, max_value),
                    ))
                    .into(),
                    0,
                    vec![max_value.into()],
                );
            }
        }

        Ok(SingletonLayer::new(
            self.launcher_id,
            ConditionsLayer::new(base_conditions),
        ))
    }

    pub fn inner_puzzle_hash(self, ctx: &mut SpendContext) -> Result<TreeHash, DriverError> {
        let layers = self.into_layers()?;
        let inner_puzzle = layers.inner_puzzle.construct_puzzle(ctx)?;

        Ok(ctx.tree_hash(inner_puzzle))
    }
}
