//! Where a merchant's collateral rests while it is not being spent.
//!
//! A merchant posts USDC to back the points it issues. Between the moment it is posted and the
//! moment a creditor is paid, that USDC sits idle in the vault. A `YieldAdapter` is the seam that
//! decides *where* it sits: as plain USDC in the vault (the default, and what devnet runs), or put
//! to work in a lending market that pays interest ("self-funding loyalty").
//!
//! Two rules are load-bearing and neither is negotiable:
//!
//! - **Solvency is measured on principal, never on principal + yield.** `math.rs` asks only whether
//!   a merchant can cover the debts it has already incurred, and it asks it of the collateral the
//!   merchant *deposited*, not of whatever that collateral has since grown into. Yield is strictly
//!   additive: it can make a merchant wealthier, never — by being counted early — let it issue
//!   points it cannot back. Nothing in this file touches that invariant.
//! - **The default path is a passthrough.** With [`NullAdapter`], `deposit` and `withdraw` do
//!   nothing beyond what the surrounding instruction already did — the USDC lands in the vault and
//!   stays there. So wiring `deposit_collateral` / `withdraw_collateral` through this trait is real
//!   (the instructions genuinely call it) and yet, on devnet, changes not a single byte of state.
//!
//! The [`KaminoAdapter`] — behind `feature = "kamino"`, which devnet never compiles — is the other
//! implementation: it routes the vault's USDC into Kamino's KLend reserve, holds the cTokens the
//! reserve mints back, and redeems them for *more* USDC than it put in. It talks to KLend the same
//! way `hook_cpi.rs` talks to the hook — a hand-built `Instruction` and `invoke_signed`, because
//! there is no Anchor-1.x-compatible CPI crate for KLend and there is not going to be one. It is
//! proven against real mainnet KLend state in the fork test under `kamino-fork/`.

use anchor_lang::prelude::*;

/// The collateral yield seam.
///
/// An adapter takes USDC principal in, may put it to work, and gives (at least) that principal back
/// out. The three methods are all an instruction needs to route collateral through a yield source
/// without knowing which source it is.
pub trait YieldAdapter {
    /// Put `principal_in` USDC to work. The USDC is already in the vault when this is called — the
    /// surrounding instruction transferred it there — so a passthrough adapter has nothing to do.
    fn deposit(&self, principal_in: u64) -> Result<()>;

    /// Make principal available in the vault again, returning the USDC actually freed. For a
    /// passthrough that is exactly `principal_out`; for a yield source it is `principal_out` plus
    /// whatever interest the redeemed position had accrued.
    fn withdraw(&self, principal_out: u64) -> Result<u64>;

    /// Principal plus accrued yield currently held by the position, in USDC micro-units. Solvency
    /// never reads this — it is for reporting the merchant's claimable balance, not for deciding
    /// whether it may issue.
    fn total_assets(&self) -> Result<u64>;
}

/// The `amount` field of an SPL token account: a little-endian `u64` at offset 64 in both the
/// legacy Token and the Token-2022 base layouts.
pub(crate) fn read_token_amount(account: &AccountInfo) -> Result<u64> {
    let data = account.try_borrow_data()?;
    let bytes: [u8; 8] = data
        .get(64..72)
        .and_then(|s| s.try_into().ok())
        .ok_or(ProgramError::InvalidAccountData)?;
    Ok(u64::from_le_bytes(bytes))
}

/// The vault holds USDC and nothing else happens to it. This is the devnet path, and the reason the
/// 102-test core suite keeps passing byte-for-byte with the seam in place: every method here is a
/// no-op over the state the surrounding instruction already set.
pub struct NullAdapter<'a, 'info> {
    /// The merchant's USDC collateral vault.
    pub vault: &'a AccountInfo<'info>,
}

impl YieldAdapter for NullAdapter<'_, '_> {
    fn deposit(&self, _principal_in: u64) -> Result<()> {
        // The USDC is already in the vault. There is nowhere else for it to go.
        Ok(())
    }

    fn withdraw(&self, principal_out: u64) -> Result<u64> {
        // The USDC never left the vault; the caller moves it out. No interest, no surprise.
        Ok(principal_out)
    }

    fn total_assets(&self) -> Result<u64> {
        // Principal and nothing more: whatever the vault holds is all there is.
        read_token_amount(self.vault)
    }
}

/// Build the adapter the collateral instructions route through. On the default build this is the
/// passthrough [`NullAdapter`]; under `feature = "kamino"` it is a [`KaminoAdapter`] assembled from
/// the KLend accounts the caller appended as `remaining`.
///
/// The signature is identical across both builds so the call site in `deposit_collateral` /
/// `withdraw_collateral` carries no `cfg`. `owner` is the vault's authority (the merchant PDA) and
/// `signer_seeds` are its seeds — the passthrough ignores both; the Kamino path signs KLend CPIs
/// with them.
#[cfg(not(feature = "kamino"))]
pub fn vault_adapter<'a, 'info>(
    vault: &'a AccountInfo<'info>,
    _remaining: &'a [AccountInfo<'info>],
    _owner: &'a AccountInfo<'info>,
    _signer_seeds: &'a [&'a [u8]],
) -> Result<NullAdapter<'a, 'info>> {
    Ok(NullAdapter { vault })
}

#[cfg(feature = "kamino")]
pub fn vault_adapter<'a, 'info>(
    vault: &'a AccountInfo<'info>,
    remaining: &'a [AccountInfo<'info>],
    owner: &'a AccountInfo<'info>,
    signer_seeds: &'a [&'a [u8]],
) -> Result<KaminoAdapter<'a, 'info>> {
    KaminoAdapter::from_remaining(owner, vault, remaining, signer_seeds)
}

#[cfg(feature = "kamino")]
pub use kamino::KaminoAdapter;

#[cfg(feature = "kamino")]
mod kamino {
    //! Hand-rolled CPI into Kamino's KLend, the same shape as `hook_cpi.rs`: an `Instruction`
    //! built by hand and driven with `invoke_signed`, because KLend is on Anchor 0.29 with
    //! `publish = false` and a BUSL-1.1 licence — there is no crate to depend on, and none is
    //! wanted in this tree. None of this is compiled on devnet.
    //!
    //! The instruction discriminators and account orders below are pinned to KLend's on-chain
    //! program (`KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`) and exercised against real mainnet
    //! reserve state in the fork test. A getter reads the reserve's collateral exchange rate to
    //! value the position; the load-bearing proof, though, is the realized USDC a redemption
    //! returns, measured as a balance delta on the vault.

    use anchor_lang::prelude::*;
    use anchor_lang::solana_program::{instruction::Instruction, program::invoke_signed};

    use super::{read_token_amount, YieldAdapter};
    use crate::error::ObligoError;

    /// KLend, one program id on devnet and mainnet alike.
    pub const KLEND_PROGRAM_ID: Pubkey = pubkey!("KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD");

    /// `sha256("global:refresh_reserve")[..8]`
    pub const REFRESH_RESERVE: [u8; 8] = [2, 218, 138, 235, 79, 201, 25, 102];
    /// `sha256("global:deposit_reserve_liquidity")[..8]` — arg: `u64 liquidity_amount`.
    pub const DEPOSIT_RESERVE_LIQUIDITY: [u8; 8] = [169, 201, 30, 126, 6, 205, 102, 68];
    /// `sha256("global:redeem_reserve_collateral")[..8]` — arg: `u64 collateral_amount`.
    pub const REDEEM_RESERVE_COLLATERAL: [u8; 8] = [234, 117, 181, 125, 185, 142, 220, 29];

    /// The KLend accounts a caller appends after the core's own, in this exact order. The vault's
    /// USDC account (the "user liquidity") and its authority are passed separately; everything else
    /// KLend needs to move liquidity in and cTokens out lives here.
    ///
    /// | idx | account                              |
    /// |-----|--------------------------------------|
    /// | 0   | KLend program                        |
    /// | 1   | reserve (mut)                        |
    /// | 2   | lending market                       |
    /// | 3   | lending market authority (PDA)       |
    /// | 4   | reserve liquidity mint (USDC)        |
    /// | 5   | reserve liquidity supply (mut)       |
    /// | 6   | reserve collateral mint (mut)        |
    /// | 7   | vault collateral / cToken acct (mut) |
    /// | 8   | collateral token program             |
    /// | 9   | liquidity token program              |
    /// | 10  | price oracle (for refresh_reserve)   |
    pub struct KaminoAdapter<'a, 'info> {
        klend_program: &'a AccountInfo<'info>,
        owner: &'a AccountInfo<'info>,
        reserve: &'a AccountInfo<'info>,
        lending_market: &'a AccountInfo<'info>,
        lending_market_authority: &'a AccountInfo<'info>,
        reserve_liquidity_mint: &'a AccountInfo<'info>,
        reserve_liquidity_supply: &'a AccountInfo<'info>,
        reserve_collateral_mint: &'a AccountInfo<'info>,
        user_liquidity: &'a AccountInfo<'info>,
        user_collateral: &'a AccountInfo<'info>,
        collateral_token_program: &'a AccountInfo<'info>,
        liquidity_token_program: &'a AccountInfo<'info>,
        oracle: &'a AccountInfo<'info>,
        signer_seeds: &'a [&'a [u8]],
    }

    impl<'a, 'info> KaminoAdapter<'a, 'info> {
        pub fn from_remaining(
            owner: &'a AccountInfo<'info>,
            user_liquidity: &'a AccountInfo<'info>,
            remaining: &'a [AccountInfo<'info>],
            signer_seeds: &'a [&'a [u8]],
        ) -> Result<Self> {
            let at = |i: usize| -> Result<&'a AccountInfo<'info>> {
                remaining
                    .get(i)
                    .ok_or(ObligoError::InvalidYieldAccounts.into())
            };
            Ok(Self {
                klend_program: at(0)?,
                reserve: at(1)?,
                lending_market: at(2)?,
                lending_market_authority: at(3)?,
                reserve_liquidity_mint: at(4)?,
                reserve_liquidity_supply: at(5)?,
                reserve_collateral_mint: at(6)?,
                user_collateral: at(7)?,
                collateral_token_program: at(8)?,
                liquidity_token_program: at(9)?,
                oracle: at(10)?,
                owner,
                user_liquidity,
                signer_seeds,
            })
        }

        /// KLend will not price a reserve whose oracle it has not read this slot, so every deposit
        /// and redeem is preceded by a `refresh_reserve`. The absent oracle slots are filled with
        /// the KLend program id, the sentinel its optional-account decoding expects.
        fn refresh_reserve(&self) -> Result<()> {
            let ix = Instruction {
                program_id: KLEND_PROGRAM_ID,
                accounts: vec![
                    AccountMeta::new(self.reserve.key(), false),
                    AccountMeta::new_readonly(self.lending_market.key(), false),
                    AccountMeta::new_readonly(self.oracle.key(), false),
                    AccountMeta::new_readonly(KLEND_PROGRAM_ID, false),
                    AccountMeta::new_readonly(KLEND_PROGRAM_ID, false),
                    AccountMeta::new_readonly(KLEND_PROGRAM_ID, false),
                ],
                data: REFRESH_RESERVE.to_vec(),
            };
            invoke_signed(
                &ix,
                &[
                    self.reserve.clone(),
                    self.lending_market.clone(),
                    self.oracle.clone(),
                    self.klend_program.clone(),
                ],
                self.signer_seeds_wrapped().as_slice(),
            )
            .map_err(Into::into)
        }

        fn signer_seeds_wrapped(&self) -> Vec<&[&[u8]]> {
            vec![self.signer_seeds]
        }

        fn deposit_reserve_liquidity(&self, liquidity_amount: u64) -> Result<()> {
            let mut data = Vec::with_capacity(16);
            data.extend_from_slice(&DEPOSIT_RESERVE_LIQUIDITY);
            data.extend_from_slice(&liquidity_amount.to_le_bytes());

            let ix = Instruction {
                program_id: KLEND_PROGRAM_ID,
                accounts: vec![
                    AccountMeta::new_readonly(self.owner.key(), true),
                    AccountMeta::new(self.reserve.key(), false),
                    AccountMeta::new_readonly(self.lending_market.key(), false),
                    AccountMeta::new_readonly(self.lending_market_authority.key(), false),
                    AccountMeta::new_readonly(self.reserve_liquidity_mint.key(), false),
                    AccountMeta::new(self.reserve_liquidity_supply.key(), false),
                    AccountMeta::new(self.reserve_collateral_mint.key(), false),
                    AccountMeta::new(self.user_liquidity.key(), false),
                    AccountMeta::new(self.user_collateral.key(), false),
                    AccountMeta::new_readonly(self.collateral_token_program.key(), false),
                    AccountMeta::new_readonly(self.liquidity_token_program.key(), false),
                ],
                data,
            };
            invoke_signed(
                &ix,
                &self.deposit_infos(),
                self.signer_seeds_wrapped().as_slice(),
            )
            .map_err(Into::into)
        }

        fn redeem_reserve_collateral(&self, collateral_amount: u64) -> Result<()> {
            let mut data = Vec::with_capacity(16);
            data.extend_from_slice(&REDEEM_RESERVE_COLLATERAL);
            data.extend_from_slice(&collateral_amount.to_le_bytes());

            let ix = Instruction {
                program_id: KLEND_PROGRAM_ID,
                accounts: vec![
                    AccountMeta::new_readonly(self.owner.key(), true),
                    AccountMeta::new(self.reserve.key(), false),
                    AccountMeta::new_readonly(self.lending_market.key(), false),
                    AccountMeta::new_readonly(self.lending_market_authority.key(), false),
                    AccountMeta::new_readonly(self.reserve_liquidity_mint.key(), false),
                    AccountMeta::new(self.reserve_collateral_mint.key(), false),
                    AccountMeta::new(self.reserve_liquidity_supply.key(), false),
                    AccountMeta::new(self.user_collateral.key(), false),
                    AccountMeta::new(self.user_liquidity.key(), false),
                    AccountMeta::new_readonly(self.collateral_token_program.key(), false),
                    AccountMeta::new_readonly(self.liquidity_token_program.key(), false),
                ],
                data,
            };
            invoke_signed(
                &ix,
                &self.redeem_infos(),
                self.signer_seeds_wrapped().as_slice(),
            )
            .map_err(Into::into)
        }

        fn deposit_infos(&self) -> Vec<AccountInfo<'info>> {
            vec![
                self.owner.clone(),
                self.reserve.clone(),
                self.lending_market.clone(),
                self.lending_market_authority.clone(),
                self.reserve_liquidity_mint.clone(),
                self.reserve_liquidity_supply.clone(),
                self.reserve_collateral_mint.clone(),
                self.user_liquidity.clone(),
                self.user_collateral.clone(),
                self.collateral_token_program.clone(),
                self.liquidity_token_program.clone(),
            ]
        }

        fn redeem_infos(&self) -> Vec<AccountInfo<'info>> {
            vec![
                self.owner.clone(),
                self.reserve.clone(),
                self.lending_market.clone(),
                self.lending_market_authority.clone(),
                self.reserve_liquidity_mint.clone(),
                self.reserve_collateral_mint.clone(),
                self.reserve_liquidity_supply.clone(),
                self.user_collateral.clone(),
                self.user_liquidity.clone(),
                self.collateral_token_program.clone(),
                self.liquidity_token_program.clone(),
            ]
        }
    }

    impl YieldAdapter for KaminoAdapter<'_, '_> {
        fn deposit(&self, principal_in: u64) -> Result<()> {
            self.refresh_reserve()?;
            self.deposit_reserve_liquidity(principal_in)
        }

        /// Redeem the vault's whole cToken position back to USDC and report how much USDC that
        /// realized. `principal_out` is the principal the caller expects to recover; a full-position
        /// withdrawal — the merchant pulling its yield deposit — recovers principal plus the
        /// interest the cTokens accrued, so the returned figure is `>= principal_out`, and the fork
        /// test asserts exactly that. A partial withdrawal would instead redeem
        /// `principal_out * collateral_supply / total_liquidity` cTokens; that refinement is left to
        /// the mainnet wiring and documented in `docs/YIELD.md`, not stubbed here.
        fn withdraw(&self, principal_out: u64) -> Result<u64> {
            let before = read_token_amount(self.user_liquidity)?;
            let collateral = read_token_amount(self.user_collateral)?;
            self.refresh_reserve()?;
            self.redeem_reserve_collateral(collateral)?;
            let after = read_token_amount(self.user_liquidity)?;
            let realized = after.checked_sub(before).ok_or(ObligoError::Overflow)?;
            require!(realized >= principal_out, ObligoError::YieldShortfall);
            Ok(realized)
        }

        /// Value the vault's cTokens at the reserve's current collateral exchange rate, plus any
        /// idle USDC still in the vault. The rate is read from the reserve account; if the layout
        /// cannot be parsed this returns the idle balance alone rather than inventing a number.
        fn total_assets(&self) -> Result<u64> {
            let idle = read_token_amount(self.user_liquidity)?;
            let collateral = read_token_amount(self.user_collateral)?;
            let valued = collateral_to_liquidity(self.reserve, collateral).unwrap_or(0);
            idle.checked_add(valued).ok_or(ObligoError::Overflow.into())
        }
    }

    /// A cToken is a claim on `total_liquidity / mint_total_supply` of the reserve's USDC, and that
    /// ratio only rises as interest accrues — which is the whole mechanism. KLend keeps
    /// `total_available_amount` as a plain `u64` and `borrowed_amount_sf` as a scaled fraction (a
    /// `u128` with [`SF_BITS`] fractional bits); their sum is the total liquidity the cToken supply
    /// is a claim on. This reads the three anchoring fields at their pinned offsets. It is
    /// best-effort *reporting*, never a solvency input, so any parse miss degrades to `None` and the
    /// position is valued at zero rather than at a fabricated number.
    fn collateral_to_liquidity(reserve: &AccountInfo, collateral: u64) -> Option<u64> {
        if collateral == 0 {
            return Some(0);
        }
        let data = reserve.try_borrow_data().ok()?;
        let read_u64 = |off: usize| -> Option<u64> {
            Some(u64::from_le_bytes(data.get(off..off + 8)?.try_into().ok()?))
        };
        let available = read_u64(RESERVE_AVAILABLE_OFF)?;
        let borrowed_sf = u128::from_le_bytes(
            data.get(RESERVE_BORROWED_SF_OFF..RESERVE_BORROWED_SF_OFF + 16)?
                .try_into()
                .ok()?,
        );
        let supply = read_u64(RESERVE_COLL_SUPPLY_OFF)?;
        if supply == 0 {
            return Some(0);
        }
        let borrowed = borrowed_sf >> SF_BITS;
        let total_liquidity = (available as u128).checked_add(borrowed)?;
        let value = (collateral as u128)
            .checked_mul(total_liquidity)?
            .checked_div(supply as u128)?;
        u64::try_from(value).ok()
    }

    // Offsets into the raw mainnet KLend `Reserve` account, 8-byte Anchor discriminator included.
    // Layout: disc(8) + version:u64(8) + last_update:LastUpdate(16) + lending_market/farm_collateral
    // /farm_debt: 3×Pubkey(96) = 128, then ReserveLiquidity, in which total_available_amount sits
    // after mint/supply/fee vaults (3×Pubkey = 96) and borrowed_amount_sf follows it. collateral
    // .mint_total_supply sits past the whole liquidity block and its 150×u64 padding. The fork test
    // cross-checks the exchange rate these produce against a real redemption, so a layout change
    // surfaces there rather than silently.
    const RESERVE_AVAILABLE_OFF: usize = 128 + 96; // 224: liquidity.total_available_amount (u64)
    const RESERVE_BORROWED_SF_OFF: usize = RESERVE_AVAILABLE_OFF + 8; // 232: liquidity.borrowed_amount_sf (u128)
    const RESERVE_COLL_SUPPLY_OFF: usize = 2592; // collateral.mint_total_supply (u64)

    /// Fractional bits in a KLend scaled-fraction (`_sf`) field. Pinned; validated in the fork test.
    const SF_BITS: u32 = 60;
}
