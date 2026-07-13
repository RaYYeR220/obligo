//! Every number that decides whether a merchant is solvent lives here, and only here.
//!
//! Points are a liability denominated in USDC micro-units. A merchant does not have to hold
//! that liability in full — it holds a *reserve* against points still in customers' hands, and
//! the full amount only once a point has actually been spent somewhere and turned into debt.

use crate::error::ObligoError;
use anchor_lang::prelude::*;

pub const BPS: u128 = 10_000;

/// The USDC micro-value a merchant has promised to honour for `points`.
pub fn face(points: u64, usdc_per_point: u64) -> Result<u64> {
    let product = (points as u128)
        .checked_mul(usdc_per_point as u128)
        .ok_or(ObligoError::Overflow)?;
    u64::try_from(product).map_err(|_| ObligoError::Overflow.into())
}

/// Collateral a merchant must hold: the full value of what it already owes, plus a fractional
/// reserve against the points still at large.
///
/// `reserve_bps < 10_000` is the whole point: it lets a merchant issue points worth more than
/// its collateral, exactly as a bank lends more than it holds.
pub fn required_collateral(
    obligations_out: u64,
    points_outstanding: u64,
    usdc_per_point: u64,
    reserve_bps: u16,
) -> Result<u64> {
    let at_large = face(points_outstanding, usdc_per_point)? as u128;
    let reserve = at_large
        .checked_mul(reserve_bps as u128)
        .ok_or(ObligoError::Overflow)?
        .checked_div(BPS)
        .ok_or(ObligoError::Overflow)?;
    let total = (obligations_out as u128)
        .checked_add(reserve)
        .ok_or(ObligoError::Overflow)?;
    u64::try_from(total).map_err(|_| ObligoError::Overflow.into())
}

/// Collateral as a fraction of what is required, in bps. `10_000` is exactly fully reserved.
/// A merchant that requires nothing is infinitely healthy.
pub fn health_bps(collateral: u64, required: u64) -> u64 {
    if required == 0 {
        return u64::MAX;
    }
    (((collateral as u128) * BPS) / (required as u128)) as u64
}

/// Solvency ignores the reserve entirely: it asks only whether the merchant can pay the debts
/// it has already incurred. Below this line, anyone may liquidate it.
pub fn is_solvent(collateral: u64, obligations_out: u64) -> bool {
    collateral >= obligations_out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserve_is_fractional() {
        // 1000 points at $0.01 = $10.00 face; 30% reserve = $3.00; no debt yet.
        assert_eq!(
            required_collateral(0, 1000, 10_000, 3000).unwrap(),
            3_000_000
        );
    }

    #[test]
    fn redemption_converts_reserve_into_full_debt() {
        // all 1000 points redeemed: outstanding -> 0, obligations_out -> $10.00 face
        assert_eq!(
            required_collateral(10_000_000, 0, 10_000, 3000).unwrap(),
            10_000_000
        );
    }

    #[test]
    fn health_falls_when_points_are_redeemed() {
        let before = health_bps(
            3_000_000,
            required_collateral(0, 1000, 10_000, 3000).unwrap(),
        );
        let after = health_bps(
            3_000_000,
            required_collateral(10_000_000, 0, 10_000, 3000).unwrap(),
        );
        assert_eq!(before, 10_000); // exactly 1.0
        assert!(after < 10_000); // underwater — liquidatable
    }

    #[test]
    fn face_overflow_is_caught_not_wrapped() {
        assert!(face(u64::MAX, 2).is_err());
    }
}
