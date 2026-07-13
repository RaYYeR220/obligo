use anchor_lang::prelude::*;

#[error_code]
pub enum ObligoError {
    #[msg("arithmetic overflow")]
    Overflow,
    #[msg("the merchant would hold less collateral than its outstanding points require")]
    ReserveBreached,
    #[msg("terms are out of range")]
    InvalidTerms,
    #[msg("amount must be greater than zero")]
    InvalidAmount,
    #[msg("the merchant has defaulted and may no longer issue points")]
    MerchantDefaulted,
    #[msg("merchant name is too long")]
    NameTooLong,
    #[msg("the merchant does not hold that much collateral")]
    InsufficientCollateral,
    #[msg("the face value of a point cannot be repriced while points are outstanding")]
    TermsLocked,
    #[msg("token metadata is too long")]
    MetadataTooLong,
    #[msg("the merchant already has a points mint")]
    MintAlreadyExists,
    #[msg("an acceptance rate must be between 1 and 20000 bps")]
    InvalidRate,
    #[msg("the acceptance offer has expired")]
    OfferExpired,
    #[msg("a merchant cannot post an acceptance offer against its own points")]
    SelfOffer,
    #[msg("the redemption would exceed the acceptor's remaining budget for this issuer")]
    OfferExhausted,
    #[msg("the issuer has defaulted and its points can no longer be redeemed")]
    IssuerDefaulted,
    #[msg("these points are past the issuer's time to live")]
    PointsExpired,
    #[msg("the customer does not hold that many of this merchant's points")]
    InsufficientPoints,
    #[msg("these two merchants owe each other nothing")]
    NothingToSettle,
    #[msg("the accounts given do not describe a real cycle in the obligation graph")]
    InvalidCycle,
    #[msg("a cycle whose smallest edge is zero clears nothing")]
    EmptyCycle,
}
