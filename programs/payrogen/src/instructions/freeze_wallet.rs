//! # Freeze Wallet Instruction
//!
//! Freezes a wallet for 24 hours, blocking outgoing transfers made through
//! this program. Only the gateway authority (Admin_MultiSig) can invoke it.
//!
//! The freeze is time-based and auto-expires after 24 hours (86400 seconds)
//! without requiring an explicit unfreeze instruction. Re-freezing resets the
//! 24-hour window.
//!
//! ## Scope
//! The freeze is enforced by `execute_split_payment`,
//! `execute_sol_split_payment`, `create_escrow`, and buyer-initiated
//! `release_escrow`. It cannot block SPL/System transfers the wallet owner
//! signs outside this program.

use anchor_lang::prelude::*;

use crate::errors::PayRogenError;
use crate::state::FREEZE_DURATION_SECONDS;
use crate::WalletFrozenEvent;

/// Handler for the `freeze_wallet` instruction.
///
/// Authority validation is enforced by `has_one = authority` on
/// `gateway_config` in the `FreezeWallet` accounts struct.
pub fn handler(ctx: Context<crate::FreezeWallet>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let expires_at = now
        .checked_add(FREEZE_DURATION_SECONDS)
        .ok_or(PayRogenError::InvalidTimeout)?;

    let wallet = ctx.accounts.wallet.key();
    let freeze_account = &mut ctx.accounts.freeze_account;
    freeze_account.wallet = wallet;
    freeze_account.frozen_at = now;
    freeze_account.expires_at = expires_at;
    freeze_account.bump = ctx.bumps.freeze_account;

    emit!(WalletFrozenEvent { wallet, expires_at });
    Ok(())
}
