//! # PayRogen Program State
//!
//! Defines all on-chain account structures and their associated types for the
//! PayRogen payment gateway smart contract. These accounts are stored as PDAs
//! (Program Derived Addresses) on Solana.

use anchor_lang::prelude::*;

/// Maximum number of split recipients allowed in a single escrow or payment.
/// Capped at 5 to keep account size bounded and compute costs predictable.
pub const MAX_SPLITS: usize = 5;

/// Basis-point denominator: 10000 bps = 100%.
pub const BPS_DENOMINATOR: u32 = 10_000;

/// Maximum gateway fee in basis points (500 = 5%).
pub const MAX_FEE_BPS: u16 = 500;

/// Duration of a wallet freeze in seconds (24 hours).
pub const FREEZE_DURATION_SECONDS: i64 = 86_400;

/// Maximum escrow timeout (90 days). Bounds how long funds can be locked
/// before the buyer becomes eligible for a permissionless refund.
pub const MAX_ESCROW_TIMEOUT_SECONDS: i64 = 90 * 86_400;

/// PDA seed for the singleton gateway configuration.
pub const GATEWAY_CONFIG_SEED: &[u8] = b"gateway_config";
/// PDA seed prefix for escrow state accounts: `[ESCROW_SEED, buyer, order_id]`.
pub const ESCROW_SEED: &[u8] = b"escrow";
/// PDA seed prefix for escrow token vaults: `[VAULT_SEED, escrow]`.
pub const VAULT_SEED: &[u8] = b"vault";
/// PDA seed prefix for wallet freezes: `[FREEZE_SEED, wallet]`.
pub const FREEZE_SEED: &[u8] = b"freeze";
/// PDA seed for the singleton pending-authority account used by the
/// two-step authority rotation.
pub const PENDING_AUTHORITY_SEED: &[u8] = b"pending_authority";

/// Represents the lifecycle status of an escrow account.
///
/// Transitions:
/// - `Locked → Released` (buyer confirms delivery)
/// - `Locked → Refunded` (timeout expired; permissionless)
/// - `Locked → Disputed` (buyer or seller raises a dispute before timeout)
/// - `Disputed → Released` (buyer concedes, or gateway authority resolves for the seller)
/// - `Disputed → Refunded` (gateway authority resolves for the buyer)
///
/// `Released` and `Refunded` are terminal; the escrow account is closed in the
/// same instruction, so these values are only observable in emitted events.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum EscrowStatus {
    /// Funds are locked and awaiting delivery confirmation or timeout
    Locked,
    /// Funds have been distributed to recipients per split config
    Released,
    /// Funds have been returned to the buyer
    Refunded,
    /// A dispute has been raised; timeout refunds are suspended until the
    /// gateway authority (or the buyer, by releasing) resolves it
    Disputed,
}

/// Split configuration entry stored within an escrow account.
/// Defines a single recipient and their share of the escrowed funds.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub struct SplitConfig {
    /// Recipient wallet address (token account owner) for this portion
    pub recipient: Pubkey,
    /// Share in basis points (1 bp = 0.01%). All entries must sum to 10000.
    pub basis_points: u16,
}

/// Gateway configuration account storing admin authority, treasury, and fee settings.
///
/// This is a singleton PDA account that holds global program parameters.
/// Only the designated authority (Admin_MultiSig) can modify these settings.
///
/// The layout is fixed at 75 bytes and is read at fixed offsets by off-chain
/// clients (treasury at 40..72, fee at 72..74). Do not add fields here; use a
/// separate PDA (see [`PendingAuthority`]) for new state.
///
/// Seeds: `[b"gateway_config"]`
#[account]
pub struct GatewayConfig {
    /// Admin_MultiSig authority that can update gateway parameters (fee, treasury)
    pub authority: Pubkey,
    /// PayRogen treasury wallet address for receiving gateway fees
    pub treasury: Pubkey,
    /// Current gateway fee in basis points (max 500 = 5%). Enforced on-chain:
    /// every split must allocate at least this many bps to `treasury`.
    pub gateway_fee_bps: u16,
    /// PDA bump seed for address derivation
    pub bump: u8,
}

impl GatewayConfig {
    /// Account space calculation:
    /// Discriminator (8) + authority (32) + treasury (32) + gateway_fee_bps (2) + bump (1)
    pub const LEN: usize = 8 + 32 + 32 + 2 + 1;
}

/// Pending authority nomination for the two-step authority rotation.
///
/// Created by `propose_authority` (current authority) and consumed by
/// `accept_authority` (the nominee must sign), so a mistyped address can
/// never take over or brick the gateway.
///
/// Seeds: `[b"pending_authority"]`
#[account]
pub struct PendingAuthority {
    /// Nominated authority that must sign `accept_authority`
    pub new_authority: Pubkey,
    /// Unix timestamp of the nomination
    pub proposed_at: i64,
    /// PDA bump seed
    pub bump: u8,
}

impl PendingAuthority {
    /// Discriminator (8) + new_authority (32) + proposed_at (8) + bump (1)
    pub const LEN: usize = 8 + 32 + 8 + 1;
}

/// Escrow account holding the state for a specific order's locked funds.
///
/// The tokens themselves sit in a program-derived vault token account
/// (`[b"vault", escrow]`) whose authority is this escrow PDA.
///
/// Seeds: `[b"escrow", buyer.key().as_ref(), order_id.as_ref()]`
#[account]
pub struct EscrowAccount {
    /// Buyer who funded the escrow (also the PDA seed component)
    pub buyer: Pubkey,
    /// Seller; must be one of the split recipients. May raise a dispute.
    pub seller: Pubkey,
    /// Merchant platform facilitating the transaction (informational; used
    /// by off-chain indexers, not for authorization)
    pub merchant: Pubkey,
    /// Amount deposited at creation, in the mint's smallest unit
    pub amount: u64,
    /// Unique order identifier (32 bytes) — used as PDA seed to enforce uniqueness
    pub order_id: [u8; 32],
    /// Current escrow lifecycle status
    pub status: EscrowStatus,
    /// Unix timestamp when funds were locked (set at creation)
    pub locked_at: i64,
    /// Unix timestamp when a `Locked` escrow becomes eligible for refund
    pub timeout_at: i64,
    /// Number of active split entries (1..=MAX_SPLITS)
    pub split_count: u8,
    /// Split configuration validated at escrow creation (fixed-size array, max 5 entries)
    pub splits: [SplitConfig; MAX_SPLITS],
    /// PDA bump seed for account address derivation
    pub bump: u8,
    /// SPL mint of the escrowed tokens. Every payout account must use this mint.
    pub mint: Pubkey,
    /// PDA bump of the vault token account (`[b"vault", escrow]`)
    pub vault_bump: u8,
}

impl EscrowAccount {
    /// Account space calculation:
    /// Discriminator (8) + buyer (32) + seller (32) + merchant (32) + amount (8) +
    /// order_id (32) + status (1 byte enum tag) + locked_at (8) + timeout_at (8) +
    /// split_count (1) + splits (MAX_SPLITS * (32 + 2)) + bump (1) + mint (32) +
    /// vault_bump (1)
    pub const LEN: usize =
        8 + 32 + 32 + 32 + 8 + 32 + 1 + 8 + 8 + 1 + (MAX_SPLITS * 34) + 1 + 32 + 1;

    /// Active split entries.
    pub fn active_splits(&self) -> &[SplitConfig] {
        &self.splits[..self.split_count as usize]
    }
}

/// Wallet freeze account that blocks outgoing transfers for 24 hours.
///
/// Created by the `freeze_wallet` instruction when a duress phrase is detected
/// or an admin triggers a security freeze. The freeze automatically expires
/// after 24 hours (86400 seconds) without requiring an explicit unfreeze.
///
/// Scope: a freeze only blocks transfers made *through this program*
/// (split payments, escrow creation, buyer-initiated escrow release). It cannot
/// stop the wallet owner from signing a plain SPL/System transfer elsewhere.
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
