//! # Freeze Wallet Instruction
//!
//! Freezes a wallet for 24 hours, blocking all outgoing transfers.
//! Only the gateway program authority (Admin_MultiSig) can invoke this instruction.
//!
//! The freeze is time-based and auto-expires after 24 hours (86400 seconds)
//! without requiring an explicit unfreeze instruction.
//!
//! ## Requirements Addressed
//! - 7.3: Wallet freeze mechanism
//! - 7.4: Freeze duration enforcement
//! - 7.5: Only gateway authority can freeze wallets
//! - 7.6: Auto-restore after 24h expiry
//! - 11.5: On-chain freeze enforcement

use anchor_lang::prelude::*;

/// Duration of a wallet freeze in seconds (24 hours).
const FREEZE_DURATION_SECONDS: i64 = 86400;

/// Handler for the `freeze_wallet` instruction.
///
/// Sets the freeze parameters on the WalletFreeze PDA account, blocking
/// outgoing transfers from the target wallet for 24 hours.
///
/// # Security
/// - Authority validation is enforced by Anchor's `has_one = authority`
///   constraint on the `gateway_config` account in the `FreezeWallet` struct.
/// - The `init_if_needed` constraint allows re-freezing an already existing
///   (expired) freeze account, effectively resetting the 24h timer.
///
/// # Arguments
/// * `ctx` - The Anchor context containing the freeze account, wallet, and authority
///
/// # Behavior
/// - Sets `frozen_at` to the current Solana clock timestamp
/// - Sets `expires_at` to `frozen_at + 86400` (24 hours from now)
/// - Incoming transfers to the frozen wallet are NOT affected
/// - The freeze auto-expires when `expires_at < current_time`
pub fn handler(ctx: Context<crate::FreezeWallet>) -> Result<()> {
    let clock = Clock::get()?;
    let current_time = clock.unix_timestamp;

    let freeze_account = &mut ctx.accounts.freeze_account;
    freeze_account.wallet = ctx.accounts.wallet.key();
    freeze_account.frozen_at = current_time;
    freeze_account.expires_at = current_time + FREEZE_DURATION_SECONDS;
    freeze_account.bump = ctx.bumps.freeze_account;

    Ok(())
}
