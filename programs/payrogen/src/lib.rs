//! # PayRogen Payment Gateway Smart Contract
//!
//! A Solana program (built with Anchor 1.1.2) that enforces atomic payment splitting,
//! escrow logic, on-chain gateway fees, and wallet freezing for the PayRogen
//! non-custodial payment gateway.
//!
//! ## Instructions
//! - `initialize_gateway` — One-time setup; upgrade authority only
//! - `execute_split_payment` — Atomic multi-recipient SPL token distribution
//! - `execute_sol_split_payment` — Atomic multi-recipient SOL distribution
//! - `create_escrow` — Lock SPL funds in a PDA vault with split config and timeout
//! - `release_escrow` — Pay the stored split recipients (buyer, or authority on dispute)
//! - `refund_escrow` — Return funds to the buyer (after timeout, or authority on dispute)
//! - `raise_dispute` — Buyer or seller suspends the timeout pending resolution
//! - `freeze_wallet` — Block outgoing transfers through this program for 24h
//! - `update_gateway_fee` — Admin fee adjustment with 5% cap
//! - `propose_authority` / `accept_authority` — Two-step authority rotation
//! - `set_treasury` — Change the fee treasury
//!
//! ## Trust model
//! The gateway authority can freeze wallets (24h, this program only), change
//! the fee (max 5%) and treasury, and resolve *disputed* escrows. It cannot
//! move escrowed funds anywhere other than the split recipients fixed at
//! creation or back to the buyer. The program upgrade authority can replace
//! the program and should be a multisig.

use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

pub mod errors;
pub mod instructions;
pub mod state;
pub mod validation;

#[cfg(test)]
mod tests;

use errors::PayRogenError;
use state::*;

declare_id!("AeJVPDMxcSvn89F5kgiita5cndcQvamEnCSvY38HeYFg");

#[program]
pub mod payrogen {
    use super::*;

    /// Initialize the gateway configuration PDA. Must be called once before
    /// any other instruction can succeed. Sets the admin authority, treasury
    /// wallet, and initial gateway fee.
    ///
    /// Security: one-time (`init`), and only the program upgrade authority may call it.
    pub fn initialize_gateway(
        ctx: Context<InitializeGateway>,
        gateway_fee_bps: u16,
    ) -> Result<()> {
        instructions::initialize_gateway::handler(ctx, gateway_fee_bps)
    }

    /// Execute an atomic split payment distributing SPL tokens to multiple
    /// recipients according to basis point allocations that must sum to 10000.
    ///
    /// `remaining_accounts[i]` is the token account of `splits[i].recipient`
    /// (same mint as the payer's account). The treasury entry must carry at
    /// least the gateway fee.
    pub fn execute_split_payment<'info>(
        ctx: Context<'info, ExecuteSplitPayment<'info>>,
        amount: u64,
        splits: Vec<SplitEntry>,
    ) -> Result<()> {
        instructions::execute_split_payment::handler(ctx, amount, splits)
    }

    /// Execute an atomic split payment distributing native SOL to multiple
    /// recipients according to basis point allocations that must sum to 10000.
    ///
    /// `remaining_accounts[i]` must be `splits[i].recipient`. The treasury
    /// entry must carry at least the gateway fee.
    pub fn execute_sol_split_payment<'info>(
        ctx: Context<'info, ExecuteSolSplitPayment<'info>>,
        amount: u64,
        splits: Vec<SplitEntry>,
    ) -> Result<()> {
        instructions::execute_sol_split_payment::handler(ctx, amount, splits)
    }

    /// Lock SPL funds in a program-derived vault for a specific order.
    pub fn create_escrow(
        ctx: Context<CreateEscrow>,
        order_id: [u8; 32],
        amount: u64,
        timeout_seconds: i64,
        splits: Vec<SplitEntry>,
    ) -> Result<()> {
        instructions::create_escrow::handler(ctx, order_id, amount, timeout_seconds, splits)
    }

    /// Release escrowed funds to the stored split recipients.
    ///
    /// `remaining_accounts[i]` is the token account of the i-th stored
    /// recipient (escrow mint). Callable by the buyer (Locked or Disputed) or
    /// by the gateway authority (Disputed only). Closes the vault and escrow.
    pub fn release_escrow<'info>(ctx: Context<'info, ReleaseEscrow<'info>>) -> Result<()> {
        instructions::release_escrow::handler(ctx)
    }

    /// Refund escrowed funds to the buyer. Callable by anyone once a Locked
    /// escrow has timed out, or by the gateway authority when Disputed.
    /// Closes the vault and escrow.
    pub fn refund_escrow(ctx: Context<RefundEscrow>) -> Result<()> {
        instructions::refund_escrow::handler(ctx)
    }

    /// Mark a Locked escrow as Disputed (buyer or seller, before timeout).
    pub fn raise_dispute(ctx: Context<RaiseDispute>) -> Result<()> {
        instructions::raise_dispute::handler(ctx)
    }

    /// Freeze a wallet for 24 hours, blocking outgoing transfers through this
    /// program. Only the gateway authority can invoke this instruction.
    pub fn freeze_wallet(ctx: Context<FreezeWallet>) -> Result<()> {
        instructions::freeze_wallet::handler(ctx)
    }

    /// Update the gateway fee. Gateway authority only; max 500 bps (5%).
    pub fn update_gateway_fee(
        ctx: Context<UpdateGatewayFee>,
        new_fee_bps: u16,
    ) -> Result<()> {
        instructions::update_gateway_fee::handler(ctx, new_fee_bps)
    }

    /// Step 1 of authority rotation: current authority nominates `new_authority`.
    pub fn propose_authority(ctx: Context<ProposeAuthority>, new_authority: Pubkey) -> Result<()> {
        instructions::authority::propose(ctx, new_authority)
    }

    /// Step 2 of authority rotation: the nominee signs to take over.
    pub fn accept_authority(ctx: Context<AcceptAuthority>) -> Result<()> {
        instructions::authority::accept(ctx)
    }

    /// Change the treasury that must receive the gateway fee.
    pub fn set_treasury(ctx: Context<SetTreasury>, new_treasury: Pubkey) -> Result<()> {
        instructions::authority::set_treasury(ctx, new_treasury)
    }
}

// ─── Shared Types ────────────────────────────────────────────────────────────

/// Split entry representing a recipient and their share in basis points.
/// Used as an instruction argument for both direct payments and escrow creation.
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct SplitEntry {
    /// Recipient wallet address (for SPL: the owner of the recipient token account)
    pub recipient: Pubkey,
    /// Share in basis points (1 bp = 0.01%). All entries must sum to 10000.
    pub basis_points: u16,
}

// ─── Events ──────────────────────────────────────────────────────────────────

/// Emitted when an escrow is funded.
#[event]
pub struct EscrowCreated {
    pub escrow: Pubkey,
    pub buyer: Pubkey,
    pub seller: Pubkey,
    pub mint: Pubkey,
    pub amount: u64,
    pub order_id: [u8; 32],
    pub timeout_at: i64,
}

/// Emitted when an escrow is paid out to its split recipients.
#[event]
pub struct EscrowReleased {
    pub escrow: Pubkey,
    pub released_by: Pubkey,
    pub amount: u64,
}

/// Emitted when an escrow is refunded to the buyer.
#[event]
pub struct EscrowRefunded {
    pub escrow: Pubkey,
    pub refunded_by: Pubkey,
    pub amount: u64,
}

/// Emitted when a dispute is raised.
#[event]
pub struct DisputeRaised {
    pub escrow: Pubkey,
    pub raised_by: Pubkey,
}

/// Emitted when a wallet is frozen.
#[event]
pub struct WalletFrozenEvent {
    pub wallet: Pubkey,
    pub expires_at: i64,
}

/// Emitted when a new authority is nominated.
#[event]
pub struct AuthorityProposed {
    pub current_authority: Pubkey,
    pub new_authority: Pubkey,
}

/// Emitted whenever GatewayConfig changes (fee, treasury, authority).
#[event]
pub struct GatewayConfigUpdated {
    pub authority: Pubkey,
    pub treasury: Pubkey,
    pub gateway_fee_bps: u16,
}

// ─── Account Structs ─────────────────────────────────────────────────────────

/// Account struct for the `initialize_gateway` instruction.
/// Creates the singleton GatewayConfig PDA. Only the program's upgrade
/// authority may call it, which prevents front-running after deployment.
#[derive(Accounts)]
pub struct InitializeGateway<'info> {
    /// The program upgrade authority. Becomes the gateway admin.
    #[account(mut)]
    pub authority: Signer<'info>,

    /// Treasury wallet that will receive gateway fees.
    /// CHECK: Any non-default pubkey (checked in handler); stored in config.
    pub treasury: UncheckedAccount<'info>,

    /// Gateway configuration PDA — `init` ensures one-time creation.
    #[account(
        init,
        payer = authority,
        space = GatewayConfig::LEN,
        seeds = [GATEWAY_CONFIG_SEED],
        bump,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,

    /// This program's account; must point at `program_data`.
    #[account(
        constraint = program.programdata_address()? == Some(program_data.key())
            @ PayRogenError::UnauthorizedSigner,
    )]
    pub program: Program<'info, crate::program::Payrogen>,

    /// This program's ProgramData; its upgrade authority must be the signer.
    #[account(
        constraint = program_data.upgrade_authority_address == Some(authority.key())
            @ PayRogenError::UnauthorizedSigner,
    )]
    pub program_data: Account<'info, ProgramData>,

    /// System program for account creation.
    pub system_program: Program<'info, System>,
}

/// Account struct for the `execute_split_payment` instruction.
///
/// Recipient token accounts are passed via `remaining_accounts` — one per split entry.
/// The account list is part of the public client interface; do not reorder.
#[derive(Accounts)]
pub struct ExecuteSplitPayment<'info> {
    /// The payer authorizing the split payment. Must be the transaction signer.
    #[account(mut)]
    pub payer: Signer<'info>,

    /// The payer's SPL token account to transfer funds from. Its mint is the
    /// payment mint; every recipient account must use the same mint.
    #[account(
        mut,
        constraint = payer_token_account.owner == payer.key() @ PayRogenError::UnauthorizedSigner
    )]
    pub payer_token_account: Account<'info, TokenAccount>,

    /// Gateway configuration PDA (treasury and fee are enforced from it).
    #[account(
        seeds = [GATEWAY_CONFIG_SEED],
        bump = gateway_config.bump,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,

    /// WalletFreeze PDA for the payer (may be uninitialized).
    /// CHECK: Address enforced by seeds; contents validated by
    /// `validation::assert_not_frozen` (owner, discriminator, wallet, expiry).
    #[account(
        seeds = [FREEZE_SEED, payer.key().as_ref()],
        bump,
    )]
    pub freeze_account: UncheckedAccount<'info>,

    /// SPL Token program for executing CPI transfers.
    pub token_program: Program<'info, Token>,
}

/// Account struct for the `execute_sol_split_payment` instruction.
///
/// Recipient wallet accounts are passed via `remaining_accounts` — one per split entry.
/// The account list is part of the public client interface; do not reorder.
#[derive(Accounts)]
pub struct ExecuteSolSplitPayment<'info> {
    /// The payer authorizing the SOL split payment. Must be the transaction signer.
    #[account(mut)]
    pub payer: Signer<'info>,

    /// Gateway configuration PDA (treasury and fee are enforced from it).
    #[account(
        seeds = [GATEWAY_CONFIG_SEED],
        bump = gateway_config.bump,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,

    /// WalletFreeze PDA for the payer (may be uninitialized).
    /// CHECK: Address enforced by seeds; contents validated by
    /// `validation::assert_not_frozen`.
    #[account(
        seeds = [FREEZE_SEED, payer.key().as_ref()],
        bump,
    )]
    pub freeze_account: UncheckedAccount<'info>,

    /// System program for native SOL transfers.
    pub system_program: Program<'info, System>,
}

/// Account struct for the `create_escrow` instruction.
///
/// The escrow PDA is derived from `[b"escrow", buyer, order_id]`; the token
/// vault from `[b"vault", escrow]`. Both are created here.
#[derive(Accounts)]
#[instruction(order_id: [u8; 32])]
pub struct CreateEscrow<'info> {
    /// Buyer who funds the escrow (signer and payer for account rent)
    #[account(mut)]
    pub buyer: Signer<'info>,

    /// Seller; must be one of the split recipients (checked in handler).
    /// CHECK: Only the key is stored.
    pub seller: UncheckedAccount<'info>,

    /// Merchant platform facilitating the transaction (informational).
    /// CHECK: Only the key is stored.
    pub merchant: UncheckedAccount<'info>,

    /// SPL mint of the escrowed tokens.
    pub mint: Account<'info, Mint>,

    /// Escrow PDA account — `init` ensures double-lock prevention.
    #[account(
        init,
        payer = buyer,
        space = EscrowAccount::LEN,
        seeds = [ESCROW_SEED, buyer.key().as_ref(), order_id.as_ref()],
        bump,
    )]
    pub escrow: Account<'info, EscrowAccount>,

    /// Buyer's SPL token account (source of funds), same mint.
    #[account(
        mut,
        token::mint = mint,
        token::authority = buyer,
    )]
    pub buyer_token_account: Account<'info, TokenAccount>,

    /// Vault token account created by this instruction; authority = escrow PDA.
    #[account(
        init,
        payer = buyer,
        seeds = [VAULT_SEED, escrow.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = escrow,
        token::token_program = token_program,
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    /// WalletFreeze PDA for the buyer (may be uninitialized).
    /// CHECK: Address enforced by seeds; contents validated in handler.
    #[account(
        seeds = [FREEZE_SEED, buyer.key().as_ref()],
        bump,
    )]
    pub freeze_account: UncheckedAccount<'info>,

    /// Gateway configuration PDA (treasury and fee are enforced from it).
    #[account(
        seeds = [GATEWAY_CONFIG_SEED],
        bump = gateway_config.bump,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,

    /// SPL Token program.
    pub token_program: Program<'info, Token>,

    /// System program (required for PDA account creation via `init`).
    pub system_program: Program<'info, System>,
}

/// Account struct for the `release_escrow` instruction.
///
/// Recipient token accounts are passed via `remaining_accounts`, in stored
/// split order. The vault and escrow are closed; rent goes to the buyer.
#[derive(Accounts)]
pub struct ReleaseEscrow<'info> {
    /// The buyer (Locked or Disputed) or the gateway authority (Disputed only).
    pub authority: Signer<'info>,

    /// Buyer wallet; receives the escrow and vault rent.
    /// CHECK: Address must equal `escrow.buyer`.
    #[account(mut, address = escrow.buyer @ PayRogenError::UnauthorizedSigner)]
    pub buyer: UncheckedAccount<'info>,

    /// The escrow PDA — closed after successful release.
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.buyer.as_ref(), escrow.order_id.as_ref()],
        bump = escrow.bump,
        close = buyer,
    )]
    pub escrow: Account<'info, EscrowAccount>,

    /// The escrow's vault.
    #[account(
        mut,
        seeds = [VAULT_SEED, escrow.key().as_ref()],
        bump = escrow.vault_bump,
        token::mint = escrow.mint,
        token::authority = escrow,
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    /// WalletFreeze PDA for the buyer (checked for buyer-initiated release).
    /// CHECK: Address enforced by seeds; contents validated in handler.
    #[account(
        seeds = [FREEZE_SEED, escrow.buyer.as_ref()],
        bump,
    )]
    pub buyer_freeze_account: UncheckedAccount<'info>,

    /// Gateway configuration (authority for dispute resolution).
    #[account(
        seeds = [GATEWAY_CONFIG_SEED],
        bump = gateway_config.bump,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,

    /// SPL Token program.
    pub token_program: Program<'info, Token>,
}

/// Account struct for the `refund_escrow` instruction.
///
/// The vault and escrow are closed; rent goes to the buyer.
#[derive(Accounts)]
pub struct RefundEscrow<'info> {
    /// Anyone (Locked and timed out) or the gateway authority (Disputed).
    pub caller: Signer<'info>,

    /// The escrow PDA — closed after successful refund.
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.buyer.as_ref(), escrow.order_id.as_ref()],
        bump = escrow.bump,
        close = buyer_wallet,
    )]
    pub escrow: Account<'info, EscrowAccount>,

    /// The escrow's vault.
    #[account(
        mut,
        seeds = [VAULT_SEED, escrow.key().as_ref()],
        bump = escrow.vault_bump,
        token::mint = escrow.mint,
        token::authority = escrow,
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    /// Buyer's token account for the refund: owner = buyer, mint = escrow mint.
    #[account(
        mut,
        token::mint = escrow.mint,
        token::authority = escrow.buyer,
    )]
    pub buyer_token_account: Account<'info, TokenAccount>,

    /// Buyer wallet; receives the escrow and vault rent.
    /// CHECK: Address must equal `escrow.buyer`.
    #[account(mut, address = escrow.buyer @ PayRogenError::UnauthorizedSigner)]
    pub buyer_wallet: UncheckedAccount<'info>,

    /// Gateway configuration (authority for dispute resolution).
    #[account(
        seeds = [GATEWAY_CONFIG_SEED],
        bump = gateway_config.bump,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,

    /// SPL Token program.
    pub token_program: Program<'info, Token>,
}

/// Account struct for the `raise_dispute` instruction.
#[derive(Accounts)]
pub struct RaiseDispute<'info> {
    /// Buyer or seller of the escrow.
    pub caller: Signer<'info>,

    /// The escrow being disputed.
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.buyer.as_ref(), escrow.order_id.as_ref()],
        bump = escrow.bump,
    )]
    pub escrow: Account<'info, EscrowAccount>,
}

/// Account struct for the `freeze_wallet` instruction.
#[derive(Accounts)]
pub struct FreezeWallet<'info> {
    /// Gateway authority (Admin_MultiSig) that can invoke freeze
    #[account(mut)]
    pub authority: Signer<'info>,

    /// Gateway config — `has_one = authority` ensures only the admin can freeze
    #[account(
        seeds = [GATEWAY_CONFIG_SEED],
        bump = gateway_config.bump,
        has_one = authority,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,

    /// WalletFreeze PDA — `init_if_needed` allows re-freezing an existing account
    #[account(
        init_if_needed,
        payer = authority,
        space = WalletFreeze::LEN,
        seeds = [FREEZE_SEED, wallet.key().as_ref()],
        bump,
    )]
    pub freeze_account: Account<'info, WalletFreeze>,

    /// CHECK: The wallet to be frozen, validated by seed derivation
    pub wallet: UncheckedAccount<'info>,

    /// System program (required for potential account creation)
    pub system_program: Program<'info, System>,
}

/// Account struct for the `update_gateway_fee` instruction.
#[derive(Accounts)]
pub struct UpdateGatewayFee<'info> {
    /// Gateway authority (Admin_MultiSig) that can update fees
    #[account(mut)]
    pub authority: Signer<'info>,

    /// Gateway config PDA — `has_one = authority` enforces admin-only access
    #[account(
        mut,
        seeds = [GATEWAY_CONFIG_SEED],
        bump = gateway_config.bump,
        has_one = authority,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,
}

/// Account struct for the `propose_authority` instruction.
#[derive(Accounts)]
pub struct ProposeAuthority<'info> {
    /// Current gateway authority; pays rent for the pending account.
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [GATEWAY_CONFIG_SEED],
        bump = gateway_config.bump,
        has_one = authority,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,

    /// Pending nomination; overwritten by a new proposal.
    #[account(
        init_if_needed,
        payer = authority,
        space = PendingAuthority::LEN,
        seeds = [PENDING_AUTHORITY_SEED],
        bump,
    )]
    pub pending_authority: Account<'info, PendingAuthority>,

    pub system_program: Program<'info, System>,
}

/// Account struct for the `accept_authority` instruction.
#[derive(Accounts)]
pub struct AcceptAuthority<'info> {
    /// The nominated authority.
    pub new_authority: Signer<'info>,

    #[account(
        mut,
        seeds = [GATEWAY_CONFIG_SEED],
        bump = gateway_config.bump,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,

    /// Pending nomination; must name the signer. Closed on acceptance.
    #[account(
        mut,
        seeds = [PENDING_AUTHORITY_SEED],
        bump = pending_authority.bump,
        has_one = new_authority @ PayRogenError::UnauthorizedSigner,
        close = previous_authority,
    )]
    pub pending_authority: Account<'info, PendingAuthority>,

    /// Outgoing authority; receives the pending account's rent.
    /// CHECK: Address must equal the current `gateway_config.authority`.
    #[account(mut, address = gateway_config.authority @ PayRogenError::UnauthorizedSigner)]
    pub previous_authority: UncheckedAccount<'info>,
}

/// Account struct for the `set_treasury` instruction.
#[derive(Accounts)]
pub struct SetTreasury<'info> {
    /// Gateway authority.
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [GATEWAY_CONFIG_SEED],
        bump = gateway_config.bump,
        has_one = authority,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,
}
