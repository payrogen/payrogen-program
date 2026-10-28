//! # PayRogen Payment Gateway Smart Contract
//!
//! A Solana program (built with Anchor 1.1.2) that enforces atomic payment splitting,
//! escrow logic, dynamic gateway fees, and wallet freezing for the PayRogen
//! zero-knowledge, zero-custody payment gateway.
//!
//! ## Instructions
//! - `initialize_gateway` — One-time gateway configuration setup
//! - `execute_split_payment` — Atomic multi-recipient token distribution
//! - `create_escrow` — Lock funds in PDA with split config and timeout
//! - `release_escrow` — Distribute escrowed funds per split config
//! - `refund_escrow` — Return escrowed funds to buyer
//! - `freeze_wallet` — Block outgoing transfers for 24h
//! - `update_gateway_fee` — Admin fee adjustment with cap

use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

pub mod errors;
pub mod instructions;
pub mod state;

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
    /// Security: Can only be called once (Anchor `init` prevents re-initialization).
    pub fn initialize_gateway(
        ctx: Context<InitializeGateway>,
        gateway_fee_bps: u16,
    ) -> Result<()> {
        instructions::initialize_gateway::handler(ctx, gateway_fee_bps)
    }

    /// Execute an atomic split payment distributing funds to multiple recipients
    /// according to basis point allocations that must sum to 10000.
    ///
    /// Recipient token accounts are passed via `remaining_accounts` (one per split entry).
    /// The transaction reverts atomically if any individual transfer fails.
    pub fn execute_split_payment<'info>(
        ctx: Context<'info, ExecuteSplitPayment<'info>>,
        amount: u64,
        splits: Vec<SplitEntry>,
    ) -> Result<()> {
        instructions::execute_split_payment::handler(ctx, amount, splits)
    }

    /// Execute an atomic split payment distributing native SOL to multiple recipients
    /// according to basis point allocations that must sum to 10000.
    ///
    /// Recipient wallets are passed via `remaining_accounts` (one per split entry).
    /// The transaction reverts atomically if any individual transfer fails.
    pub fn execute_sol_split_payment<'info>(
        ctx: Context<'info, ExecuteSolSplitPayment<'info>>,
        amount: u64,
        splits: Vec<SplitEntry>,
    ) -> Result<()> {
        instructions::execute_sol_split_payment::handler(ctx, amount, splits)
    }

    /// Lock funds in an escrow PDA for a specific order, preventing any party
    /// from withdrawing until delivery confirmation or timeout.
    ///
    /// Validates split configuration sums to 10000 basis points before locking (Req 4.5).
    /// Double-lock prevention is enforced by Anchor's `init` constraint (Req 19.5).
    pub fn create_escrow(
        ctx: Context<CreateEscrow>,
        order_id: [u8; 32],
        amount: u64,
        timeout_seconds: i64,
        splits: Vec<SplitEntry>,
    ) -> Result<()> {
        instructions::create_escrow::handler(ctx, order_id, amount, timeout_seconds, splits)
    }

    /// Release escrowed funds to the seller and distribute according to the
    /// configured split (gateway fee + merchant fee + seller payout).
    ///
    /// Recipient token accounts are passed via `remaining_accounts` (one per split entry).
    /// The buyer must sign to confirm delivery. All transfers are atomic.
    /// The escrow account is closed after release to reclaim rent.
    pub fn release_escrow<'info>(
        ctx: Context<'info, ReleaseEscrow<'info>>,
    ) -> Result<()> {
        instructions::release_escrow::handler(ctx)
    }

    /// Refund escrowed funds back to the buyer when timeout expires or
    /// the gateway authority initiates a refund (dispute resolution).
    /// The escrow account is closed after refund to reclaim rent.
    pub fn refund_escrow(ctx: Context<RefundEscrow>) -> Result<()> {
        instructions::refund_escrow::handler(ctx)
    }

    /// Freeze a wallet for 24 hours, blocking all outgoing transfers.
    /// Only the gateway program authority can invoke this instruction.
    pub fn freeze_wallet(ctx: Context<FreezeWallet>) -> Result<()> {
        instructions::freeze_wallet::handler(ctx)
    }

    /// Update the gateway fee parameter. Requires Admin_MultiSig authorization.
    /// Fee is constrained to a maximum of 500 basis points (5%).
    pub fn update_gateway_fee(
        ctx: Context<UpdateGatewayFee>,
        new_fee_bps: u16,
    ) -> Result<()> {
        instructions::update_gateway_fee::handler(ctx, new_fee_bps)
    }
}

// ─── Shared Types ────────────────────────────────────────────────────────────

/// Split entry representing a recipient and their share in basis points.
/// Used as an instruction argument for both direct payments and escrow creation.
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct SplitEntry {
    /// Recipient wallet address that will receive this portion of the payment
    pub recipient: Pubkey,
    /// Share in basis points (1 bp = 0.01%). All entries must sum to 10000.
    pub basis_points: u16,
}

// ─── Account Structs ─────────────────────────────────────────────────────────

/// Account struct for the `initialize_gateway` instruction.
/// Creates the singleton GatewayConfig PDA. Can only be called once.
#[derive(Accounts)]
pub struct InitializeGateway<'info> {
    /// The deployer/authority who initializes the gateway. Becomes the admin.
    #[account(mut)]
    pub authority: Signer<'info>,

    /// Treasury wallet that will receive gateway fees.
    /// CHECK: Any valid pubkey; stored in config for fee distribution.
    pub treasury: UncheckedAccount<'info>,

    /// Gateway configuration PDA — `init` ensures one-time creation.
    #[account(
        init,
        payer = authority,
        space = GatewayConfig::LEN,
        seeds = [b"gateway_config"],
        bump,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,

    /// System program for account creation.
    pub system_program: Program<'info, System>,
}

/// Account struct for the `execute_split_payment` instruction.
///
/// Recipient token accounts are passed via `remaining_accounts` — one per split entry.
#[derive(Accounts)]
pub struct ExecuteSplitPayment<'info> {
    /// The payer authorizing the split payment. Must be the transaction signer.
    #[account(mut)]
    pub payer: Signer<'info>,

    /// The payer's SPL token account to transfer funds from.
    #[account(
        mut,
        constraint = payer_token_account.owner == payer.key() @ PayRogenError::UnauthorizedSigner
    )]
    pub payer_token_account: Account<'info, TokenAccount>,

    /// Gateway configuration PDA storing fee parameters and authority.
    #[account(
        seeds = [b"gateway_config"],
        bump = gateway_config.bump,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,

    /// Optional WalletFreeze PDA for the payer.
    /// If this account exists and the freeze has not expired, the transaction is rejected.
    /// CHECK: Validated manually in the handler — if account has data, it's deserialized
    /// and checked for freeze expiry. If empty/non-existent, payment proceeds.
    #[account(
        seeds = [b"freeze", payer.key().as_ref()],
        bump,
    )]
    pub freeze_account: UncheckedAccount<'info>,

    /// SPL Token program for executing CPI transfers.
    pub token_program: Program<'info, Token>,
}

/// Account struct for the `execute_sol_split_payment` instruction.
///
/// Recipient wallet accounts are passed via `remaining_accounts` — one per split entry.
#[derive(Accounts)]
pub struct ExecuteSolSplitPayment<'info> {
    /// The payer authorizing the SOL split payment. Must be the transaction signer.
    #[account(mut)]
    pub payer: Signer<'info>,

    /// Gateway configuration PDA storing fee parameters and authority.
    #[account(
        seeds = [b"gateway_config"],
        bump = gateway_config.bump,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,

    /// Optional WalletFreeze PDA for the payer.
    /// If this account exists and the freeze has not expired, the transaction is rejected.
    /// CHECK: Validated manually in the handler.
    #[account(
        seeds = [b"freeze", payer.key().as_ref()],
        bump,
    )]
    pub freeze_account: UncheckedAccount<'info>,

    /// System program for native SOL transfers.
    pub system_program: Program<'info, System>,
}

// ─── CreateEscrow Account Struct ─────────────────────────────────────────────

/// Account struct for the `create_escrow` instruction.
///
/// The escrow PDA is derived from `[b"escrow", buyer.key(), order_id]`.
/// Using `init` ensures double-lock prevention (Req 19.5).
#[derive(Accounts)]
#[instruction(order_id: [u8; 32])]
pub struct CreateEscrow<'info> {
    /// Buyer who funds the escrow (signer and payer for account rent)
    #[account(mut)]
    pub buyer: Signer<'info>,

    /// Seller who will receive funds on release.
    /// CHECK: Validated by being stored in escrow state; address verified during release.
    pub seller: UncheckedAccount<'info>,

    /// Merchant platform facilitating the transaction.
    /// CHECK: Validated by being stored in escrow state; used for split distribution on release.
    pub merchant: UncheckedAccount<'info>,

    /// Escrow PDA account — `init` ensures double-lock prevention.
    #[account(
        init,
        payer = buyer,
        space = EscrowAccount::LEN,
        seeds = [b"escrow", buyer.key().as_ref(), order_id.as_ref()],
        bump,
    )]
    pub escrow: Account<'info, EscrowAccount>,

    /// Buyer's SPL token account (source of funds to be locked).
    #[account(
        mut,
        constraint = buyer_token_account.owner == buyer.key() @ PayRogenError::UnauthorizedSigner,
    )]
    pub buyer_token_account: Account<'info, TokenAccount>,

    /// Escrow's SPL token account (destination to hold locked funds).
    #[account(mut)]
    pub escrow_token_account: Account<'info, TokenAccount>,

    /// Gateway configuration PDA (validates program is initialized).
    #[account(
        seeds = [b"gateway_config"],
        bump = gateway_config.bump,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,

    /// SPL Token program for executing the lock transfer.
    pub token_program: Program<'info, Token>,

    /// System program (required for PDA account creation via `init`).
    pub system_program: Program<'info, System>,
}

/// Account struct for the `release_escrow` instruction.
///
/// The buyer signs to confirm delivery. Recipient token accounts are passed
/// via `remaining_accounts`. Escrow account is closed after release.
#[derive(Accounts)]
pub struct ReleaseEscrow<'info> {
    /// Buyer confirming delivery (must match escrow.buyer). Receives rent back.
    #[account(mut)]
    pub buyer: Signer<'info>,

    /// The escrow PDA — closed after successful release, rent returned to buyer.
    #[account(
        mut,
        seeds = [b"escrow", escrow.buyer.as_ref(), escrow.order_id.as_ref()],
        bump = escrow.bump,
        close = buyer,
    )]
    pub escrow: Account<'info, EscrowAccount>,

    /// Escrow's SPL token account holding the locked funds.
    #[account(mut)]
    pub escrow_token_account: Account<'info, TokenAccount>,

    /// Gateway configuration (available for fee reference)
    #[account(
        seeds = [b"gateway_config"],
        bump = gateway_config.bump,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,

    /// SPL Token program for executing CPI transfers
    pub token_program: Program<'info, Token>,
}

/// Account struct for the `refund_escrow` instruction.
///
/// Escrow account is closed after refund, rent returned to buyer.
#[derive(Accounts)]
pub struct RefundEscrow<'info> {
    /// Authority initiating the refund (anyone if timed out, or gateway authority)
    #[account(mut)]
    pub authority: Signer<'info>,

    /// The escrow PDA — closed after successful refund.
    #[account(
        mut,
        seeds = [b"escrow", escrow.buyer.as_ref(), escrow.order_id.as_ref()],
        bump = escrow.bump,
        close = buyer_wallet,
    )]
    pub escrow: Account<'info, EscrowAccount>,

    /// Escrow's SPL token account holding the locked funds
    #[account(mut)]
    pub escrow_token_account: Account<'info, TokenAccount>,

    /// Buyer's SPL token account to receive the refund
    #[account(mut)]
    pub buyer_token_account: Account<'info, TokenAccount>,

    /// Buyer's wallet to receive rent from closed escrow account.
    /// CHECK: Must match escrow.buyer — validated in handler.
    #[account(mut)]
    pub buyer_wallet: UncheckedAccount<'info>,

    /// Gateway configuration (for authority verification)
    #[account(
        seeds = [b"gateway_config"],
        bump = gateway_config.bump,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,

    /// SPL Token program for executing CPI transfers
    pub token_program: Program<'info, Token>,
}

/// Account struct for the `freeze_wallet` instruction.
#[derive(Accounts)]
pub struct FreezeWallet<'info> {
    /// Gateway authority (Admin_MultiSig) that can invoke freeze
    #[account(mut)]
    pub authority: Signer<'info>,

    /// Gateway config — `has_one = authority` ensures only the admin can freeze
    #[account(
        seeds = [b"gateway_config"],
        bump = gateway_config.bump,
        has_one = authority,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,

    /// WalletFreeze PDA — `init_if_needed` allows re-freezing an expired account
    #[account(
        init_if_needed,
        payer = authority,
        space = WalletFreeze::LEN,
        seeds = [b"freeze", wallet.key().as_ref()],
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
        seeds = [b"gateway_config"],
        bump = gateway_config.bump,
        has_one = authority,
    )]
    pub gateway_config: Account<'info, GatewayConfig>,
}
