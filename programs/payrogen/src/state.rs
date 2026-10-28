//! # PayRogen Program State
//!
//! Defines all on-chain account structures and their associated types for the
//! PayRogen payment gateway smart contract. These accounts are stored as PDAs
//! (Program Derived Addresses) on Solana.

use anchor_lang::prelude::*;

/// Maximum number of split recipients allowed in a single escrow or payment.
/// Capped at 5 to keep account size bounded and compute costs predictable.
pub const MAX_SPLITS: usize = 5;

/// Represents the lifecycle status of an escrow account.
/// Transitions: Locked → Released | Locked → Refunded | Locked → Disputed → Released | Disputed → Refunded
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum EscrowStatus {
    /// Funds are locked and awaiting delivery confirmation or timeout
    Locked,
    /// Funds have been distributed to recipients per split config
    Released,
    /// Funds have been returned to the buyer
    Refunded,
    /// A dispute has been raised; funds are held pending resolution
    Disputed,
}

/// Split configuration entry stored within an escrow account.
/// Defines a single recipient and their share of the escrowed funds.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub struct SplitConfig {
    /// Recipient wallet address that will receive this portion on release
    pub recipient: Pubkey,
    /// Share in basis points (1 bp = 0.01%). All entries must sum to 10000.
    pub basis_points: u16,
}

/// Gateway configuration account storing admin authority, treasury, and fee settings.
///
/// This is a singleton PDA account that holds global program parameters.
/// Only the designated authority (Admin_MultiSig) can modify these settings.
///
/// Seeds: `[b"gateway_config"]`
#[account]
pub struct GatewayConfig {
    /// Admin_MultiSig authority that can update gateway parameters (fee, treasury)
    pub authority: Pubkey,
    /// PayRogen treasury wallet address for receiving gateway fees
    pub treasury: Pubkey,
    /// Current gateway fee in basis points (max 500 = 5%)
    pub gateway_fee_bps: u16,
    /// PDA bump seed for address derivation
    pub bump: u8,
}

impl GatewayConfig {
    /// Account space calculation:
    /// Discriminator (8) + authority (32) + treasury (32) + gateway_fee_bps (2) + bump (1)
    pub const LEN: usize = 8 + 32 + 32 + 2 + 1;
}

/// Escrow account holding locked funds for a specific order.
///
/// Created by the `create_escrow` instruction when a buyer initiates an
/// escrow-based payment. Funds are held until delivery confirmation (release)
/// or timeout expiry (refund).
///
/// Seeds: `[b"escrow", buyer.key().as_ref(), order_id.as_ref()]`
#[account]
pub struct EscrowAccount {
    /// Buyer who funded the escrow (also the PDA seed component)
    pub buyer: Pubkey,
    /// Seller who will receive the majority of funds on release
    pub seller: Pubkey,
    /// Merchant platform facilitating the transaction
    pub merchant: Pubkey,
    /// Total amount locked in token units (lamports for SOL, smallest unit for SPL)
    pub amount: u64,
    /// Unique order identifier (32 bytes) — used as PDA seed to enforce uniqueness
    pub order_id: [u8; 32],
    /// Current escrow lifecycle status
    pub status: EscrowStatus,
    /// Unix timestamp when funds were locked (set at creation)
    pub locked_at: i64,
    /// Unix timestamp when escrow becomes eligible for auto-refund
    pub timeout_at: i64,
    /// Number of active split entries (0..=MAX_SPLITS)
    pub split_count: u8,
    /// Split configuration validated at escrow creation (fixed-size array, max 5 entries)
    pub splits: [SplitConfig; MAX_SPLITS],
    /// PDA bump seed for account address derivation
    pub bump: u8,
}

impl EscrowAccount {
    /// Account space calculation:
    /// Discriminator (8) + buyer (32) + seller (32) + merchant (32) + amount (8) +
    /// order_id (32) + status (1 byte enum tag) + locked_at (8) + timeout_at (8) +
    /// split_count (1) + splits (MAX_SPLITS * (32 + 2)) + bump (1)
    pub const LEN: usize = 8 + 32 + 32 + 32 + 8 + 32 + 1 + 8 + 8 + 1 + (MAX_SPLITS * 34) + 1;
}

/// Wallet freeze account that blocks outgoing transfers for 24 hours.
///
/// Created by the `freeze_wallet` instruction when a duress phrase is detected
/// or an admin triggers a security freeze. The freeze automatically expires
/// after 24 hours (86400 seconds) without requiring an explicit unfreeze.
///
/// Seeds: `[b"freeze", wallet.as_ref()]`
#[account]
pub struct WalletFreeze {
    /// The wallet address that is frozen (used as PDA seed)
    pub wallet: Pubkey,
    /// Unix timestamp when the freeze was activated
    pub frozen_at: i64,
    /// Unix timestamp when the freeze expires (frozen_at + 86400)
    pub expires_at: i64,
    /// PDA bump seed for account address derivation
    pub bump: u8,
}

impl WalletFreeze {
    /// Account space calculation:
    /// Discriminator (8) + wallet (32) + frozen_at (8) + expires_at (8) + bump (1)
    pub const LEN: usize = 8 + 32 + 8 + 8 + 1;
}
