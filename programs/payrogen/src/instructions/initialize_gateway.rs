//! # Initialize Gateway Instruction Handler
//!
//! One-time initialization of the GatewayConfig PDA that stores global
//! program parameters: admin authority, treasury address, and gateway fee.
//!
//! ## Security
//! - Can only be called once (Anchor's `init` constraint on the PDA prevents
//!   re-initialization at the Solana runtime level).
//! - Only the program's upgrade authority can call it: the accounts struct
//!   requires the program's ProgramData account and checks that its
//!   `upgrade_authority_address` is the signer. This closes the
//!   deploy-then-initialize front-running window.
//! - Fee is validated against the 500 bps maximum at initialization.
//! - The signer becomes the gateway authority; rotate it to a multisig with
//!   `propose_authority` / `accept_authority`.

use anchor_lang::prelude::*;

use crate::errors::PayRogenError;
use crate::state::MAX_FEE_BPS;

/// Handler for the `initialize_gateway` instruction.
pub fn handler(ctx: Context<crate::InitializeGateway>, gateway_fee_bps: u16) -> Result<()> {
    require!(
        gateway_fee_bps <= MAX_FEE_BPS,
        PayRogenError::FeeExceedsMaximum
    );
    require_keys_neq!(
        ctx.accounts.treasury.key(),
        Pubkey::default(),
        PayRogenError::InvalidAuthority
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
