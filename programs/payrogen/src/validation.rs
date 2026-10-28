//! # Shared Validation Helpers
//!
//! Security checks shared by several instructions. Keeping them in one place
//! guarantees that direct payments and escrows enforce identical rules.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, TokenAccount};

use crate::errors::PayRogenError;
use crate::state::{WalletFreeze, BPS_DENOMINATOR, MAX_SPLITS};
use crate::SplitEntry;

/// Validates a split configuration.
///
/// Rules:
/// - 1..=`MAX_SPLITS` entries
/// - no default (all-zero) recipient
/// - shares sum to exactly 10000 bps (zero-bps entries are allowed and
///   receive nothing; clients emit a 0-bps treasury entry when the fee is 0)
/// - entries whose recipient is `treasury` together allocate at least
///   `min_fee_bps` (on-chain gateway fee enforcement)
/// - optionally, no duplicate recipients
pub fn validate_splits(
    splits: &[SplitEntry],
    treasury: &Pubkey,
    min_fee_bps: u16,
    allow_duplicate_recipients: bool,
) -> Result<()> {
    require!(
        !splits.is_empty() && splits.len() <= MAX_SPLITS,
        PayRogenError::InvalidSplitSum
    );

    let mut total_bps: u32 = 0;
    let mut treasury_bps: u32 = 0;
    for (i, entry) in splits.iter().enumerate() {
        require!(
            entry.recipient != Pubkey::default(),
            PayRogenError::InvalidRecipient
        );
        if !allow_duplicate_recipients {
            require!(
                splits[..i].iter().all(|prev| prev.recipient != entry.recipient),
                PayRogenError::InvalidRecipient
            );
        }
        // Max 5 * 65535 — cannot overflow u32.
        total_bps += entry.basis_points as u32;
        if entry.recipient == *treasury {
            treasury_bps += entry.basis_points as u32;
        }
    }

    require!(total_bps == BPS_DENOMINATOR, PayRogenError::InvalidSplitSum);
    require!(
        treasury_bps >= min_fee_bps as u32,
        PayRogenError::GatewayFeeNotPaid
    );
    Ok(())
}

/// Splits `amount` according to `bps` (which must sum to 10000).
///
/// Each share is `floor(amount * bps / 10000)`. The rounding remainder (at
/// most `len - 1` base units) is added to the first entry, so the shares
/// always sum to exactly `amount` and no dust is left behind.
pub fn compute_shares(amount: u64, bps: &[u16]) -> Result<Vec<u64>> {
    require!(
        !bps.is_empty() && bps.len() <= MAX_SPLITS,
        PayRogenError::InvalidSplitSum
    );
    let total: u32 = bps.iter().map(|b| *b as u32).sum();
    require!(total == BPS_DENOMINATOR, PayRogenError::InvalidSplitSum);

    let mut shares = Vec::with_capacity(bps.len());
    let mut distributed: u64 = 0;
    for b in bps {
        // amount * bps <= u64::MAX * 10000 fits in u128; result <= amount.
        let share = ((amount as u128) * (*b as u128) / BPS_DENOMINATOR as u128) as u64;
        distributed = distributed
            .checked_add(share)
            .ok_or(PayRogenError::InvalidSplitSum)?;
        shares.push(share);
    }
    let remainder = amount
        .checked_sub(distributed)
        .ok_or(PayRogenError::InvalidSplitSum)?;
    shares[0] = shares[0]
        .checked_add(remainder)
        .ok_or(PayRogenError::InvalidSplitSum)?;
    Ok(shares)
}

/// Rejects the operation if `wallet` has an active freeze.
///
/// `freeze_info` must already be constrained to the PDA
/// `[b"freeze", wallet]` by the caller's `seeds` constraint. An empty account
/// (never frozen) passes. A non-empty account must be a program-owned
/// `WalletFreeze` (discriminator checked) for `wallet`; anything else fails
/// closed.
pub fn assert_not_frozen(freeze_info: &AccountInfo, wallet: &Pubkey, now: i64) -> Result<()> {
    if freeze_info.data_is_empty() {
        return Ok(());
    }
    require_keys_eq!(
        *freeze_info.owner,
        crate::ID,
        PayRogenError::InvalidFreezeAccount
    );
    let data = freeze_info.try_borrow_data()?;
    let freeze = WalletFreeze::try_deserialize(&mut &data[..])
        .map_err(|_| error!(PayRogenError::InvalidFreezeAccount))?;
    require_keys_eq!(freeze.wallet, *wallet, PayRogenError::InvalidFreezeAccount);
    require!(freeze.expires_at < now, PayRogenError::WalletFrozen);
    Ok(())
}

/// Validates a recipient SPL token account passed through `remaining_accounts`.
///
/// The account must be writable, owned by the SPL Token program, initialized,
/// owned (token-account owner) by `expected_owner`, and hold `expected_mint`.
/// This binds each payout to the recipient wallet named in the instruction
/// data or escrow state, so a caller cannot substitute their own account.
pub fn validate_recipient_token_account(
    account: &AccountInfo,
    expected_owner: &Pubkey,
    expected_mint: &Pubkey,
) -> Result<()> {
    require!(account.is_writable, PayRogenError::InvalidRecipient);
    require_keys_eq!(*account.owner, token::ID, PayRogenError::InvalidRecipient);
    let data = account.try_borrow_data()?;
    let token_account = TokenAccount::try_deserialize(&mut &data[..])
        .map_err(|_| error!(PayRogenError::InvalidRecipient))?;
    require_keys_eq!(
        token_account.owner,
        *expected_owner,
        PayRogenError::InvalidRecipient
    );
    require_keys_eq!(
        token_account.mint,
        *expected_mint,
        PayRogenError::InvalidRecipient
    );
    Ok(())
}
