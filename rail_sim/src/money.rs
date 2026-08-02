//! Player treasury — cents as the atomic unit.
//!
//! Paid actions (track, trains, …) should call [`Money::try_debit`] and abort
//! the action when it returns [`InsufficientFunds`]. Income uses [`Money::credit`].
//! Balance is never forced negative by these helpers (soft-fail / block builds).

use bevy_ecs::prelude::Resource;

/// Sandbox starting cash: $10,000.00 — enough for a short starter line plus a train.
pub const STARTING_CASH_CENTS: i64 = 1_000_000;

/// Player money balance in integer cents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Resource)]
pub struct Money {
    cents: i64,
}

/// Returned when a debit would drive the balance below zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsufficientFunds {
    pub requested: i64,
    pub available: i64,
}

impl Money {
    /// Create a balance with the given amount in cents.
    pub fn new(cents: i64) -> Self {
        Self { cents }
    }

    /// Sandbox default starting cash.
    pub fn sandbox_starting() -> Self {
        Self::new(STARTING_CASH_CENTS)
    }

    /// Current balance in cents.
    pub fn cents(&self) -> i64 {
        self.cents
    }

    /// `true` when `amount` cents can be debited without going negative.
    pub fn can_afford(&self, amount: i64) -> bool {
        amount <= 0 || self.cents >= amount
    }

    /// Add income / refunds. Negative `amount` is ignored (use [`try_debit`]).
    pub fn credit(&mut self, amount: i64) {
        if amount > 0 {
            self.cents = self.cents.saturating_add(amount);
        }
    }

    /// Spend `amount` cents. Fails without mutating when funds are insufficient.
    ///
    /// A non-positive `amount` succeeds as a no-op.
    pub fn try_debit(&mut self, amount: i64) -> Result<(), InsufficientFunds> {
        if amount <= 0 {
            return Ok(());
        }
        if self.cents < amount {
            return Err(InsufficientFunds {
                requested: amount,
                available: self.cents,
            });
        }
        self.cents -= amount;
        Ok(())
    }
}

impl Default for Money {
    fn default() -> Self {
        Self::sandbox_starting()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debit_refuses_when_insufficient() {
        let mut money = Money::new(100);
        assert!(money.can_afford(100));
        assert!(!money.can_afford(101));

        assert_eq!(money.try_debit(60), Ok(()));
        assert_eq!(money.cents(), 40);

        assert_eq!(
            money.try_debit(50),
            Err(InsufficientFunds {
                requested: 50,
                available: 40,
            })
        );
        assert_eq!(money.cents(), 40, "failed debit must not mutate balance");
    }

    #[test]
    fn credit_increases_balance() {
        let mut money = Money::new(10);
        money.credit(25);
        assert_eq!(money.cents(), 35);
        money.credit(-5);
        assert_eq!(money.cents(), 35);
    }
}
