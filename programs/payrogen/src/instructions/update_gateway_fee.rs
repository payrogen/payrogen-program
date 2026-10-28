//! # Update Gateway Fee Instruction Handler
//!
//! Updates the gateway fee parameter in the GatewayConfig PDA.
//! Authorization is enforced by Anchor's `has_one = authority` constraint
//! on the UpdateGatewayFee account struct, requiring the Admin_MultiSig
//! authority to sign the transaction.
//!
//! The new fee applies immediately: every subsequent split payment and escrow
//! must allocate at least this many bps to the treasury. Clients read the fee
//! from the GatewayConfig account when building transactions; a transaction
//! built before an increase will fail with `GatewayFeeNotPaid` and must be
//! rebuilt.

use anchor_lang::prelude::*;

use crate::errors::PayRogenError;
use crate::state::MAX_FEE_BPS;
use crate::{GatewayConfigUpdated, UpdateGatewayFee};

/// Handler for the `update_gateway_fee` instruction.
pub fn handler(ctx: Context<UpdateGatewayFee>, new_fee_bps: u16) -> Result<()> {
    require!(
        new_fee_bps <= MAX_FEE_BPS,
        PayRogenError::FeeExceedsMaximum
    );

    let gateway_config = &mut ctx.accounts.gateway_config;
    gateway_config.gateway_fee_bps = new_fee_bps;

    msg!(
        "Gateway fee updated to {} basis points ({}.{:02}%)",
        new_fee_bps,
        new_fee_bps / 100,
        new_fee_bps % 100
    );

    emit!(GatewayConfigUpdated {
        authority: gateway_config.authority,
        treasury: gateway_config.treasury,
        gateway_fee_bps: gateway_config.gateway_fee_bps,
    });
    Ok(())
}
