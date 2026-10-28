//! # Create Escrow Instruction Handler
//!
//! Locks a payment amount into an Escrow PDA associated with a specific `order_id`.
//! The escrow enforces fund safety: no party can withdraw the locked funds until
//! delivery confirmation or timeout expiry.
//!
//! ## Requirements Fulfilled
//! - **Req 4.1**: Lock total payment amount in Escrow_PDA for the specific order.
//! - **Req 4.2**: Prevent any party from withdrawing while funds are locked.
//! - **Req 4.5**: Validate split configuration sums to exactly 10000 basis points.
//! - **Req 19.5**: Reject double-lock attempts (enforced by Anchor's `init`).
//!
//! ## Security
//! - Validates no recipient is the system program (zero address).
//! - Validates no duplicate recipients in split configuration.
//! - Timeout overflow protection via `checked_add`.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Transfer};

use crate::errors::PayRogenError;
use crate::state::{EscrowStatus, SplitConfig, MAX_SPLITS};
use crate::SplitEntry;

/// Handler for the `create_escrow` instruction.
pub fn handler(
    ctx: Context<crate::CreateEscrow>,
    order_id: [u8; 32],
    amount: u64,
    timeout_seconds: i64,
    splits: Vec<SplitEntry>,
) -> Result<()> {
    let clock = Clock::get()?;

    // ─── Step 1: Validate escrow amount ──────────────────────────────────────
    require!(amount > 0, PayRogenError::InsufficientBalance);

    // ─── Step 2: Validate timeout duration ───────────────────────────────────
    require!(timeout_seconds > 0, PayRogenError::EscrowTimeout);

    // ─── Step 3: Validate split configuration ────────────────────────────────
    require!(!splits.is_empty(), PayRogenError::InvalidSplitSum);
    require!(splits.len() <= MAX_SPLITS, PayRogenError::InvalidSplitSum);

    // Validate no zero-address recipients (security: prevent burning funds)
    for entry in splits.iter() {
        require!(
            entry.recipient != Pubkey::default(),
            PayRogenError::InvalidRecipient
        );
    }

    // Validate no duplicate recipients (prevent configuration errors)
    for i in 0..splits.len() {
        for j in (i + 1)..splits.len() {
            require!(
                splits[i].recipient != splits[j].recipient,
                PayRogenError::InvalidRecipient
            );
        }
    }

    // Sum must equal exactly 10000 basis points (100%)
    let total_bps: u32 = splits.iter().map(|s| s.basis_points as u32).sum();
    require!(total_bps == 10000, PayRogenError::InvalidSplitSum);

    // ─── Step 4: Initialize escrow account state ─────────────────────────────
    let escrow = &mut ctx.accounts.escrow;

    escrow.buyer = ctx.accounts.buyer.key();
    escrow.seller = ctx.accounts.seller.key();
    escrow.merchant = ctx.accounts.merchant.key();
    escrow.amount = amount;
    escrow.order_id = order_id;
    escrow.status = EscrowStatus::Locked;
    escrow.locked_at = clock.unix_timestamp;
    escrow.timeout_at = clock
        .unix_timestamp
        .checked_add(timeout_seconds)
        .ok_or(PayRogenError::EscrowTimeout)?;
    escrow.bump = ctx.bumps.escrow;

    // Store validated split configuration
    escrow.split_count = splits.len() as u8;

    let mut stored_splits = [SplitConfig {
        recipient: Pubkey::default(),
        basis_points: 0,
    }; MAX_SPLITS];

    for (i, entry) in splits.iter().enumerate() {
        stored_splits[i] = SplitConfig {
            recipient: entry.recipient,
            basis_points: entry.basis_points,
        };
    }
    escrow.splits = stored_splits;

    // ─── Step 5: Transfer SPL tokens to escrow ───────────────────────────────
    let transfer_ctx = CpiContext::new(
        ctx.accounts.token_program.key(),
        Transfer {
            from: ctx.accounts.buyer_token_account.to_account_info(),
            to: ctx.accounts.escrow_token_account.to_account_info(),
            authority: ctx.accounts.buyer.to_account_info(),
        },
    );
    token::transfer(transfer_ctx, amount)?;

    Ok(())
}
