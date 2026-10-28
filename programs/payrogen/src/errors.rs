//! # PayRogen Error Codes
//!
//! Custom error codes for the PayRogen payment gateway smart contract.
//! Each error has a unique code (auto-assigned by Anchor starting at 6000)
//! and a human-readable message for debugging.

use anchor_lang::prelude::*;

/// Custom error codes for the PayRogen program.
/// These are returned as transaction errors when instruction validation fails.
#[error_code]
pub enum PayRogenError {
    /// Split basis points do not sum to exactly 10000, or splits array is
    /// empty or exceeds MAX_SPLITS limit.
    #[msg("Split values must sum to exactly 10000 basis points")]
    InvalidSplitSum,

    /// The payer/buyer does not have sufficient token balance for the
    /// requested operation, or the amount is zero.
    #[msg("Insufficient balance for this transaction")]
    InsufficientBalance,

    /// The payer's wallet is currently frozen (WalletFreeze.expires_at has
    /// not yet passed). Outgoing transfers are blocked during freeze.
    #[msg("Wallet is currently frozen")]
    WalletFrozen,

    /// The signer does not have authority to perform this operation.
    /// For example, the token account owner does not match the payer.
    #[msg("Unauthorized signer for this operation")]
    UnauthorizedSigner,

    /// The multi-signature authorization check failed. Required for
    /// admin operations like fee updates.
    #[msg("Invalid multi-signature authorization")]
    InvalidMultiSig,

    /// The proposed gateway fee exceeds the maximum allowed value of
    /// 500 basis points (5%).
    #[msg("Fee exceeds maximum of 500 basis points (5%)")]
    FeeExceedsMaximum,

    /// Attempted to create an escrow for an order_id that already has a
    /// locked escrow. Note: This is primarily enforced by Anchor's `init`
    /// constraint on the PDA, so this error serves as a semantic backup.
    #[msg("Escrow is already locked for this order")]
    EscrowAlreadyLocked,

    /// The escrow is not in the expected Locked state for the requested
    /// operation (release or refund).
    #[msg("Escrow is not in locked state")]
    EscrowNotLocked,

    /// A recipient token account is invalid (not writable, wrong owner,
    /// or wrong mint).
    #[msg("Invalid recipient address")]
    InvalidRecipient,

    /// The escrow timeout_seconds is invalid (zero or negative), or the
    /// computed timeout_at would overflow i64, or the escrow has already
    /// timed out for the requested operation.
    #[msg("Escrow has timed out")]
    EscrowTimeout,
}
