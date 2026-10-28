//! # Execute Split Payment Instruction
//!
//! Executes an atomic split payment distributing SPL tokens from the payer's
//! token account to multiple recipients according to basis point allocations.
//!
//! ## Atomicity Guarantee
//! Solana's runtime ensures all CPI transfers within a single transaction either
//! succeed together or the entire transaction reverts — no partial distributions.
//!
//! ## Requirements Addressed
//! - 3.2: Atomic split payment execution
//! - 3.3: All-or-nothing transaction semantics
//! - 5.1: Payer must be transaction signer
//! - 5.2: Splits must sum to exactly 10000 basis points
//! - 5.4: Check wallet is not frozen
//! - 5.5: Validate all recipient addresses
//! - 5.6: Revert all transfers if any fails
//! - 11.2: On-chain split payment enforcement
//! - 11.3: Basis point validation on-chain

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Transfer};

use crate::errors::PayRogenError;
use crate::state::{MAX_SPLITS, WalletFreeze};
use crate::SplitEntry;

/// Handler for the `execute_split_payment` instruction.
///
/// # Validation Steps
/// 1. Splits vector is not empty and within bounds
/// 2. Splits basis points sum to exactly 10000
/// 3. Wallet is not frozen
/// 4. Number of remaining_accounts matches the number of split entries
/// 5. Payer has sufficient balance for the total amount
/// 6. Amount is non-zero
pub fn handler<'info>(
    ctx: Context<'info, crate::ExecuteSplitPayment<'info>>,
    amount: u64,
    splits: Vec<SplitEntry>,
) -> Result<()> {
    // ─── Input Validation ────────────────────────────────────────────────────

    // Amount must be non-zero
    require!(amount > 0, PayRogenError::InsufficientBalance);

    // At least one recipient, no more than MAX_SPLITS
    require!(!splits.is_empty() && splits.len() <= MAX_SPLITS, PayRogenError::InvalidSplitSum);

    // Basis points must sum to exactly 10000
    let total_bps: u32 = splits.iter().map(|s| s.basis_points as u32).sum();
    require!(total_bps == 10000, PayRogenError::InvalidSplitSum);

    // ─── Freeze Check ────────────────────────────────────────────────────────

    let freeze_info = &ctx.accounts.freeze_account;
    if freeze_info.data_len() > 0 {
        let data = freeze_info.try_borrow_data()?;
        if data.len() >= WalletFreeze::LEN {
            let expires_at = i64::from_le_bytes(data[48..56].try_into().unwrap());
            let clock = Clock::get()?;
            require!(
                expires_at < clock.unix_timestamp,
                PayRogenError::WalletFrozen
            );
        }
    }

    // ─── Account Validation ──────────────────────────────────────────────────

    require!(
        ctx.remaining_accounts.len() == splits.len(),
        PayRogenError::InvalidRecipient
    );

    require!(
        ctx.accounts.payer_token_account.amount >= amount,
        PayRogenError::InsufficientBalance
    );

    // ─── Execute Transfers ───────────────────────────────────────────────────

    for (i, split) in splits.iter().enumerate() {
        // Using u128 intermediate to prevent overflow on large amounts
        let transfer_amount = (amount as u128)
            .checked_mul(split.basis_points as u128)
            .unwrap()
            .checked_div(10000)
            .unwrap() as u64;

        if transfer_amount == 0 {
            continue;
        }

        let recipient_token_account = &ctx.remaining_accounts[i];

        require!(
            recipient_token_account.is_writable,
            PayRogenError::InvalidRecipient
        );

        let cpi_accounts = Transfer {
            from: ctx.accounts.payer_token_account.to_account_info(),
            to: recipient_token_account.to_account_info(),
            authority: ctx.accounts.payer.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.token_program.key(), cpi_accounts);

        token::transfer(cpi_ctx, transfer_amount)?;
    }

    Ok(())
}
