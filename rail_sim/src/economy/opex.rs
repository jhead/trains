//! Per-train operating cost each Advance tick (soft-fail).

use bevy_ecs::prelude::*;

use crate::money::Money;
use crate::trains::TrainLocation;

use super::ledger::{MoneyCategory, MoneyLedger};

/// Operating cost per active (unparked) train per tick: $0.10.
pub const TRAIN_OPEX_CENTS: i64 = 10;

/// Debit opex for each unparked train. On insufficient funds, park the train
/// (do not delete) — it stops moving until money recovers.
pub fn apply_train_opex(
    mut money: ResMut<Money>,
    mut ledger: ResMut<MoneyLedger>,
    mut q: Query<&mut TrainLocation>,
) {
    for mut loc in q.iter_mut() {
        if loc.parked {
            // Try to unpark if we can afford opex again.
            if money.can_afford(TRAIN_OPEX_CENTS) {
                if ledger
                    .try_debit(&mut money, MoneyCategory::TrainOpex, TRAIN_OPEX_CENTS)
                    .is_ok()
                {
                    loc.parked = false;
                }
            }
            continue;
        }
        if ledger
            .try_debit(&mut money, MoneyCategory::TrainOpex, TRAIN_OPEX_CENTS)
            .is_err()
        {
            loc.parked = true;
        }
    }
}
