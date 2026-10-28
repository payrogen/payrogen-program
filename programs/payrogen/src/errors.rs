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

    /// Reserved. Not emitted by the program: multisig enforcement is done by
    /// making `GatewayConfig.authority` (and the upgrade authority) a multisig
    /// vault such as Squads. Kept so later error codes stay stable.
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

    // ── Appended variants. Never reorder or insert above this line: Anchor
    // ── assigns codes sequentially (6000 + index) and clients map them.

    /// The amount is zero.
    #[msg("Amount must be greater than zero")]
    InvalidAmount,

    /// The split configuration does not allocate at least
    /// `gateway_config.gateway_fee_bps` to `gateway_config.treasury`.
    #[msg("Split does not pay the required gateway fee to the treasury")]
    GatewayFeeNotPaid,

    /// The escrow timeout is zero, negative, or above the maximum allowed.
    #[msg("Escrow timeout must be between 1 second and 90 days")]
    InvalidTimeout,

    /// The escrow has not reached its timeout and the caller has no other
    /// authority to perform the operation.
    #[msg("Escrow has not timed out yet")]
    EscrowNotTimedOut,

    /// The operation requires the escrow to be in the Disputed state.
    #[msg("Escrow is not in disputed state")]
    EscrowNotDisputed,

    /// The freeze account is not a valid WalletFreeze for this wallet.
    #[msg("Invalid wallet freeze account")]
    InvalidFreezeAccount,

    /// The provided authority or treasury is the default (all-zero) pubkey.
    #[msg("Invalid authority or treasury address")]
    InvalidAuthority,

    /// The seller is not one of the escrow split recipients.
    #[msg("Seller must be one of the split recipients")]
    SellerNotInSplits,
}
