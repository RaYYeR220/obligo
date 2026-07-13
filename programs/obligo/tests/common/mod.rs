//! Shared litesvm harness for the core program.
//!
//! Prerequisites (PowerShell, from the workspace root):
//!   cargo-build-sbf
//!   cargo test -p obligo

#![allow(dead_code)]

use anchor_lang::solana_program::program_pack::Pack;
use anchor_lang::{
    prelude::Pubkey, AccountDeserialize, AccountSerialize, InstructionData, ToAccountMetas,
};
use anchor_spl::token::spl_token;
use litesvm::LiteSVM;
use obligo::state::{Merchant, Protocol};
use solana_instruction::{error::InstructionError, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;
use std::path::PathBuf;

pub const TOKEN_PROGRAM_ID: Pubkey = anchor_spl::token::ID;
pub const TOKEN_2022_ID: Pubkey = anchor_spl::token_2022::ID;
pub const ATA_PROGRAM_ID: Pubkey = anchor_spl::associated_token::ID;
pub const SYSTEM_PROGRAM_ID: Pubkey = solana_system_interface::program::ID;

pub const USDC_DECIMALS: u8 = 6;
pub const DOLLAR: u64 = 1_000_000;

/// ObligoError, as Anchor emits it: 6000 + the variant's index.
pub const E_OVERFLOW: u32 = 6000;
pub const E_RESERVE_BREACHED: u32 = 6001;
pub const E_INVALID_TERMS: u32 = 6002;
pub const E_INVALID_AMOUNT: u32 = 6003;
pub const E_MERCHANT_DEFAULTED: u32 = 6004;
pub const E_NAME_TOO_LONG: u32 = 6005;
pub const E_INSUFFICIENT_COLLATERAL: u32 = 6006;
pub const E_TERMS_LOCKED: u32 = 6007;

/// Anchor's own constraint failures.
pub const E_CONSTRAINT_HAS_ONE: u32 = 2001;
pub const E_CONSTRAINT_SEEDS: u32 = 2006;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn read_program(relative: &str) -> Vec<u8> {
    let path = workspace_root().join(relative);
    std::fs::read(&path)
        .unwrap_or_else(|_| panic!("missing {}\nbuild it first: cargo-build-sbf", path.display()))
}

/// A registered merchant and everything a test needs to act as it.
pub struct MerchantHandle {
    pub authority: Keypair,
    pub merchant: Pubkey,
    pub vault: Pubkey,
    /// The merchant authority's own USDC account, pre-funded.
    pub usdc: Pubkey,
    pub points_mint: Pubkey,
}

pub struct Env {
    pub svm: LiteSVM,
    pub payer: Keypair,
    /// The protocol authority. It funds genesis and can change global params — nothing else.
    pub protocol_authority: Keypair,
    pub usdc_mint: Pubkey,
    pub protocol: Pubkey,
}

impl Env {
    /// Loads both programs, mints a USDC, and runs genesis.
    pub fn new() -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program(obligo::ID, &read_program("target/deploy/obligo.so"))
            .unwrap();
        svm.add_program(
            obligo_hook::ID,
            &read_program("target/deploy/obligo_hook.so"),
        )
        .unwrap();

        let payer = Keypair::new();
        let protocol_authority = Keypair::new();
        svm.airdrop(&payer.pubkey(), 1_000_000_000_000).unwrap();
        svm.airdrop(&protocol_authority.pubkey(), 1_000_000_000_000)
            .unwrap();

        let mut env = Env {
            svm,
            payer,
            protocol_authority,
            usdc_mint: Pubkey::default(),
            protocol: protocol_address(),
        };

        env.usdc_mint = env.create_usdc_mint();
        env.init_protocol();
        env
    }

    // ---- transactions -------------------------------------------------------------------

    pub fn send(
        &mut self,
        ixs: &[Instruction],
        extra_signers: &[&Keypair],
    ) -> Result<(), TransactionError> {
        let mut signers: Vec<&Keypair> = vec![&self.payer];
        signers.extend_from_slice(extra_signers);

        // Otherwise an identical transaction is rejected as a duplicate before the program ever
        // runs, and a test that expects a specific failure would pass for the wrong reason.
        self.svm.expire_blockhash();

        let message = Message::new(ixs, Some(&self.payer.pubkey()));
        let tx = Transaction::new(&signers, message, self.svm.latest_blockhash());

        self.svm
            .send_transaction(tx)
            .map(|_| ())
            .map_err(|failed| failed.err)
    }

    // ---- USDC ---------------------------------------------------------------------------

    fn create_usdc_mint(&mut self) -> Pubkey {
        let kp = Keypair::new();
        let rent = self
            .svm
            .minimum_balance_for_rent_exemption(spl_token::state::Mint::LEN);

        let create = solana_system_interface::instruction::create_account(
            &self.payer.pubkey(),
            &kp.pubkey(),
            rent,
            spl_token::state::Mint::LEN as u64,
            &TOKEN_PROGRAM_ID,
        );
        let init = spl_token::instruction::initialize_mint2(
            &TOKEN_PROGRAM_ID,
            &kp.pubkey(),
            &self.payer.pubkey(),
            None,
            USDC_DECIMALS,
        )
        .unwrap();

        self.send(&[create, init], &[&kp]).expect("usdc mint");
        kp.pubkey()
    }

    /// A plain USDC token account, funded with `amount`.
    pub fn usdc_account(&mut self, owner: &Pubkey, amount: u64) -> Pubkey {
        let kp = Keypair::new();
        let rent = self
            .svm
            .minimum_balance_for_rent_exemption(spl_token::state::Account::LEN);

        let create = solana_system_interface::instruction::create_account(
            &self.payer.pubkey(),
            &kp.pubkey(),
            rent,
            spl_token::state::Account::LEN as u64,
            &TOKEN_PROGRAM_ID,
        );
        let init = spl_token::instruction::initialize_account3(
            &TOKEN_PROGRAM_ID,
            &kp.pubkey(),
            &self.usdc_mint,
            owner,
        )
        .unwrap();
        self.send(&[create, init], &[&kp]).expect("usdc account");

        if amount > 0 {
            let mint_to = spl_token::instruction::mint_to(
                &TOKEN_PROGRAM_ID,
                &self.usdc_mint,
                &kp.pubkey(),
                &self.payer.pubkey(),
                &[],
                amount,
            )
            .unwrap();
            self.send(&[mint_to], &[]).expect("fund usdc");
        }
        kp.pubkey()
    }

    // ---- protocol -----------------------------------------------------------------------

    fn init_protocol(&mut self) {
        let ix = Instruction {
            program_id: obligo::ID,
            accounts: obligo::accounts::InitProtocol {
                authority: self.protocol_authority.pubkey(),
                protocol: protocol_address(),
                usdc_mint: self.usdc_mint,
                hook_program: obligo_hook::ID,
                protocol_authority: core_authority(),
                system_program: SYSTEM_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: obligo::instruction::InitProtocol {}.data(),
        };
        let authority = self.protocol_authority.insecure_clone();
        self.send(&[ix], &[&authority]).expect("init_protocol");
    }

    /// Registers a merchant and hands back everything needed to act as it.
    /// `usdc` is pre-funded with $10,000 so the test never has to think about it.
    pub fn register_merchant(
        &mut self,
        name: &str,
        usdc_per_point: u64,
        reserve_bps: u16,
        point_ttl: i64,
    ) -> MerchantHandle {
        let authority = Keypair::new();
        self.svm
            .airdrop(&authority.pubkey(), 1_000_000_000_000)
            .unwrap();

        self.try_register(&authority, name, usdc_per_point, reserve_bps, point_ttl)
            .expect("register_merchant");

        let merchant = merchant_address(&authority.pubkey());
        let usdc = self.usdc_account(&authority.pubkey(), 10_000 * DOLLAR);

        MerchantHandle {
            authority,
            merchant,
            vault: vault_address(&merchant),
            usdc,
            points_mint: points_mint_address(&merchant),
        }
    }

    /// The raw registration, for tests that expect it to be refused.
    pub fn try_register(
        &mut self,
        authority: &Keypair,
        name: &str,
        usdc_per_point: u64,
        reserve_bps: u16,
        point_ttl: i64,
    ) -> Result<(), TransactionError> {
        let merchant = merchant_address(&authority.pubkey());

        let ix = Instruction {
            program_id: obligo::ID,
            accounts: obligo::accounts::RegisterMerchant {
                authority: authority.pubkey(),
                protocol: protocol_address(),
                merchant,
                usdc_mint: self.usdc_mint,
                vault: vault_address(&merchant),
                token_program: TOKEN_PROGRAM_ID,
                system_program: SYSTEM_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: obligo::instruction::RegisterMerchant {
                name: name.to_string(),
                usdc_per_point,
                reserve_bps,
                point_ttl,
            }
            .data(),
        };
        let signer = authority.insecure_clone();
        self.send(&[ix], &[&signer])
    }

    /// Test surgery: put points on a merchant's books without going through `issue_points`.
    ///
    /// The registry and the vault have to enforce the reserve invariant against outstanding
    /// points, and they have to do it whether or not the issuance path exists. Poking the books
    /// directly keeps those tests honest about what they are actually testing.
    pub fn set_points_outstanding(&mut self, m: &MerchantHandle, points: u64) {
        let mut raw = self.svm.get_account(&m.merchant).unwrap();
        let mut state = Merchant::try_deserialize(&mut raw.data.as_slice()).unwrap();
        state.points_outstanding = points;
        state.total_issued = points;

        let mut buf = Vec::new();
        state.try_serialize(&mut buf).unwrap();
        raw.data[..buf.len()].copy_from_slice(&buf);

        self.svm.set_account(m.merchant, raw).unwrap();
    }

    // ---- collateral ---------------------------------------------------------------------

    pub fn deposit(&mut self, m: &MerchantHandle, amount: u64) -> Result<(), TransactionError> {
        let ix = Instruction {
            program_id: obligo::ID,
            accounts: obligo::accounts::DepositCollateral {
                depositor: m.authority.pubkey(),
                merchant: m.merchant,
                vault: m.vault,
                usdc_mint: self.usdc_mint,
                from: m.usdc,
                token_program: TOKEN_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: obligo::instruction::DepositCollateral { amount }.data(),
        };
        let signer = m.authority.insecure_clone();
        self.send(&[ix], &[&signer])
    }

    pub fn withdraw(&mut self, m: &MerchantHandle, amount: u64) -> Result<(), TransactionError> {
        self.withdraw_as(&m.authority.insecure_clone(), m.merchant, m.vault, m.usdc, amount)
    }

    /// Withdraw signed by an arbitrary key, naming an arbitrary merchant. The point of the
    /// separate helper is to be able to aim it at somebody else's merchant account.
    pub fn withdraw_as(
        &mut self,
        signer: &Keypair,
        merchant: Pubkey,
        vault: Pubkey,
        destination: Pubkey,
        amount: u64,
    ) -> Result<(), TransactionError> {
        let ix = Instruction {
            program_id: obligo::ID,
            accounts: obligo::accounts::WithdrawCollateral {
                authority: signer.pubkey(),
                merchant,
                vault,
                usdc_mint: self.usdc_mint,
                destination,
                token_program: TOKEN_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: obligo::instruction::WithdrawCollateral { amount }.data(),
        };
        let signer = signer.insecure_clone();
        self.send(&[ix], &[&signer])
    }

    pub fn set_terms(
        &mut self,
        m: &MerchantHandle,
        usdc_per_point: u64,
        reserve_bps: u16,
        point_ttl: i64,
    ) -> Result<(), TransactionError> {
        let ix = Instruction {
            program_id: obligo::ID,
            accounts: obligo::accounts::SetTerms {
                authority: m.authority.pubkey(),
                merchant: m.merchant,
            }
            .to_account_metas(None),
            data: obligo::instruction::SetTerms {
                usdc_per_point,
                reserve_bps,
                point_ttl,
            }
            .data(),
        };
        let signer = m.authority.insecure_clone();
        self.send(&[ix], &[&signer])
    }

    // ---- points -------------------------------------------------------------------------

    pub fn create_points_mint(
        &mut self,
        m: &MerchantHandle,
        name: &str,
        symbol: &str,
        uri: &str,
    ) -> Result<(), TransactionError> {
        let ix = Instruction {
            program_id: obligo::ID,
            accounts: obligo::accounts::CreatePointsMint {
                authority: m.authority.pubkey(),
                protocol: protocol_address(),
                merchant: m.merchant,
                points_mint: m.points_mint,
                extra_account_meta_list: eaml_address(&m.points_mint),
                hook_program: obligo_hook::ID,
                token_program: TOKEN_2022_ID,
                system_program: SYSTEM_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: obligo::instruction::CreatePointsMint {
                name: name.to_string(),
                symbol: symbol.to_string(),
                uri: uri.to_string(),
            }
            .data(),
        };
        let signer = m.authority.insecure_clone();
        self.send(&[ix], &[&signer])
    }

    /// A merchant with a live points mint and collateral already posted.
    pub fn issuer(
        &mut self,
        name: &str,
        usdc_per_point: u64,
        reserve_bps: u16,
        collateral: u64,
    ) -> MerchantHandle {
        let m = self.register_merchant(name, usdc_per_point, reserve_bps, 86_400);
        self.create_points_mint(&m, name, "PTS", "https://example.invalid/points.json")
            .expect("create_points_mint");
        if collateral > 0 {
            self.deposit(&m, collateral).expect("deposit");
        }
        m
    }

    // ---- reads --------------------------------------------------------------------------

    pub fn protocol_state(&self) -> Protocol {
        let raw = self.svm.get_account(&protocol_address()).unwrap();
        Protocol::try_deserialize(&mut raw.data.as_slice()).unwrap()
    }

    pub fn merchant_state(&self, m: &MerchantHandle) -> Merchant {
        let raw = self.svm.get_account(&m.merchant).unwrap();
        Merchant::try_deserialize(&mut raw.data.as_slice()).unwrap()
    }

    /// Works for both token programs: `amount` is a u64 at offset 64 in either layout.
    pub fn token_balance(&self, account: &Pubkey) -> u64 {
        let raw = self.svm.get_account(account).unwrap();
        u64::from_le_bytes(raw.data[64..72].try_into().unwrap())
    }
}

// ---- addresses --------------------------------------------------------------------------

pub fn protocol_address() -> Pubkey {
    Pubkey::find_program_address(&[obligo::PROTOCOL_SEED], &obligo::ID).0
}

pub fn core_authority() -> Pubkey {
    Pubkey::find_program_address(&[obligo::AUTHORITY_SEED], &obligo::ID).0
}

pub fn merchant_address(authority: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[obligo::MERCHANT_SEED, authority.as_ref()], &obligo::ID).0
}

pub fn vault_address(merchant: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[obligo::VAULT_SEED, merchant.as_ref()], &obligo::ID).0
}

pub fn points_mint_address(merchant: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[obligo::POINTS_SEED, merchant.as_ref()], &obligo::ID).0
}

pub fn batch_address(merchant: &Pubkey, customer: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[obligo::BATCH_SEED, merchant.as_ref(), customer.as_ref()],
        &obligo::ID,
    )
    .0
}

pub fn eaml_address(mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"extra-account-metas", mint.as_ref()], &obligo_hook::ID).0
}

#[track_caller]
pub fn assert_custom_error(err: TransactionError, expected: u32) {
    match err {
        TransactionError::InstructionError(_, InstructionError::Custom(code)) => {
            assert_eq!(code, expected, "expected custom error {expected}, got {code}");
        }
        other => panic!("expected custom error {expected}, got {other:?}"),
    }
}
