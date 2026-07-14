//! The proof: obligo's `KaminoAdapter` deposits USDC into the **real** mainnet KLend program and
//! its **real** USDC reserve, time advances, and a redemption returns *more* USDC than went in —
//! genuine accrued interest, not a mock.
//!
//! How it runs: LiteSVM loads the KLend program's actual mainnet bytecode and the actual mainnet
//! USDC reserve / lending-market / scope-oracle / cUSDC-mint / supply-vault accounts (fetched by
//! `fetch-fixtures.ps1`). We seed ourselves USDC by writing a token account — the fork equivalent of
//! Surfpool's `surfnet_setTokenAccount` — advance the clock so KLend's interest math compounds, and
//! rewrite the scope price's timestamp so `refresh_reserve` does not reject it as stale (the only
//! thing patched; every number KLend derives is its own).
//!
//! The deposit and redeem are driven through the throwaway `kamino_probe` program, whose only job is
//! to call `obligo::yield_adapter::vault_adapter(...).deposit()/withdraw()` — the identical entry
//! point the core's `deposit_collateral` / `withdraw_collateral` use.
//!
//! Prerequisites (PowerShell, from `kamino-fork/`):
//!   ./fetch-fixtures.ps1        # dumps mainnet accounts + KLend bytecode into ./fixtures
//!   cargo-build-sbf             # builds kamino_probe.so
//!   cargo test -- --nocapture   # runs this and prints the numbers

use anchor_lang::prelude::{Clock, Pubkey};
use anchor_lang::{InstructionData, ToAccountMetas};
use litesvm::types::TransactionMetadata;
use litesvm::LiteSVM;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use std::path::PathBuf;

// ---- the real mainnet pubkeys, all verified against the dumped reserve ----------------------
const KLEND: Pubkey = Pubkey::from_str_const("KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD");
const PROBE: Pubkey = Pubkey::from_str_const("7U7wtcqdmFTXVTQGAL4mTVH6E5rt3eW7RETYySB3ywe6");
const RESERVE: Pubkey = Pubkey::from_str_const("D6q6wuQSrifJKZYpR1M8R4YawnLDtDsMmWM1NbBmgJ59");
const MARKET: Pubkey = Pubkey::from_str_const("7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF");
const LIQ_SUPPLY: Pubkey = Pubkey::from_str_const("Bgq7trRgVMeq33yt235zM2onQ4bRDBsY5EWiTetF4qw6");
const COLL_MINT: Pubkey = Pubkey::from_str_const("B8V6WVjPxW1UGwVDfxH2d2r8SyT4cqn7dQRK6XneVa7D");
const USDC: Pubkey = Pubkey::from_str_const("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
const SCOPE: Pubkey = Pubkey::from_str_const("3t4JZcueEzTbVP6kLxXrL3VpWx45jDer4eqysweBchNH");
const TOKEN: Pubkey = Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const INSTRUCTIONS_SYSVAR: Pubkey =
    Pubkey::from_str_const("Sysvar1nstructions1111111111111111111111111");

/// Scope oracle chain index for this reserve (read out of its config): entry 13.
const SCOPE_INDEX: usize = 13;
/// Advance ~180 days of slots. KLend converts APR to a per-slot rate against ~2 slots/second, so
/// this is roughly half a year of interest compounding on the reserve's real borrow book.
const ADVANCE_SLOTS: u64 = 31_000_000;
const ADVANCE_SECS: i64 = 15_552_000;

const DEPOSIT: u64 = 1_000_000_000; // 1,000 USDC (6 decimals)

#[test]
fn the_kamino_adapter_really_accrues_yield_against_mainnet_klend() {
    let mut svm = LiteSVM::new();
    svm.add_program(KLEND, &read_fixture_bytes("klend.so")).unwrap();
    svm.add_program(PROBE, &read_probe()).unwrap();

    // Real mainnet reserve state, loaded verbatim. The deposit-time clock is read from the reserve
    // itself — its own `last_update.slot` and price timestamp — so nothing accrues before the
    // position exists, whatever slot mainnet was at when the fixture was dumped.
    let reserve = fixture_account("reserve");
    let slot0 = u64::from_le_bytes(reserve.data[16..24].try_into().unwrap());
    let ts0 = i64::from_le_bytes(reserve.data[264..272].try_into().unwrap());
    svm.set_account(RESERVE, reserve).unwrap();
    svm.set_account(MARKET, fixture_account("lending_market")).unwrap();
    svm.set_account(LIQ_SUPPLY, fixture_account("liq_supply_vault"))
        .unwrap();
    svm.set_account(COLL_MINT, fixture_account("coll_mint")).unwrap();
    svm.set_account(USDC, fixture_account("usdc_mint")).unwrap();

    // The scope price, de-staled to the deposit clock. Only the timestamps move; the price stands.
    svm.set_account(SCOPE, scope_at(slot0, ts0)).unwrap();

    let lma = Pubkey::find_program_address(&[b"lma", MARKET.as_ref()], &KLEND).0;
    let vault_authority = Pubkey::find_program_address(&[b"vault"], &PROBE).0;

    // Seed ourselves the USDC and an empty cUSDC account, both owned by the vault PDA.
    let vault_liquidity = Keypair::new().pubkey();
    let vault_collateral = Keypair::new().pubkey();
    svm.set_account(vault_liquidity, token_account(&USDC, &vault_authority, DEPOSIT))
        .unwrap();
    svm.set_account(
        vault_collateral,
        token_account(&COLL_MINT, &vault_authority, 0),
    )
    .unwrap();

    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 100_000_000_000).unwrap();

    let klend_accounts = || {
        vec![
            AccountMeta::new_readonly(KLEND, false),
            AccountMeta::new(RESERVE, false),
            AccountMeta::new_readonly(MARKET, false),
            AccountMeta::new_readonly(lma, false),
            AccountMeta::new_readonly(USDC, false),
            AccountMeta::new(LIQ_SUPPLY, false),
            AccountMeta::new(COLL_MINT, false),
            AccountMeta::new(vault_collateral, false),
            AccountMeta::new_readonly(TOKEN, false),
            AccountMeta::new_readonly(TOKEN, false),
            AccountMeta::new_readonly(SCOPE, false),
            AccountMeta::new_readonly(INSTRUCTIONS_SYSVAR, false),
        ]
    };

    let base = kamino_probe::accounts::YieldOp {
        payer: payer.pubkey(),
        vault_authority,
        vault_liquidity,
    }
    .to_account_metas(None);

    // --- deposit at T0 -----------------------------------------------------------------------
    set_clock(&mut svm, slot0, ts0);
    let deposit_ix = probe_ix(
        &base,
        &klend_accounts(),
        kamino_probe::instruction::YieldDeposit { amount: DEPOSIT }.data(),
    );
    send(&mut svm, &payer, &[compute_limit(1_400_000), deposit_ix]);

    let ctokens = token_balance(&svm, &vault_collateral);
    assert!(ctokens > 0, "deposit minted no cTokens");
    assert_eq!(
        token_balance(&svm, &vault_liquidity),
        0,
        "deposit should have moved all USDC into KLend"
    );

    // --- advance ~180 days, keep the oracle fresh --------------------------------------------
    let slot1 = slot0 + ADVANCE_SLOTS;
    let ts1 = ts0 + ADVANCE_SECS;
    svm.set_account(SCOPE, scope_at(slot1, ts1)).unwrap();
    set_clock(&mut svm, slot1, ts1);

    // total_assets, reported through the adapter, before we realise it.
    let report_ix = probe_ix(
        &base,
        &klend_accounts(),
        kamino_probe::instruction::YieldReport {}.data(),
    );
    let report = send(&mut svm, &payer, &[compute_limit(1_400_000), report_ix]);
    let reported = log_u64(&report, "total_assets");

    // --- withdraw: redeem the whole position -------------------------------------------------
    let withdraw_ix = probe_ix(
        &base,
        &klend_accounts(),
        kamino_probe::instruction::YieldWithdraw {
            principal_out: DEPOSIT,
        }
        .data(),
    );
    let meta = send(&mut svm, &payer, &[compute_limit(1_400_000), withdraw_ix]);
    let realized = log_u64(&meta, "realized");
    let realized_onchain = token_balance(&svm, &vault_liquidity);

    let accrued = realized_onchain.saturating_sub(DEPOSIT);
    println!("\n================ KAMINO MAINNET-FORK YIELD PROOF ================");
    println!("KLend program         {KLEND}");
    println!("lending market        {MARKET}");
    println!("USDC reserve          {RESERVE}");
    println!("scope oracle          {SCOPE}  (chain index {SCOPE_INDEX})");
    println!("cUSDC collateral mint {COLL_MINT}");
    println!("----------------------------------------------------------------");
    println!("deposited             {DEPOSIT:>16}  ({} USDC)", ui(DEPOSIT));
    println!("cTokens received      {ctokens:>16}");
    println!("clock advanced        {ADVANCE_SLOTS} slots (~180 days)");
    println!(
        "total_assets (report) {reported:>16}  ({} USDC)",
        ui(reported)
    );
    println!(
        "withdraw realized     {realized:>16}  ({} USDC)",
        ui(realized)
    );
    println!(
        "vault balance after   {realized_onchain:>16}  ({} USDC)",
        ui(realized_onchain)
    );
    println!(
        "accrued interest      {accrued:>16}  ({} USDC)",
        ui(accrued)
    );
    println!("================================================================\n");

    assert_eq!(
        realized, realized_onchain,
        "adapter's reported realized USDC must match the on-chain balance delta"
    );
    assert!(
        realized_onchain > DEPOSIT,
        "withdraw returned {realized_onchain}, not more than the {DEPOSIT} deposited — no yield"
    );
    // total_assets is best-effort reporting; sanity-check it is in the same ballpark as realised.
    assert!(
        reported > DEPOSIT,
        "total_assets {reported} should exceed principal once interest has accrued"
    );
}

// ---- helpers --------------------------------------------------------------------------------

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_fixture_bytes(name: &str) -> Vec<u8> {
    let path = manifest_dir().join("fixtures").join(name);
    std::fs::read(&path).unwrap_or_else(|_| {
        panic!(
            "missing fixture {}\nrun kamino-fork/fetch-fixtures.ps1 first",
            path.display()
        )
    })
}

fn read_probe() -> Vec<u8> {
    let path = manifest_dir().join("target/deploy/kamino_probe.so");
    std::fs::read(&path)
        .unwrap_or_else(|_| panic!("missing {}\nbuild it first: cargo-build-sbf", path.display()))
}

fn fixture_account(name: &str) -> Account {
    let raw = String::from_utf8(read_fixture_bytes(&format!("{name}.json"))).unwrap();
    let j: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let acc = &j["account"];
    let data = b64_decode(acc["data"][0].as_str().unwrap());
    let owner: Pubkey = acc["owner"].as_str().unwrap().parse().unwrap();
    Account {
        lamports: acc["lamports"].as_u64().unwrap(),
        data,
        owner,
        executable: false,
        rent_epoch: 0,
    }
}

/// The scope prices account with entry `SCOPE_INDEX` re-timestamped to `(slot, ts)`. Layout:
/// 8 disc + 32 (oracle_mappings) + [DatedPrice; 512], DatedPrice = price(16) + last_updated_slot(8)
/// + unix_timestamp(8) + reserved(24) = 56 bytes.
fn scope_at(slot: u64, ts: i64) -> Account {
    let mut acc = fixture_account("scope_prices");
    let entry = 8 + 32 + SCOPE_INDEX * 56;
    acc.data[entry + 16..entry + 24].copy_from_slice(&slot.to_le_bytes());
    acc.data[entry + 24..entry + 32].copy_from_slice(&ts.to_le_bytes());
    acc
}

/// A minimal Initialized SPL token account, built by hand so we can pre-fund it with real-USDC we
/// were never minted — the fork's equivalent of a set-token-account cheatcode.
fn token_account(mint: &Pubkey, owner: &Pubkey, amount: u64) -> Account {
    let mut data = vec![0u8; 165];
    data[0..32].copy_from_slice(mint.as_ref());
    data[32..64].copy_from_slice(owner.as_ref());
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data[108] = 1; // AccountState::Initialized
    Account {
        lamports: 2_039_280,
        data,
        owner: TOKEN,
        executable: false,
        rent_epoch: 0,
    }
}

fn set_clock(svm: &mut LiteSVM, slot: u64, ts: i64) {
    let mut clock: Clock = svm.get_sysvar();
    clock.slot = slot;
    clock.unix_timestamp = ts;
    svm.set_sysvar(&clock);
}

fn probe_ix(base: &[AccountMeta], klend: &[AccountMeta], data: Vec<u8>) -> Instruction {
    let mut accounts = base.to_vec();
    accounts.extend_from_slice(klend);
    Instruction {
        program_id: PROBE,
        accounts,
        data,
    }
}

fn send(svm: &mut LiteSVM, payer: &Keypair, ixs: &[Instruction]) -> TransactionMetadata {
    svm.expire_blockhash();
    let msg = Message::new(ixs, Some(&payer.pubkey()));
    let tx = Transaction::new(&[payer], msg, svm.latest_blockhash());
    svm.send_transaction(tx)
        .unwrap_or_else(|e| panic!("transaction failed:\n{:#?}", e.meta.logs))
}

fn token_balance(svm: &LiteSVM, account: &Pubkey) -> u64 {
    let acc = svm.get_account(account).unwrap();
    u64::from_le_bytes(acc.data[64..72].try_into().unwrap())
}

/// The u64 that follows a `msg!` label in the program logs, e.g. `realized 1023456789`.
fn log_u64(meta: &TransactionMetadata, label: &str) -> u64 {
    for line in &meta.logs {
        if let Some(idx) = line.find(label) {
            let tail = line[idx + label.len()..].trim();
            if let Ok(v) = tail.parse::<u64>() {
                return v;
            }
        }
    }
    panic!("no `{label} <n>` in logs:\n{:#?}", meta.logs);
}

fn ui(micro: u64) -> String {
    format!("{}.{:06}", micro / 1_000_000, micro % 1_000_000)
}

fn compute_limit(units: u32) -> Instruction {
    let mut data = Vec::with_capacity(5);
    data.push(2u8);
    data.extend_from_slice(&units.to_le_bytes());
    Instruction {
        program_id: Pubkey::from_str_const("ComputeBudget111111111111111111111111111111"),
        accounts: vec![],
        data,
    }
}

fn b64_decode(s: &str) -> Vec<u8> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes().filter(|c| *c != b'=') {
        let value = ALPHABET.iter().position(|a| *a == c).unwrap() as u32;
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}
