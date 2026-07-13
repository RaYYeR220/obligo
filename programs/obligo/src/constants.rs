/// Global config. One per deployment.
pub const PROTOCOL_SEED: &[u8] = b"protocol";

/// The program's signer PDA. The hook grants a permit to nobody else.
pub const AUTHORITY_SEED: &[u8] = b"authority";

pub const MERCHANT_SEED: &[u8] = b"merchant";
pub const VAULT_SEED: &[u8] = b"vault";
pub const POINTS_SEED: &[u8] = b"points";
pub const BATCH_SEED: &[u8] = b"batch";

/// Points are whole things a cashier can count. They do not have decimals.
pub const POINT_DECIMALS: u8 = 0;

pub const MAX_NAME_LEN: usize = 32;
pub const MAX_SYMBOL_LEN: usize = 10;
pub const MAX_URI_LEN: usize = 200;

/// One year, in seconds. Just an upper bound on declared terms; not a policy.
pub const MAX_POINT_TTL: i64 = 60 * 60 * 24 * 365 * 5;
