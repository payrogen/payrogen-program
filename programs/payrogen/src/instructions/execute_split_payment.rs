//! # Execute Split Payment Instruction
//!
//! Executes an atomic split payment distributing SPL tokens from the payer's
//! token account to multiple recipients according to basis point allocations.
//!
//! ## Atomicity Guarantee
//! Solana's runtime ensures all CPI transfers within a single transaction either
//! succeed together or the entire transaction reverts — no partial distributions.
//!
//! ## Security
//! - Splits are validated (count, non-zero shares, sum == 10000).
//! - The gateway fee is enforced on-chain: the treasury entry must carry at
//!   least `gateway_config.gateway_fee_bps`.
//! - Each recipient token account (`remaining_accounts[i]`) must be an SPL
//!   token account owned by `splits[i].recipient` with the payer's mint, so a
//!   caller cannot redirect a share to an account the named recipient does not
//!   control.
//! - Frozen payers are rejected.
//! - The rounding remainder goes to the first entry; the payer is debited
//!   exactly `amount`.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Transfer};

use crate::errors::PayRogenError;
use crate::validation::{
    assert_not_frozen, compute_shares, validate_recipient_token_account, validate_splits,
};
use crate::SplitEntry;

/// Handler for the `execute_split_payment` instruction.
pub fn handler<'info>(
    ctx: Context<'info, crate::ExecuteSplitPayment<'info>>,
    amount: u64,
    splits: Vec<SplitEntry>,
) -> Result<()> {
    // ─── Input Validation ────────────────────────────────────────────────────
    require!(amount > 0, PayRogenError::InvalidAmount);

    let config = &ctx.accounts.gateway_config;
    // Duplicate recipients are harmless for a direct payment (e.g. the
    // platform owner is also the seller), so they are allowed here.
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
        ctx.accounts.payer_token_account.amount >= amount,
        PayRogenError::InsufficientBalance
    );

    let mint = ctx.accounts.payer_token_account.mint;
    for (split, account) in splits.iter().zip(ctx.remaining_accounts.iter()) {
        validate_recipient_token_account(account, &split.recipient, &mint)?;
    }

    // ─── Execute Transfers ───────────────────────────────────────────────────
    let bps: Vec<u16> = splits.iter().map(|s| s.basis_points).collect();
    let shares = compute_shares(amount, &bps)?;

    for (share, account) in shares.iter().zip(ctx.remaining_accounts.iter()) {
        if *share == 0 {
            continue;
        }
        let cpi_accounts = Transfer {
            from: ctx.accounts.payer_token_account.to_account_info(),
            to: account.to_account_info(),
            authority: ctx.accounts.payer.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.token_program.key(), cpi_accounts);
        token::transfer(cpi_ctx, *share)?;
    }

    Ok(())
}
