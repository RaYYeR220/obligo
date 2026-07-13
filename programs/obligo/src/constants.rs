/// Global config. One per deployment.
pub const PROTOCOL_SEED: &[u8] = b"protocol";

/// The program's signer PDA. The hook grants a permit to nobody else.
pub const AUTHORITY_SEED: &[u8] = b"authority";

pub const MERCHANT_SEED: &[u8] = b"merchant";
pub const VAULT_SEED: &[u8] = b"vault";
pub const POINTS_SEED: &[u8] = b"points";
pub const BATCH_SEED: &[u8] = b"batch";

/// An acceptor's standing bid for one issuer's points: `[b"offer", acceptor, issuer]`.
pub const OFFER_SEED: &[u8] = b"offer";

/// One directed edge of the debt graph: `[b"obligation", debtor, creditor]`.
pub const OBLIGATION_SEED: &[u8] = b"obligation";

/// An acceptance rate must be a real bid. Zero would be an acceptor taking a customer's points
/// and handing back nothing; past 200% it is a fat-fingered decimal rather than a strategy.
pub const MIN_RATE_BPS: u16 = 1;
pub const MAX_RATE_BPS: u16 = 20_000;

/// Points are whole things a cashier can count. They do not have decimals.
pub const POINT_DECIMALS: u8 = 0;

pub const MAX_NAME_LEN: usize = 32;
pub const MAX_SYMBOL_LEN: usize = 10;
pub const MAX_URI_LEN: usize = 200;

/// One year, in seconds. Just an upper bound on declared terms; not a policy.
pub const MAX_POINT_TTL: i64 = 60 * 60 * 24 * 365 * 5;
