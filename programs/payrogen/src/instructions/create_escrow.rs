//! # Create Escrow Instruction Handler
//!
//! Locks a payment amount into a program-derived token vault associated with a
//! specific `order_id`. No party can withdraw the locked funds until delivery
//! confirmation, timeout expiry, or dispute resolution.
//!
//! ## Security
//! - The vault is created by this instruction at `[b"vault", escrow]` with the
//!   escrow PDA as its token authority and the escrow's `mint`, so funds can
//!   only leave through `release_escrow` / `refund_escrow`.
//! - The buyer's source account must hold the same mint.
//! - The mint is recorded in escrow state; every payout is checked against it.
//! - Splits: no zero address, no duplicates, non-zero shares, sum == 10000,
//!   treasury share >= gateway fee, and the seller must be a recipient.
//! - Timeout is bounded to (0, 90 days].
//! - Frozen buyers are rejected.
//! - Double-lock prevention via Anchor `init` on the escrow PDA.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Transfer};

use crate::errors::PayRogenError;
use crate::state::{EscrowStatus, SplitConfig, MAX_ESCROW_TIMEOUT_SECONDS, MAX_SPLITS};
use crate::validation::{assert_not_frozen, validate_splits};
use crate::{EscrowCreated, SplitEntry};

/// Handler for the `create_escrow` instruction.
pub fn handler(
    ctx: Context<crate::CreateEscrow>,
    order_id: [u8; 32],
    amount: u64,
    timeout_seconds: i64,
    splits: Vec<SplitEntry>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    // ─── Input validation ────────────────────────────────────────────────────
    require!(amount > 0, PayRogenError::InvalidAmount);
    require!(
        timeout_seconds > 0 && timeout_seconds <= MAX_ESCROW_TIMEOUT_SECONDS,
        PayRogenError::InvalidTimeout
    );

    let config = &ctx.accounts.gateway_config;
    validate_splits(&splits, &config.treasury, config.gateway_fee_bps, false)?;

    let seller = ctx.accounts.seller.key();
    let merchant = ctx.accounts.merchant.key();
    require_keys_neq!(seller, Pubkey::default(), PayRogenError::InvalidRecipient);
    require_keys_neq!(merchant, Pubkey::default(), PayRogenError::InvalidRecipient);
    require!(
        splits.iter().any(|s| s.recipient == seller),
        PayRogenError::SellerNotInSplits
    );
    // The escrow PDA owns the vault; naming it as a recipient would make the
    // vault pay itself and leave a balance that blocks closing on release.
    let escrow_key = ctx.accounts.escrow.key();
    require!(
        splits.iter().all(|s| s.recipient != escrow_key),
        PayRogenError::InvalidRecipient
    );

    // ─── Freeze check ────────────────────────────────────────────────────────
    let buyer = ctx.accounts.buyer.key();
    assert_not_frozen(&ctx.accounts.freeze_account.to_account_info(), &buyer, now)?;

    require!(
        ctx.accounts.buyer_token_account.amount >= amount,
        PayRogenError::InsufficientBalance
    );

    // ─── Initialize escrow state ─────────────────────────────────────────────
    let mut stored_splits = [SplitConfig {
        recipient: Pubkey::default(),
        basis_points: 0,
    }; MAX_SPLITS];
    for (slot, entry) in stored_splits.iter_mut().zip(splits.iter()) {
        *slot = SplitConfig {
            recipient: entry.recipient,
            basis_points: entry.basis_points,
        };
    }

    let timeout_at = now
        .checked_add(timeout_seconds)
        .ok_or(PayRogenError::InvalidTimeout)?;
    let mint = ctx.accounts.mint.key();

    let escrow = &mut ctx.accounts.escrow;
    escrow.buyer = buyer;
    escrow.seller = seller;
    escrow.merchant = merchant;
    escrow.amount = amount;
    escrow.order_id = order_id;
    escrow.status = EscrowStatus::Locked;
    escrow.locked_at = now;
    escrow.timeout_at = timeout_at;
    escrow.split_count = splits.len() as u8;
    escrow.splits = stored_splits;
    escrow.bump = ctx.bumps.escrow;
    escrow.mint = mint;
    escrow.vault_bump = ctx.bumps.escrow_token_account;

    // ─── Lock funds in the vault ─────────────────────────────────────────────
    let transfer_ctx = CpiContext::new(
        ctx.accounts.token_program.key(),
        Transfer {
            from: ctx.accounts.buyer_token_account.to_account_info(),
            to: ctx.accounts.escrow_token_account.to_account_info(),
            authority: ctx.accounts.buyer.to_account_info(),
        },
    );
    token::transfer(transfer_ctx, amount)?;

    emit!(EscrowCreated {
        escrow: ctx.accounts.escrow.key(),
        buyer,
        seller,
        mint,
        amount,
        order_id,
        timeout_at,
    });

    Ok(())
}
