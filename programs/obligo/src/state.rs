use anchor_lang::prelude::*;

use crate::constants::MAX_NAME_LEN;

/// Global configuration. Its `authority` may change global parameters and nothing else:
/// it cannot move a merchant's collateral, mint or burn a point, cancel an obligation or
/// block a redemption. There is no instruction in this program that would let it.
#[account]
#[derive(InitSpace)]
pub struct Protocol {
    pub authority: Pubkey,
    pub usdc_mint: Pubkey,
    pub hook_program: Pubkey,
    pub merchant_count: u64,
    pub bump: u8,
    /// Bump of `[b"authority"]`, the signer PDA the hook trusts. Stored so no instruction
    /// ever has to pay 12,136 CU to rediscover it.
    pub authority_bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum MerchantStatus {
    Active,
    Defaulted,
}

/// One issuer of loyalty points, its declared terms, and its books.
#[account]
#[derive(InitSpace)]
pub struct Merchant {
    pub authority: Pubkey,
    pub points_mint: Pubkey,
    pub vault: Pubkey,
    #[max_len(MAX_NAME_LEN)]
    pub name: String,

    // Terms the merchant declares and the protocol enforces.
    /// USDC micro-units the merchant will honour per point. 10_000 => 1 point = $0.01.
    pub usdc_per_point: u64,
    /// Fraction of the face value of outstanding points that must be collateralised.
    /// Below 10_000 this is a fractional reserve, which is the entire idea.
    pub reserve_bps: u16,
    /// Seconds a batch of points survives without further activity.
    pub point_ttl: i64,

    // Books. USDC micro-units, except the three point counters.
    pub collateral: u64,
    pub points_outstanding: u64,
    /// Owed BY this merchant to other merchants that honoured its points.
    pub obligations_out: u64,
    /// Owed TO this merchant by issuers whose points it honoured.
    pub obligations_in: u64,
    pub total_issued: u64,
    pub total_redeemed: u64,
    pub total_expired: u64,

    pub status: MerchantStatus,
    pub bump: u8,
    pub vault_bump: u8,
    pub mint_bump: u8,
}

/// Points a single customer holds from a single merchant, and when they were last touched.
/// Expiry needs something to act on; this is it.
#[account]
#[derive(InitSpace)]
pub struct PointBatch {
    pub merchant: Pubkey,
    pub customer: Pubkey,
    pub amount: u64,
    /// Reset on every issuance to that customer: the TTL runs from last activity, the way
    /// every real loyalty programme states it.
    pub issued_at: i64,
    pub bump: u8,
}

/// One merchant's standing bid to honour another merchant's points.
///
/// The auction is the interesting half of this protocol. An acceptor that wants footfall bids
/// *above* face and eats the difference as customer acquisition; an acceptor that doubts the
/// issuer's credit bids *below* face. Either way it claims face — the full 100% — from the
/// issuer, and `rate_bps` prices only the goods it hands the customer.
#[account]
#[derive(InitSpace)]
pub struct AcceptanceOffer {
    /// The merchant honouring the points.
    pub acceptor: Pubkey,
    /// The merchant whose points are honoured.
    pub issuer: Pubkey,
    /// Goods given, as a fraction of face. Above 10_000 the acceptor is buying footfall; below
    /// it, discounting the issuer's credit.
    pub rate_bps: u16,
    /// USDC micro of *face value* this offer will absorb. The acceptor's acquisition budget,
    /// and the only line in an offer that the chain enforces against the acceptor's own till.
    pub capacity: u64,
    /// Face value redeemed against this offer so far.
    pub consumed: u64,
    pub expires_at: i64,
    pub bump: u8,
}

/// One directed edge of the debt graph: what `debtor` owes `creditor`, in USDC micro.
///
/// Deliberately flat. Settlement nets two of these against each other, and cycle clearing walks a
/// ring of them and decrements every one by the smallest — so an edge has to be re-derivable from
/// nothing but the pair it connects, with a bump already in hand. `debtor` and `creditor` are held
/// here, rather than inferred from the account's address, so a crank handed a ring of raw accounts
/// can rebuild every seed and prove the ring is real before it touches a single number.
#[account]
#[derive(InitSpace)]
pub struct Obligation {
    pub debtor: Pubkey,
    pub creditor: Pubkey,
    pub amount: u64,
    pub bump: u8,
}
