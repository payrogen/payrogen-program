//! # Authority and Treasury Rotation
//!
//! - `propose_authority`: the current authority nominates a new authority.
//!   Re-proposing overwrites the nomination; proposing the current authority
//!   effectively cancels a pending rotation.
//! - `accept_authority`: the nominee signs to take over. Two steps mean a
//!   mistyped address can never take control or brick the gateway.
//! - `set_treasury`: the authority changes the fee treasury. Off-chain clients
//!   must read the treasury from GatewayConfig (bytes 40..72), because every
//!   split is checked against it.
//!
//! The pending nomination lives in its own PDA so the 75-byte GatewayConfig
//! layout (read at fixed offsets by clients) is unchanged.

use anchor_lang::prelude::*;

use crate::errors::PayRogenError;
use crate::{AuthorityProposed, GatewayConfigUpdated};

/// Handler for `propose_authority`.
pub fn propose(ctx: Context<crate::ProposeAuthority>, new_authority: Pubkey) -> Result<()> {
    require_keys_neq!(new_authority, Pubkey::default(), PayRogenError::InvalidAuthority);

    let pending = &mut ctx.accounts.pending_authority;
    pending.new_authority = new_authority;
    pending.proposed_at = Clock::get()?.unix_timestamp;
    pending.bump = ctx.bumps.pending_authority;

    emit!(AuthorityProposed {
        current_authority: ctx.accounts.authority.key(),
        new_authority,
    });
    Ok(())
}

/// Handler for `accept_authority`. The pending account is closed by Anchor
/// (`close = previous_authority`), refunding its rent to the proposer.
pub fn accept(ctx: Context<crate::AcceptAuthority>) -> Result<()> {
    let config = &mut ctx.accounts.gateway_config;
    config.authority = ctx.accounts.new_authority.key();

    emit!(GatewayConfigUpdated {
        authority: config.authority,
        treasury: config.treasury,
        gateway_fee_bps: config.gateway_fee_bps,
    });
    Ok(())
}

/// Handler for `set_treasury`.
pub fn set_treasury(ctx: Context<crate::SetTreasury>, new_treasury: Pubkey) -> Result<()> {
    require_keys_neq!(new_treasury, Pubkey::default(), PayRogenError::InvalidAuthority);

    let config = &mut ctx.accounts.gateway_config;
    config.treasury = new_treasury;

    emit!(GatewayConfigUpdated {
        authority: config.authority,
        treasury: config.treasury,
        gateway_fee_bps: config.gateway_fee_bps,
    });
    Ok(())
}
