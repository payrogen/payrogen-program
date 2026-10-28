//! # Execute SOL Split Payment Instruction
//!
//! Executes an atomic split payment distributing native SOL from the payer
//! to multiple recipients according to basis point allocations.
//!
//! Uses SystemProgram CPI transfers for native SOL (not SPL tokens).
//! Same atomicity and validation guarantees as `execute_split_payment`:
//! gateway fee enforced on-chain, each `remaining_accounts[i]` must be exactly
//! `splits[i].recipient`, frozen payers rejected, and the rounding remainder
//! goes to the first entry so exactly `amount` lamports leave the payer.

use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};

use crate::errors::PayRogenError;
use crate::validation::{assert_not_frozen, compute_shares, validate_splits};
use crate::SplitEntry;

/// Handler for the `execute_sol_split_payment` instruction.
pub fn handler<'info>(
    ctx: Context<'info, crate::ExecuteSolSplitPayment<'info>>,
    amount: u64,
    splits: Vec<SplitEntry>,
) -> Result<()> {
    // ─── Input Validation ────────────────────────────────────────────────────
    require!(amount > 0, PayRogenError::InvalidAmount);

    let config = &ctx.accounts.gateway_config;
    validate_splits(&splits, &config.treasury, config.gateway_fee_bps, true)?;

    // ─── Freeze Check ────────────────────────────────────────────────────────
    let now = Clock::get()?.unix_timestamp;
    assert_not_frozen(
        &ctx.accounts.freeze_account.to_account_info(),
        &ctx.accounts.payer.key(),
        now,
    )?;

    // ─── Account Validation (all before any funds move) ──────────────────────
    require!(
        ctx.remaining_accounts.len() == splits.len(),
        PayRogenError::InvalidRecipient
    );
    require!(
        ctx.accounts.payer.lamports() >= amount,
        PayRogenError::InsufficientBalance
    );
    for (split, recipient) in splits.iter().zip(ctx.remaining_accounts.iter()) {
        require!(recipient.is_writable, PayRogenError::InvalidRecipient);
        require_keys_eq!(recipient.key(), split.recipient, PayRogenError::InvalidRecipient);
    }

    // ─── Execute SOL Transfers ───────────────────────────────────────────────
    let bps: Vec<u16> = splits.iter().map(|s| s.basis_points).collect();
    let shares = compute_shares(amount, &bps)?;

    for (share, recipient) in shares.iter().zip(ctx.remaining_accounts.iter()) {
        if *share == 0 {
            continue;
        }
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.key(),
            Transfer {
                from: ctx.accounts.payer.to_account_info(),
                to: recipient.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, *share)?;
    }

    Ok(())
}
