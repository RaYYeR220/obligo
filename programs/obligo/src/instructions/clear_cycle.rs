use anchor_lang::prelude::*;

use crate::constants::{MAX_CYCLE_LEN, MIN_CYCLE_LEN, OBLIGATION_SEED};
use crate::error::ObligoError;
use crate::events::CycleCleared;
use crate::state::{Merchant, Obligation};

/// The ring itself arrives in `remaining_accounts`, because its size is not known until the caller
/// has found it. For a cycle `m0 -> m1 -> ... -> m(k-1) -> m0`:
///
/// ```text
/// [0  .. k )  merchants m0 .. m(k-1)                          (writable)
/// [k .. 2k )  edges     m0->m1, m1->m2, ..., m(k-1)->m0       (writable)
/// ```
///
/// Nothing in this list is trusted. Every account in it is re-derived from the two merchants it
/// claims to sit between, and the handler is mostly the proof.
#[derive(Accounts)]
pub struct ClearCycle<'info> {
    /// Anybody. Clearing a cycle takes nothing from anyone and hands every merchant in the ring a
    /// smaller balance sheet, so there is nobody whose permission it could sensibly require —
    /// and, more to the point, nobody who could be bribed to withhold it.
    pub cranker: Signer<'info>,
}

/// Cancel a ring of debt. Zero USDC moves. This is the instruction the protocol exists for.
///
/// Bilateral netting only helps two merchants who happen to owe each other. Real acceptance
/// networks do not look like that — the debt runs in rings. Aurora owes Belmont, Belmont owes
/// Cordoba, Cordoba owes Aurora. No pair in that ring owes each other both ways, so `settle` has
/// nothing to bite on, and all three are posting collateral against a gross number. But the ring as
/// a whole owes nothing to anybody: it is a closed loop of promises, and the smallest edge in it is
/// a debt that exists only because everyone is looking at their own books instead of the graph.
///
/// So walk the ring and take the smallest edge off every edge in it. Everyone owes less, everyone
/// is owed less, and no money is required to make it true, because none was ever really owed. This
/// is what "collateral scales with net exposure, not gross" means in practice, and it is the one
/// thing a clearing house can do that a payment network cannot.
///
/// **The verification below is the entire security of the mechanism.** The instruction hands an
/// arbitrary stranger the power to reach into sixteen accounts and *reduce what people owe*. The
/// only thing between that and a merchant simply deleting its own debts is the proof that the ring
/// it presented is real. So a forged ring is not rejected by policy; it is made unrepresentable:
///
/// 1. The ring is `3..=8` merchants long, and the account list is exactly twice that.
/// 2. The merchants are all distinct, and all of them are ours.
/// 3. Every edge is a real `Obligation` this program owns, whose stored `debtor` and `creditor` are
///    the two merchants either side of it in the ring, **and which re-derives to its own address**
///    from `[b"obligation", debtor, creditor, bump]`. An `Obligation` account can only ever be
///    created by this program, at the canonical PDA for its pair, so an account that satisfies all
///    of that *is* the edge between those two merchants. There is no second account that could be.
/// 4. The smallest edge is greater than zero, or there is nothing here to clear.
///
/// Drop check 3 and the attack is immediate and total: present the ring `[A, B, C]`, hand over a
/// genuine, program-owned obligation account belonging to some entirely unrelated pair as the edge
/// that closes it, and A's debt is written down against a stranger's.
pub(crate) fn handler<'info>(
    ctx: Context<'info, ClearCycle<'info>>,
    cycle_len: u8,
) -> Result<()> {
    let k = cycle_len as usize;

    // Two is a bilateral debt, and `settle` handles it — in cash. Letting a "cycle" of two through
    // here would let a caller cancel the same debt in the ledger and skip the money entirely.
    require!(
        (MIN_CYCLE_LEN..=MAX_CYCLE_LEN).contains(&cycle_len),
        ObligoError::InvalidCycle
    );
    require!(
        ctx.remaining_accounts.len() == 2 * k,
        ObligoError::InvalidCycle
    );

    let (merchant_infos, edge_infos) = ctx.remaining_accounts.split_at(k);

    // A repeated merchant is the cheapest forgery on the menu: it lets part of the "ring" be a
    // merchant clearing debt against itself. k is at most 8, so this is 28 comparisons.
    for i in 0..k {
        for j in (i + 1)..k {
            require_keys_neq!(
                merchant_infos[i].key(),
                merchant_infos[j].key(),
                ObligoError::InvalidCycle
            );
        }
    }

    // Boxed onto the heap by the `Vec`, and deliberately: sixteen of this program's accounts is
    // well past what a 4KB SBF stack frame will hold, and the linker will not stop you.
    let mut merchants: Vec<Account<Merchant>> = Vec::with_capacity(k);
    for info in merchant_infos {
        require!(info.is_writable, ObligoError::InvalidCycle);
        require_keys_eq!(*info.owner, crate::ID, ObligoError::InvalidCycle);
        // Checks the discriminator too — an `Obligation` is also one of ours, and read at raw byte
        // offsets as a `Merchant` it would land somewhere plausible.
        merchants.push(Account::try_from(info)?);
    }

    let mut edges: Vec<Account<Obligation>> = Vec::with_capacity(k);
    let mut min_amount = u64::MAX;

    for (i, info) in edge_infos.iter().enumerate() {
        require!(info.is_writable, ObligoError::InvalidCycle);
        require_keys_eq!(*info.owner, crate::ID, ObligoError::InvalidCycle);

        let edge: Account<Obligation> = Account::try_from(info)?;

        let debtor = merchant_infos[i].key();
        let creditor = merchant_infos[(i + 1) % k].key();

        // Debt has a direction, and this edge has to run the way the ring does.
        require_keys_eq!(edge.debtor, debtor, ObligoError::InvalidCycle);
        require_keys_eq!(edge.creditor, creditor, ObligoError::InvalidCycle);

        // And the account has to *be* the edge between them. `create_program_address` with the
        // bump the account itself carries — never `find_program_address`, which costs 12,136 CU to
        // rediscover a number we wrote down when we created the account.
        let derived = Pubkey::create_program_address(
            &[
                OBLIGATION_SEED,
                debtor.as_ref(),
                creditor.as_ref(),
                &[edge.bump],
            ],
            &crate::ID,
        )
        .map_err(|_| error!(ObligoError::InvalidCycle))?;
        require_keys_eq!(info.key(), derived, ObligoError::InvalidCycle);

        min_amount = min_amount.min(edge.amount);
        edges.push(edge);
    }

    // A ring with a dead edge clears nothing, and would still write to sixteen accounts doing it.
    require!(min_amount > 0, ObligoError::EmptyCycle);

    // Each merchant in the ring is the debtor on exactly one edge and the creditor on exactly one
    // other, and both of those edges are at least `min_amount`. So both sides of its books come
    // down by the same number, and the sum of all obligations stays symmetric.
    for i in 0..k {
        edges[i].amount = edges[i]
            .amount
            .checked_sub(min_amount)
            .ok_or(ObligoError::Overflow)?;
        merchants[i].obligations_out = merchants[i]
            .obligations_out
            .checked_sub(min_amount)
            .ok_or(ObligoError::Overflow)?;
        merchants[i].obligations_in = merchants[i]
            .obligations_in
            .checked_sub(min_amount)
            .ok_or(ObligoError::Overflow)?;
    }

    // Loaded by hand, so written back by hand — Anchor only runs `exit` for accounts it declared.
    for merchant in merchants.iter() {
        merchant.exit(&crate::ID)?;
    }
    for edge in edges.iter() {
        edge.exit(&crate::ID)?;
    }

    emit!(CycleCleared {
        cycle_len,
        amount_cleared: min_amount,
        // Not a placeholder. The claim.
        usdc_moved: 0,
    });

    Ok(())
}
