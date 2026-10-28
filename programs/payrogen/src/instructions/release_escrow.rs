//! # Release Escrow Instruction Handler
//!
//! Distributes escrowed funds to recipients according to the stored split configuration.
//! The buyer must sign the transaction to confirm delivery. All transfers happen
//! atomically within a single Solana transaction — either all succeed or none do.
//!
//! After release, the escrow PDA is closed and rent is returned to the buyer.
//!
//! ## Requirements Fulfilled
//!
//! - **Req 4.3**: Release escrowed funds to recipients on delivery confirmation.
//! - **Req 5.1**: Buyer must be the transaction signer (delivery confirmation).
//! - **Req 5.6**: Atomic distribution — all transfers revert if any single one fails.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Transfer};

use crate::errors::PayRogenError;
use crate::state::EscrowStatus;

/// Handler for the `release_escrow` instruction.
///
/// Validates escrow state, then distributes funds from the escrow token account
/// to each recipient according to the stored split configuration. Closes escrow afterward.
pub fn handler<'info>(
    ctx: Context<'info, crate::ReleaseEscrow<'info>>,
) -> Result<()> {
    let escrow = &ctx.accounts.escrow;

    // ─── Step 1: Validate escrow status ──────────────────────────────────────
    require!(
        escrow.status == EscrowStatus::Locked || escrow.status == EscrowStatus::Disputed,
        PayRogenError::EscrowNotLocked
    );

    // ─── Step 2: Validate signer is the buyer ────────────────────────────────
    require!(
        ctx.accounts.buyer.key() == escrow.buyer,
        PayRogenError::UnauthorizedSigner
    );

    // ─── Step 3: Validate remaining_accounts matches split_count ─────────────
    let split_count = escrow.split_count as usize;
    require!(
        ctx.remaining_accounts.len() == split_count,
        PayRogenError::InvalidRecipient
    );

    // ─── Step 4: Distribute funds per split config ───────────────────────────
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

    for i in 0..split_count {
        let split = &escrow.splits[i];

        // Calculate transfer amount using u128 to prevent overflow
        let transfer_amount = (amount as u128)
            .checked_mul(split.basis_points as u128)
            .unwrap()
            .checked_div(10000)
            .unwrap() as u64;

        if transfer_amount == 0 {
            continue;
        }

        let recipient_token_account = &ctx.remaining_accounts[i];

        // Validate the recipient token account is writable
        require!(
            recipient_token_account.is_writable,
            PayRogenError::InvalidRecipient
        );

        let cpi_accounts = Transfer {
            from: ctx.accounts.escrow_token_account.to_account_info(),
            to: recipient_token_account.to_account_info(),
            authority: ctx.accounts.escrow.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            cpi_accounts,
            signer_seeds,
        );

        token::transfer(cpi_ctx, transfer_amount)?;
    }

    // ─── Step 5: Mark as released (account will be closed by Anchor's `close` constraint)
    let escrow = &mut ctx.accounts.escrow;
    escrow.status = EscrowStatus::Released;

    Ok(())
}
