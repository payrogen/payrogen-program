//! # Execute SOL Split Payment Instruction
//!
//! Executes an atomic split payment distributing native SOL from the payer
//! to multiple recipients according to basis point allocations.
//!
//! Uses SystemProgram CPI transfers for native SOL (not SPL tokens).
//! Same atomicity guarantees as execute_split_payment.

use anchor_lang::prelude::*;

use crate::errors::PayRogenError;
use crate::state::MAX_SPLITS;
use crate::SplitEntry;

/// Handler for the `execute_sol_split_payment` instruction.
pub fn handler<'info>(
    ctx: Context<'info, crate::ExecuteSolSplitPayment<'info>>,
    amount: u64,
    splits: Vec<SplitEntry>,
) -> Result<()> {
    // ─── Input Validation ────────────────────────────────────────────────────

    require!(amount > 0, PayRogenError::InsufficientBalance);
    require!(!splits.is_empty() && splits.len() <= MAX_SPLITS, PayRogenError::InvalidSplitSum);

    let total_bps: u32 = splits.iter().map(|s| s.basis_points as u32).sum();
    require!(total_bps == 10000, PayRogenError::InvalidSplitSum);

    // ─── Freeze Check ────────────────────────────────────────────────────────

    let freeze_info = &ctx.accounts.freeze_account;
    if freeze_info.data_len() > 0 {
        let data = freeze_info.try_borrow_data()?;
        if data.len() >= crate::state::WalletFreeze::LEN {
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

    // Check payer has enough lamports
    let payer_lamports = ctx.accounts.payer.lamports();
    require!(payer_lamports >= amount, PayRogenError::InsufficientBalance);

    // ─── Execute SOL Transfers ───────────────────────────────────────────────

    for (i, split) in splits.iter().enumerate() {
        let transfer_amount = (amount as u128)
            .checked_mul(split.basis_points as u128)
            .unwrap()
            .checked_div(10000)
            .unwrap() as u64;

        if transfer_amount == 0 {
            continue;
        }

        let recipient = &ctx.remaining_accounts[i];

        require!(
            recipient.is_writable,
            PayRogenError::InvalidRecipient
        );

        // Verify recipient matches the split entry
        require!(
            recipient.key() == split.recipient,
            PayRogenError::InvalidRecipient
        );

        // CPI to SystemProgram for native SOL transfer
        anchor_lang::solana_program::program::invoke(
            &anchor_lang::solana_program::system_instruction::transfer(
                &ctx.accounts.payer.key(),
                &recipient.key(),
                transfer_amount,
            ),
            &[
                ctx.accounts.payer.to_account_info(),
                recipient.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;
    }

    Ok(())
}
