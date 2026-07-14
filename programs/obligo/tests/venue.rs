//! Composability, end to end: a third-party point-of-sale becomes an Obligo redemption venue by
//! CPI, permissionlessly.
//!
//! `obligo_venue` is a separate program that knows nothing of Obligo's reserve maths. Its `checkout`
//! records a `Receipt` of its own and, in the same instruction, calls the core's `redeem` by hand-
//! built CPI. This suite deploys the core, the hook and the venue into one SVM, sets up an issuer, a
//! store and a customer the ordinary way, rings up a checkout, and proves the redemption happened
//! *through the venue*: the points were burned, the obligation was booked, the venue's receipt was
//! written, and both programs' events fired in the one transaction.

mod common;

use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use common::*;
use obligo::state::MerchantStatus;
use solana_keypair::Keypair;
use solana_signer::Signer;

const VENUE_SO: &str = "target/deploy/obligo_venue.so";

fn venue_program() -> solana_pubkey::Pubkey {
    obligo_venue::ID
}

fn receipt_address(store: &solana_pubkey::Pubkey, receipt_id: u64) -> solana_pubkey::Pubkey {
    solana_pubkey::Pubkey::find_program_address(
        &[
            obligo_venue::RECEIPT_SEED,
            store.as_ref(),
            &receipt_id.to_le_bytes(),
        ],
        &obligo_venue::ID,
    )
    .0
}

/// The whole checkout instruction, assembled the way an integrator's client would: the venue's own
/// two accounts plus the accounts the core's `redeem` needs, forwarded.
#[allow(clippy::too_many_arguments)]
fn checkout(
    env: &mut Env,
    issuer: &MerchantHandle,
    store: &MerchantHandle,
    customer: &Keypair,
    receipt_id: u64,
    points: u64,
    price: u64,
) -> Result<litesvm::types::TransactionMetadata, solana_transaction_error::TransactionError> {
    let customer_points = associated_token_address(&customer.pubkey(), &issuer.points_mint);

    let ix = solana_instruction::Instruction {
        program_id: obligo_venue::ID,
        accounts: obligo_venue::accounts::Checkout {
            payer: env.payer.pubkey(),
            customer: customer.pubkey(),
            issuer: issuer.merchant,
            acceptor: store.merchant,
            receipt: receipt_address(&store.merchant, receipt_id),
            obligo_program: obligo::ID,
            protocol: protocol_address(),
            offer: offer_address(&store.merchant, &issuer.merchant),
            obligation: obligation_address(&issuer.merchant, &store.merchant),
            points_mint: issuer.points_mint,
            customer_points,
            redemption_escrow: associated_token_address(&issuer.merchant, &issuer.points_mint),
            batch: batch_address(&issuer.merchant, &customer.pubkey()),
            core_authority: core_authority(),
            permit: permit_address(&customer_points),
            extra_account_meta_list: eaml_address(&issuer.points_mint),
            hook_program: obligo_hook::ID,
            token_program: TOKEN_2022_ID,
            associated_token_program: ATA_PROGRAM_ID,
            system_program: SYSTEM_PROGRAM_ID,
        }
        .to_account_metas(None),
        data: obligo_venue::instruction::Checkout {
            receipt_id,
            points,
            price,
        }
        .data(),
    };

    let customer = customer.insecure_clone();
    // A checkout is a redemption (~119k CU) wrapped in a receipt init and a CPI hop. Give it room;
    // a till should never have to reason about the compute ceiling.
    env.send_meta(&[compute_limit(400_000), ix], &[&customer])
}

/// Cafe Aurora issues $0.01 points on a 30% reserve. Bodega Belmont — running an `obligo_venue`
/// till — bids 110% for them. A customer holds 1000 of Aurora's points.
fn scene(env: &mut Env) -> (MerchantHandle, MerchantHandle, Keypair) {
    let aurora = env.issuer("Cafe Aurora", 10_000, 3000, 3 * DOLLAR);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 3 * DOLLAR);
    let customer = Keypair::new();

    env.issue(&aurora, &customer.pubkey(), 1000).expect("issue");

    let expires_at = env.now() + 30 * 86_400;
    env.post_offer(&belmont, &aurora, 11_000, 250 * DOLLAR, expires_at)
        .expect("post_offer");

    (aurora, belmont, customer)
}

#[test]
fn a_third_party_venue_redeems_obligo_points_by_cpi() {
    let mut env = Env::new();
    env.add_program(venue_program(), VENUE_SO);
    let (aurora, belmont, customer) = scene(&mut env);

    let aurora_vault = env.token_balance(&aurora.vault);
    let belmont_vault = env.token_balance(&belmont.vault);

    // The store rings up order #42: the customer spends 500 of Aurora's points against an $8.75 item.
    let meta = checkout(&mut env, &aurora, &belmont, &customer, 42, 500, 8_750_000)
        .expect("checkout redeems by CPI into the core");

    // ---- the redemption happened, through the venue -----------------------------------------
    // The points are gone from the supply — burned by the core inside the CPI, not parked anywhere.
    assert_eq!(env.points_balance(&aurora, &customer.pubkey()), 500);
    assert_eq!(env.points_supply(&aurora), 500);
    assert_eq!(env.batch_state(&aurora, &customer.pubkey()).amount, 500);
    assert_eq!(
        env.escrow_balance(&aurora),
        0,
        "the escrow is a turnstile, not a vault"
    );

    // Aurora now owes Belmont the full $5.00 of face, as a named edge in the graph.
    let a = env.merchant_state(&aurora);
    assert_eq!(a.points_outstanding, 500);
    assert_eq!(a.total_redeemed, 500);
    assert_eq!(a.obligations_out, 5 * DOLLAR);
    assert_eq!(a.status, MerchantStatus::Active);
    let b = env.merchant_state(&belmont);
    assert_eq!(b.obligations_in, 5 * DOLLAR);
    let edge = env.obligation_state(&aurora, &belmont);
    assert_eq!(edge.debtor, aurora.merchant);
    assert_eq!(edge.creditor, belmont.merchant);
    assert_eq!(edge.amount, 5 * DOLLAR);

    // And no USDC moved: a redemption transfers liability, never cash.
    assert_eq!(env.token_balance(&aurora.vault), aurora_vault);
    assert_eq!(env.token_balance(&belmont.vault), belmont_vault);

    // ---- the venue's own record ------------------------------------------------------------
    let receipt_addr = receipt_address(&belmont.merchant, 42);
    let raw = env.raw_data(&receipt_addr);
    let receipt = obligo_venue::Receipt::try_deserialize(&mut raw.as_slice()).expect("receipt");
    assert_eq!(receipt.customer, customer.pubkey());
    assert_eq!(receipt.store, belmont.merchant);
    assert_eq!(receipt.issuer, aurora.merchant);
    assert_eq!(receipt.receipt_id, 42);
    assert_eq!(receipt.points, 500);
    assert_eq!(receipt.price, 8_750_000);
    assert_eq!(
        receipt.timestamp,
        env.now(),
        "the venue stamps the on-chain clock"
    );

    // ---- both programs told the world, in the one transaction ------------------------------
    // The venue's own event...
    let sale = decode_event::<obligo_venue::Sale>(&meta);
    assert_eq!(sale.store, belmont.merchant);
    assert_eq!(sale.customer, customer.pubkey());
    assert_eq!(sale.issuer, aurora.merchant);
    assert_eq!(sale.receipt_id, 42);
    assert_eq!(sale.points, 500);
    assert_eq!(sale.price, 8_750_000);

    // ...and the core's `Redeemed`, emitted by the CPI it drove. Same tx, so the sale and the
    // redemption are atomic: this is what "the points are spent together with the sale" means.
    let redeemed = decode_redeemed(&meta);
    assert_eq!(redeemed.issuer, aurora.merchant);
    assert_eq!(redeemed.acceptor, belmont.merchant);
    assert_eq!(redeemed.customer, customer.pubkey());
    assert_eq!(redeemed.points, 500);
    assert_eq!(redeemed.value_face, 5 * DOLLAR);
    assert_eq!(redeemed.goods_value, 5_500_000);
}

/// Atomicity is the whole promise of "spent together with the sale." When the core refuses the
/// redemption, the venue's receipt must not survive either — or a store could bank a sale it was
/// never actually paid for in points.
#[test]
fn a_checkout_the_core_rejects_writes_no_receipt() {
    let mut env = Env::new();
    env.add_program(venue_program(), VENUE_SO);
    let aurora = env.issuer("Cafe Aurora", 10_000, 3000, 3 * DOLLAR);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 3 * DOLLAR);
    let customer = Keypair::new();
    env.issue(&aurora, &customer.pubkey(), 1000).expect("issue");

    // Belmont's budget for Aurora is only $1.00 of face, but the customer tries to spend $5.00.
    let expires_at = env.now() + 30 * 86_400;
    env.post_offer(&belmont, &aurora, 11_000, DOLLAR, expires_at)
        .expect("post_offer");

    let err = checkout(&mut env, &aurora, &belmont, &customer, 7, 500, 8_750_000)
        .expect_err("the core must refuse: the offer's budget is exhausted");
    assert_custom_error(err, E_OFFER_EXHAUSTED);

    // The whole transaction reverted: the points were not spent and no receipt was written.
    assert_eq!(env.points_balance(&aurora, &customer.pubkey()), 1000);
    assert_eq!(env.merchant_state(&aurora).obligations_out, 0);
    assert!(!env.account_exists(&receipt_address(&belmont.merchant, 7)));
}

/// The venue calls the core by wire format, so that wire format is a contract. Pin the pieces it
/// hardcodes against the core crate itself: a rename or a reordering over there turns this red
/// rather than silently breaking every checkout on-chain. This is `hook_abi.rs`, from the other side.
#[test]
fn the_redeem_wire_format_the_venue_hardcodes_is_the_core_s_own() {
    use anchor_lang::{Discriminator, InstructionData};

    // The 8-byte discriminator...
    assert_eq!(
        obligo_venue::REDEEM_DISCRIMINATOR,
        obligo::instruction::Redeem::DISCRIMINATOR,
    );

    // ...and the full call: discriminator followed by borsh `points: u64`, exactly as the core
    // decodes it. Anchor's own encoding of the instruction is the reference.
    let anchors = obligo::instruction::Redeem {
        points: 7_000_000_000_000_000_042,
    }
    .data();

    let mut ours = obligo_venue::REDEEM_DISCRIMINATOR.to_vec();
    ours.extend_from_slice(&7_000_000_000_000_000_042u64.to_le_bytes());

    assert_eq!(ours, anchors);

    // And the core it invokes is the one this venue is pinned to.
    assert_eq!(obligo_venue::OBLIGO_CORE_ID, obligo::ID);
}
