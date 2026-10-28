//! # Refund Escrow Instruction Handler
//!
//! Returns the full escrowed amount back to the buyer's token account when:
//! - The escrow timeout has expired (auto-refund), OR
//! - The gateway authority initiates the refund (dispute resolution)
//!
//! After refund, the escrow PDA is closed and rent is returned to the buyer.
//!
//! ## Requirements Fulfilled
//!
//! - **Req 4.4**: Auto-refund when timeout_at expires without delivery confirmation.
//! - **Req 5.1**: Authority must be the transaction signer.
//! - **Req 5.6**: Atomic refund — full amount returned in single transfer.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Transfer};

use crate::errors::PayRogenError;
use crate::state::EscrowStatus;

/// Handler for the `refund_escrow` instruction.
///
/// Validates escrow state and authorization, then transfers the full escrowed
/// amount back to the buyer's token account. Closes the escrow PDA afterward.
///
/// # Security
/// - Buyer wallet is validated to match escrow.buyer (prevents rent theft).
/// - Two authorization paths: timeout expiry (permissionless) or gateway authority.
pub fn handler(ctx: Context<crate::RefundEscrow>) -> Result<()> {
    let escrow = &ctx.accounts.escrow;

    // ─── Validate buyer_wallet matches escrow.buyer ──────────────────────────
    // This prevents an attacker from specifying their own wallet to receive
    // the rent from the closed escrow account.
    require!(
        ctx.accounts.buyer_wallet.key() == escrow.buyer,
        PayRogenError::UnauthorizedSigner
    );

    // ─── Step 1: Validate escrow status ──────────────────────────────────────
    require!(
        escrow.status == EscrowStatus::Locked || escrow.status == EscrowStatus::Disputed,
        PayRogenError::EscrowNotLocked
    );

    // ─── Step 2: Validate authorization ──────────────────────────────────────
    // A) The escrow timeout has expired (anyone can trigger the refund)
    // B) The gateway authority is the signer (admin-initiated refund)
    let clock = Clock::get()?;
    let is_timed_out = clock.unix_timestamp >= escrow.timeout_at;
    let is_gateway_authority = ctx.accounts.authority.key() == ctx.accounts.gateway_config.authority;

    require!(
        is_timed_out || is_gateway_authority,
        PayRogenError::EscrowTimeout
    );

    // ─── Step 3: Transfer full amount back to buyer ──────────────────────────
    let escrow_buyer = escrow.buyer;
    let escrow_order_id = escrow.order_id;
    let escrow_bump = escrow.bump;
    let seeds: &[&[u8]] = &[
        b"escrow",
        escrow_buyer.as_ref(),
        escrow_order_id.as_ref(),
        &[escrow_bump],
    ];
    let signer_seeds: &[&[&[u8]]] = &[seeds];

    let amount = escrow.amount;

    let cpi_accounts = Transfer {
        from: ctx.accounts.escrow_token_account.to_account_info(),
        to: ctx.accounts.buyer_token_account.to_account_info(),
        authority: ctx.accounts.escrow.to_account_info(),
    };
    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.key(),
        cpi_accounts,
        signer_seeds,
    );

    token::transfer(cpi_ctx, amount)?;

    // ─── Step 4: Mark as refunded (account will be closed by Anchor's `close` constraint)
    let escrow = &mut ctx.accounts.escrow;
    escrow.status = EscrowStatus::Refunded;

    Ok(())
}
