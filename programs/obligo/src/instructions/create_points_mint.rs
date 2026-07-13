use anchor_lang::prelude::*;
use anchor_lang::system_program::{create_account, CreateAccount};
use anchor_spl::token_2022::spl_token_2022::{extension::ExtensionType, state::Mint as MintState};
use anchor_spl::token_2022::{initialize_mint2, InitializeMint2, Token2022};
use anchor_spl::token_2022_extensions::{
    metadata_pointer_initialize, permanent_delegate_initialize,
    spl_pod::optional_keys::OptionalNonZeroPubkey,
    spl_token_metadata_interface::state::TokenMetadata, token_metadata_initialize,
    transfer_hook_initialize, MetadataPointerInitialize, PermanentDelegateInitialize,
    TokenMetadataInitialize, TransferHookInitialize,
};

use crate::constants::{
    MAX_NAME_LEN, MAX_SYMBOL_LEN, MAX_URI_LEN, MERCHANT_SEED, POINTS_SEED, POINT_DECIMALS,
    PROTOCOL_SEED,
};
use crate::error::ObligoError;
use crate::hook_cpi;
use crate::state::{Merchant, Protocol};

#[derive(Accounts)]
pub struct CreatePointsMint<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Account<'info, Protocol>,

    #[account(
        mut,
        seeds = [MERCHANT_SEED, authority.key().as_ref()],
        bump = merchant.bump,
        has_one = authority,
    )]
    pub merchant: Account<'info, Merchant>,

    /// CHECK: created below as a Token-2022 mint. Its address is pinned by seeds, which is what
    /// lets anybody derive a merchant's points mint without being told.
    #[account(mut, seeds = [POINTS_SEED, merchant.key().as_ref()], bump)]
    pub points_mint: UncheckedAccount<'info>,

    /// CHECK: written by the hook program, which pins the address to its own
    /// `[b"extra-account-metas", mint]` seeds.
    #[account(mut)]
    pub extra_account_meta_list: UncheckedAccount<'info>,

    /// CHECK: the hook the protocol was born with, and the only one a points mint will ever
    /// answer to. Fixed at genesis; there is no instruction that changes `protocol.hook_program`.
    #[account(executable, address = protocol.hook_program)]
    pub hook_program: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token2022>,
    pub system_program: Program<'info, System>,
}

/// Create the merchant's points mint.
///
/// The extension order below is load-bearing, not stylistic. Token-2022 writes extensions into
/// the TLV region *before* the base mint is initialised, and `initialize_mint2` closes the
/// account to further extension writes. `token_metadata_initialize` then has to come last of
/// all, because it demands a mint authority to check a signature against.
///
/// Two things are deliberately absent:
///
/// - **A transfer-hook authority.** It is set to `None` here and can never be set again. An
///   authority could repoint the mint at a hook that waves every transfer through, and every
///   point in circulation would silently stop being a collateral-backed liability. This is the
///   single largest rug vector in a hooked token, and we close it by giving it away.
/// - **`NonTransferable`.** Combined with `TransferHook` it is a dead extension pair: the mint
///   initialises, then every transfer fails with error 37 and the hook never runs. Points are
///   non-transferable because the hook says so — which is strictly better, because the hook can
///   also say "except into a redemption, right now, for exactly this many".
///
/// And one is deliberately present, and needs defending:
///
/// - **`PermanentDelegate`, set to the merchant PDA.** This is the extension everybody warns you
///   about, and rightly: a permanent delegate can move any holder's tokens without their signature.
///   That is exactly what `expire_points` needs — a lapsed point has to be burned out of a
///   customer's account by a crank the customer will never sign, or expiry is not a rule, it is a
///   request. Every alternative is worse: `approve` needs the customer's signature at issuance
///   (and they can `revoke` it the next block), and "expire" that only edits the books while the
///   tokens sit in the wallet is a lie the mint's own supply would contradict.
///
///   What makes it safe is *who* the delegate is. It is not the merchant's keypair — it is the
///   merchant **PDA**, which no private key in the world can sign for. Only this program can, and
///   this program will use it as a transfer authority in exactly one instruction, `expire_points`,
///   behind `now >= batch.issued_at + point_ttl`. There is no instruction here that lets a
///   merchant reach a customer's points before that. And even then, the movement still goes through
///   the hook and still needs a permit: the permanent delegate has no more right to move a point
///   without the clearing house's say-so than the customer does.
pub(crate) fn handler(
    ctx: Context<CreatePointsMint>,
    name: String,
    symbol: String,
    uri: String,
) -> Result<()> {
    require!(
        ctx.accounts.merchant.points_mint == Pubkey::default(),
        ObligoError::MintAlreadyExists
    );
    require!(
        name.len() <= MAX_NAME_LEN && symbol.len() <= MAX_SYMBOL_LEN && uri.len() <= MAX_URI_LEN,
        ObligoError::MetadataTooLong
    );

    let merchant_key = ctx.accounts.merchant.key();
    let mint_key = ctx.accounts.points_mint.key();
    let mint_bump = ctx.bumps.points_mint;

    let mint_seeds: &[&[u8]] = &[POINTS_SEED, merchant_key.as_ref(), &[mint_bump]];

    let authority_key = ctx.accounts.authority.key();
    let merchant_bump = ctx.accounts.merchant.bump;
    let merchant_seeds: &[&[u8]] = &[MERCHANT_SEED, authority_key.as_ref(), &[merchant_bump]];

    // The fixed-size extensions determine the account's length. TokenMetadata is variable-length
    // and Token-2022 reallocs the mint to fit it — but it will not fund that realloc, so the
    // account is created with the lamports for both and the space for one.
    let mint_space = ExtensionType::try_calculate_account_len::<MintState>(&[
        ExtensionType::TransferHook,
        ExtensionType::MetadataPointer,
        ExtensionType::PermanentDelegate,
    ])
    .map_err(|_| ObligoError::Overflow)?;

    let metadata = TokenMetadata {
        update_authority: OptionalNonZeroPubkey::try_from(Some(merchant_key))?,
        mint: mint_key,
        name: name.clone(),
        symbol: symbol.clone(),
        uri: uri.clone(),
        additional_metadata: vec![],
    };
    let metadata_space = metadata.tlv_size_of().map_err(|_| ObligoError::Overflow)?;

    let lamports = Rent::get()?.minimum_balance(
        mint_space
            .checked_add(metadata_space)
            .ok_or(ObligoError::Overflow)?,
    );

    create_account(
        CpiContext::new_with_signer(
            ctx.accounts.system_program.key(),
            CreateAccount {
                from: ctx.accounts.authority.to_account_info(),
                to: ctx.accounts.points_mint.to_account_info(),
            },
            &[mint_seeds],
        ),
        lamports,
        mint_space as u64,
        &ctx.accounts.token_program.key(),
    )?;

    transfer_hook_initialize(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            TransferHookInitialize {
                token_program_id: ctx.accounts.token_program.to_account_info(),
                mint: ctx.accounts.points_mint.to_account_info(),
            },
        ),
        None,
        Some(ctx.accounts.hook_program.key()),
    )?;

    metadata_pointer_initialize(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            MetadataPointerInitialize {
                token_program_id: ctx.accounts.token_program.to_account_info(),
                mint: ctx.accounts.points_mint.to_account_info(),
            },
        ),
        None,
        Some(mint_key),
    )?;

    // The merchant PDA, and nothing else, so that `expire_points` has a transfer authority for a
    // customer who will never sign for their own lapsed points. See the note above the handler.
    permanent_delegate_initialize(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            PermanentDelegateInitialize {
                token_program_id: ctx.accounts.token_program.to_account_info(),
                mint: ctx.accounts.points_mint.to_account_info(),
            },
        ),
        &merchant_key,
    )?;

    initialize_mint2(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            InitializeMint2 {
                mint: ctx.accounts.points_mint.to_account_info(),
            },
        ),
        POINT_DECIMALS,
        &merchant_key,
        // No freeze authority. A merchant that could freeze a customer's points would be a
        // custodian of them.
        None,
    )?;

    token_metadata_initialize(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            TokenMetadataInitialize {
                program_id: ctx.accounts.token_program.to_account_info(),
                metadata: ctx.accounts.points_mint.to_account_info(),
                update_authority: ctx.accounts.merchant.to_account_info(),
                mint_authority: ctx.accounts.merchant.to_account_info(),
                mint: ctx.accounts.points_mint.to_account_info(),
            },
            &[merchant_seeds],
        ),
        name,
        symbol,
        uri,
    )?;

    // Same instruction, no exceptions: a points mint whose ExtraAccountMetaList does not exist
    // yet is a mint whose points cannot move and whose EAML the next caller gets to write.
    hook_cpi::initialize_extra_account_meta_list(
        &ctx.accounts.hook_program.to_account_info(),
        &ctx.accounts.authority.to_account_info(),
        &ctx.accounts.points_mint.to_account_info(),
        &ctx.accounts.extra_account_meta_list.to_account_info(),
        &ctx.accounts.system_program.to_account_info(),
        &[],
    )?;

    let merchant = &mut ctx.accounts.merchant;
    merchant.points_mint = mint_key;
    merchant.mint_bump = mint_bump;

    Ok(())
}
