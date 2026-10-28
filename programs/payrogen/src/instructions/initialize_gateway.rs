//! # Initialize Gateway Instruction Handler
//!
//! One-time initialization of the GatewayConfig PDA that stores global
//! program parameters: admin authority, treasury address, and gateway fee.
//!
//! ## Security
//! - Can only be called once (Anchor's `init` constraint on the PDA prevents
//!   re-initialization at the Solana runtime level).
//! - The caller becomes the gateway authority (Admin_MultiSig).
//! - Fee is validated against the 500 bps maximum at initialization.
//!
//! ## Requirements
//! - Must be called before any other instruction (all instructions reference gateway_config).

use anchor_lang::prelude::*;

use crate::errors::PayRogenError;

/// Maximum gateway fee in basis points (500 = 5%).
const MAX_FEE_BPS: u16 = 500;

/// Handler for the `initialize_gateway` instruction.
///
/// Creates the GatewayConfig PDA with the deployer as authority,
/// the specified treasury wallet, and the initial fee.
///
/// # Arguments
/// * `ctx` - The Anchor context containing the gateway_config account to initialize.
/// * `gateway_fee_bps` - Initial gateway fee in basis points (must be <= 500).
///
/// # Errors
/// * `PayRogenError::FeeExceedsMaximum` - If `gateway_fee_bps > 500`.
pub fn handler(ctx: Context<crate::InitializeGateway>, gateway_fee_bps: u16) -> Result<()> {
    // Validate initial fee does not exceed maximum
    require!(
        gateway_fee_bps <= MAX_FEE_BPS,
        PayRogenError::FeeExceedsMaximum
    );

    let gateway_config = &mut ctx.accounts.gateway_config;
    gateway_config.authority = ctx.accounts.authority.key();
    gateway_config.treasury = ctx.accounts.treasury.key();
    gateway_config.gateway_fee_bps = gateway_fee_bps;
    gateway_config.bump = ctx.bumps.gateway_config;

    msg!(
        "Gateway initialized: authority={}, treasury={}, fee={}bps",
        gateway_config.authority,
        gateway_config.treasury,
        gateway_fee_bps
    );

    Ok(())
}
