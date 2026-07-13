//! Behaviour of the transfer hook, driven through a real Token-2022 program in litesvm.
//!
//! Prerequisites (PowerShell, from the workspace root):
//!   cargo-build-sbf
//!   cargo-build-sbf --manifest-path tests/mock_core/Cargo.toml
//!   cargo test -p obligo_hook

use anchor_lang::{prelude::Pubkey, AccountDeserialize, InstructionData, ToAccountMetas};
use anchor_spl::token_2022::spl_token_2022::{
    extension::ExtensionType,
    instruction as token_ix,
    state::{Account as TokenAccountState, Mint as MintState},
};
use litesvm::LiteSVM;
use obligo_hook::{Permit, CORE_PROGRAM_ID, PERMIT_KIND_REDEEM};
use solana_instruction::{error::InstructionError, AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;
use spl_discriminator::SplDiscriminate;
use spl_transfer_hook_interface::instruction::ExecuteInstruction;
use std::path::PathBuf;

const TOKEN_2022_ID: Pubkey = anchor_spl::token_2022::ID;
const SYSTEM_PROGRAM_ID: Pubkey = solana_system_interface::program::ID;
const DECIMALS: u8 = 0;

// HookError, as Anchor emits it: 6000 + the variant's index.
const E_MOVEMENT_NOT_AUTHORIZED: u32 = 6000;
const E_AMOUNT_EXCEEDS_PERMIT: u32 = 6001;
const E_NOT_TRANSFERRING: u32 = 6002;

// Anchor's own constraint failures.
const E_CONSTRAINT_SIGNER: u32 = 2002;
const E_CONSTRAINT_SEEDS: u32 = 2006;

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = <root>/programs/obligo_hook
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn read_program(relative: &str, build_hint: &str) -> Vec<u8> {
    let path = workspace_root().join(relative);
    std::fs::read(&path)
        .unwrap_or_else(|_| panic!("missing {}\nbuild it first: {}", path.display(), build_hint))
}

/// A points mint with the hook installed, two token accounts, and 100 points on the source.
struct Env {
    svm: LiteSVM,
    payer: Keypair,
    /// Owner of `source`. Signs transfers.
    alice: Keypair,
    mint: Pubkey,
    source: Pubkey,
    destination: Pubkey,
}

impl Env {
    fn new() -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program(
            obligo_hook::ID,
            &read_program("target/deploy/obligo_hook.so", "cargo-build-sbf"),
        )
        .unwrap();
        svm.add_program(
            CORE_PROGRAM_ID,
            &read_program(
                "tests/mock_core/target/deploy/mock_core.so",
                "cargo-build-sbf --manifest-path tests/mock_core/Cargo.toml",
            ),
        )
        .unwrap();

        let payer = Keypair::new();
        let alice = Keypair::new();
        let bob = Keypair::new();
        svm.airdrop(&payer.pubkey(), 100_000_000_000).unwrap();

        let mint_kp = Keypair::new();
        let mint = mint_kp.pubkey();

        // A points mint carries the TransferHook extension, and nothing else.
        let mint_len =
            ExtensionType::try_calculate_account_len::<MintState>(&[ExtensionType::TransferHook])
                .unwrap();
        let mint_rent = svm.minimum_balance_for_rent_exemption(mint_len);

        let create_mint = solana_system_interface::instruction::create_account(
            &payer.pubkey(),
            &mint,
            mint_rent,
            mint_len as u64,
            &TOKEN_2022_ID,
        );
        // Hook authority is None: nobody, ever, can repoint this mint at a different hook.
        let init_hook =
            anchor_spl::token_2022::spl_token_2022::extension::transfer_hook::instruction::initialize(
                &TOKEN_2022_ID,
                &mint,
                None,
                Some(obligo_hook::ID),
            )
            .unwrap();
        let init_mint =
            token_ix::initialize_mint2(&TOKEN_2022_ID, &mint, &payer.pubkey(), None, DECIMALS)
                .unwrap();

        let mut env = Env {
            svm,
            payer,
            alice,
            mint,
            source: Pubkey::default(),
            destination: Pubkey::default(),
        };
        env.send(&[create_mint, init_hook, init_mint], &[&mint_kp])
            .expect("mint setup");

        // Without an EAML, Token-2022 cannot resolve the hook's extra accounts at all.
        let eaml_ix = Instruction {
            program_id: obligo_hook::ID,
            accounts: obligo_hook::accounts::InitializeExtraAccountMetaList {
                payer: env.payer.pubkey(),
                mint,
                extra_account_meta_list: eaml_address(&mint),
                system_program: SYSTEM_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: obligo_hook::instruction::InitializeExtraAccountMetaList {}.data(),
        };
        env.send(&[eaml_ix], &[]).expect("eaml init");

        env.source = env.create_token_account(&env.alice.pubkey());
        env.destination = env.create_token_account(&bob.pubkey());

        let mint_to = token_ix::mint_to(
            &TOKEN_2022_ID,
            &mint,
            &env.source,
            &env.payer.pubkey(),
            &[],
            100,
        )
        .unwrap();
        // MintTo never invokes the hook. That is why issuance accounting lives in the core.
        env.send(&[mint_to], &[]).expect("mint 100 points");

        env
    }

    fn send(
        &mut self,
        ixs: &[Instruction],
        extra_signers: &[&Keypair],
    ) -> Result<(), TransactionError> {
        let mut signers: Vec<&Keypair> = vec![&self.payer];
        signers.extend_from_slice(extra_signers);

        // Otherwise a replayed instruction is rejected as a duplicate transaction before the
        // hook ever sees it, and a replay test would prove nothing.
        self.svm.expire_blockhash();

        let message = Message::new(ixs, Some(&self.payer.pubkey()));
        let tx = Transaction::new(&signers, message, self.svm.latest_blockhash());

        self.svm
            .send_transaction(tx)
            .map(|_| ())
            .map_err(|failed| failed.err)
    }

    /// A token account on a hooked mint needs room for the TransferHookAccount extension —
    /// that is where Token-2022 raises the `transferring` flag the hook relies on.
    fn create_token_account(&mut self, owner: &Pubkey) -> Pubkey {
        let kp = Keypair::new();
        let len = ExtensionType::try_calculate_account_len::<TokenAccountState>(&[
            ExtensionType::TransferHookAccount,
        ])
        .unwrap();
        let rent = self.svm.minimum_balance_for_rent_exemption(len);

        let create = solana_system_interface::instruction::create_account(
            &self.payer.pubkey(),
            &kp.pubkey(),
            rent,
            len as u64,
            &TOKEN_2022_ID,
        );
        let init =
            token_ix::initialize_account3(&TOKEN_2022_ID, &kp.pubkey(), &self.mint, owner).unwrap();

        self.send(&[create, init], &[&kp]).expect("token account");
        kp.pubkey()
    }

    /// The core authorising a movement, exactly as `redeem` will: a CPI into `grant_permit`
    /// signed by `[b"authority"]`.
    fn grant(&mut self, kind: u8, amount: u64) -> Result<(), TransactionError> {
        let mut data = vec![0u8, kind];
        data.extend_from_slice(&amount.to_le_bytes());

        let ix = Instruction {
            program_id: CORE_PROGRAM_ID,
            accounts: vec![
                AccountMeta::new(self.payer.pubkey(), true),
                AccountMeta::new_readonly(core_authority(), false),
                AccountMeta::new_readonly(self.source, false),
                AccountMeta::new(permit_address(&self.source), false),
                AccountMeta::new_readonly(obligo_hook::ID, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            ],
            data,
        };
        self.send(&[ix], &[])
    }

    /// `transfer_checked` with the hook's extra accounts appended in interface order:
    /// resolved extras, then the hook program, then the EAML.
    fn transfer(&mut self, amount: u64) -> Result<(), TransactionError> {
        let mut ix = token_ix::transfer_checked(
            &TOKEN_2022_ID,
            &self.source,
            &self.mint,
            &self.destination,
            &self.alice.pubkey(),
            &[],
            amount,
            DECIMALS,
        )
        .unwrap();
        ix.accounts
            .push(AccountMeta::new(permit_address(&self.source), false));
        ix.accounts
            .push(AccountMeta::new_readonly(obligo_hook::ID, false));
        ix.accounts
            .push(AccountMeta::new_readonly(eaml_address(&self.mint), false));

        let alice = self.alice.insecure_clone();
        self.send(&[ix], &[&alice])
    }

    fn token_balance(&self, account: &Pubkey) -> u64 {
        let raw = self.svm.get_account(account).unwrap();
        // Base Account layout: amount is a u64 at offset 64.
        u64::from_le_bytes(raw.data[64..72].try_into().unwrap())
    }

    fn permit(&self) -> Option<Permit> {
        let raw = self.svm.get_account(&permit_address(&self.source))?;
        if raw.owner != obligo_hook::ID || raw.data.is_empty() {
            return None;
        }
        Some(Permit::try_deserialize(&mut raw.data.as_slice()).unwrap())
    }
}

fn eaml_address(mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"extra-account-metas", mint.as_ref()], &obligo_hook::ID).0
}

fn permit_address(source: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"permit", source.as_ref()], &obligo_hook::ID).0
}

fn core_authority() -> Pubkey {
    Pubkey::find_program_address(&[b"authority"], &CORE_PROGRAM_ID).0
}

#[track_caller]
fn assert_custom_error(err: TransactionError, expected: u32) {
    match err {
        TransactionError::InstructionError(_, InstructionError::Custom(code)) => {
            assert_eq!(code, expected, "expected custom error {expected}, got {code}");
        }
        other => panic!("expected custom error {expected}, got {other:?}"),
    }
}

#[test]
fn transfer_without_a_permit_is_rejected() {
    let mut env = Env::new();
    assert_eq!(env.token_balance(&env.source), 100);
    assert!(env.permit().is_none(), "nothing granted a permit");

    let err = env
        .transfer(10)
        .expect_err("points must not move without a permit");

    assert_custom_error(err, E_MOVEMENT_NOT_AUTHORIZED);
    assert_eq!(env.token_balance(&env.source), 100);
    assert_eq!(env.token_balance(&env.destination), 0);
}

#[test]
fn a_permit_authorises_one_movement_and_is_consumed_by_it() {
    let mut env = Env::new();
    env.grant(PERMIT_KIND_REDEEM, 10).expect("core grants 10");

    let permit = env.permit().expect("permit exists");
    assert_eq!(permit.source, env.source);
    assert_eq!(permit.kind, PERMIT_KIND_REDEEM);
    assert_eq!(permit.amount, 10);

    env.transfer(10).expect("permitted movement");

    assert_eq!(env.token_balance(&env.source), 90);
    assert_eq!(env.token_balance(&env.destination), 10);
    assert_eq!(
        env.permit().expect("permit still exists").amount,
        0,
        "the permit is one-shot and must be spent"
    );

    // The permit is dead. Replaying the same movement is exactly the attack this prevents.
    let err = env
        .transfer(10)
        .expect_err("a spent permit must not authorise a second movement");

    assert_custom_error(err, E_MOVEMENT_NOT_AUTHORIZED);
    assert_eq!(env.token_balance(&env.source), 90);
    assert_eq!(env.token_balance(&env.destination), 10);
}

#[test]
fn a_movement_larger_than_the_permit_is_rejected() {
    let mut env = Env::new();
    env.grant(PERMIT_KIND_REDEEM, 10).expect("core grants 10");

    let err = env
        .transfer(11)
        .expect_err("the permit caps the movement at 10");

    assert_custom_error(err, E_AMOUNT_EXCEEDS_PERMIT);
    assert_eq!(env.token_balance(&env.source), 100);
    assert_eq!(env.permit().unwrap().amount, 10, "the permit is untouched");
}

#[test]
fn execute_invoked_outside_a_transfer_is_rejected() {
    let mut env = Env::new();
    env.grant(PERMIT_KIND_REDEEM, 10).expect("core grants 10");

    // Hook vuln #1: `Execute` is a public entrypoint. An attacker who calls it directly, with a
    // live permit and forged accounts, would burn the permit down without moving a single point,
    // stranding the customer's redemption. Token-2022 raises `transferring` only inside a real
    // transfer, and that is the only thing standing in the way.
    let mut data = ExecuteInstruction::SPL_DISCRIMINATOR_SLICE.to_vec();
    data.extend_from_slice(&10u64.to_le_bytes());

    let spoofed = Instruction {
        program_id: obligo_hook::ID,
        accounts: vec![
            AccountMeta::new_readonly(env.source, false),
            AccountMeta::new_readonly(env.mint, false),
            AccountMeta::new_readonly(env.destination, false),
            AccountMeta::new_readonly(env.alice.pubkey(), false),
            AccountMeta::new_readonly(eaml_address(&env.mint), false),
            AccountMeta::new(permit_address(&env.source), false),
        ],
        data,
    };

    let err = env
        .send(&[spoofed], &[])
        .expect_err("Execute is only valid inside a real Token-2022 transfer");

    assert_custom_error(err, E_NOT_TRANSFERRING);
    assert_eq!(
        env.permit().unwrap().amount,
        10,
        "a forged Execute must not consume the permit"
    );
}

#[test]
fn only_the_core_authority_pda_may_grant_a_permit() {
    let mut env = Env::new();
    let impostor = Keypair::new();
    env.svm.airdrop(&impostor.pubkey(), 10_000_000_000).unwrap();

    // A real signature from an account that is simply not the core's PDA.
    let ix = Instruction {
        program_id: obligo_hook::ID,
        accounts: obligo_hook::accounts::GrantPermit {
            payer: env.payer.pubkey(),
            core_authority: impostor.pubkey(),
            source_token: env.source,
            permit: permit_address(&env.source),
            system_program: SYSTEM_PROGRAM_ID,
        }
        .to_account_metas(None),
        data: obligo_hook::instruction::GrantPermit {
            kind: PERMIT_KIND_REDEEM,
            amount: 1_000,
        }
        .data(),
    };
    let impostor_signer = impostor.insecure_clone();
    let err = env
        .send(&[ix], &[&impostor_signer])
        .expect_err("only the core's [b\"authority\"] PDA may authorise movement");

    assert_custom_error(err, E_CONSTRAINT_SEEDS);
    assert!(env.permit().is_none(), "no permit was created");

    // Naming the right PDA does not help either: it is off-curve, so nobody outside the core
    // program can put a signature behind it.
    let ix = Instruction {
        program_id: obligo_hook::ID,
        accounts: vec![
            AccountMeta::new(env.payer.pubkey(), true),
            AccountMeta::new_readonly(core_authority(), false),
            AccountMeta::new_readonly(env.source, false),
            AccountMeta::new(permit_address(&env.source), false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        ],
        data: obligo_hook::instruction::GrantPermit {
            kind: PERMIT_KIND_REDEEM,
            amount: 1_000,
        }
        .data(),
    };
    let err = env
        .send(&[ix], &[])
        .expect_err("the core authority must actually sign");

    assert_custom_error(err, E_CONSTRAINT_SIGNER);
    assert!(env.permit().is_none(), "no permit was created");

    // And a transfer is still refused, which is the point of all of the above.
    let err = env.transfer(10).expect_err("points still cannot move");
    assert_custom_error(err, E_MOVEMENT_NOT_AUTHORIZED);
}
