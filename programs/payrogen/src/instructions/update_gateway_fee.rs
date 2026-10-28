//! # Update Gateway Fee Instruction Handler
//!
//! Updates the gateway fee parameter in the GatewayConfig PDA.
//! Authorization is enforced by Anchor's `has_one = authority` constraint
//! on the UpdateGatewayFee account struct, requiring the Admin_MultiSig
//! authority to sign the transaction.
//!
//! ## Requirements
//! - 6.1: Admin_MultiSig authorization (via `has_one = authority`)
//! - 6.2: Fee stored in basis points
//! - 6.3: Fee applies to all subsequent transactions (read at execution time)
//! - 6.4: Immediate effect after update
//! - 6.5: Maximum fee cap of 500 basis points (5%)
//! - 11.4: On-chain fee parameter validation

use anchor_lang::prelude::*;

use crate::errors::PayRogenError;
use crate::UpdateGatewayFee;

/// Maximum gateway fee in basis points (500 = 5%).
/// Any attempt to set a fee above this value will be rejected.
const MAX_FEE_BPS: u16 = 500;

/// Handler for the `update_gateway_fee` instruction.
///
/// Validates that the new fee does not exceed the maximum cap, then persists
/// the updated value to the GatewayConfig account. The new fee takes effect
/// immediately for all subsequent transactions that read from gateway_config.
///
/// # Arguments
/// * `ctx` - The Anchor context containing the gateway_config account (mutable)
///           and the authority signer (validated by `has_one = authority`).
/// * `new_fee_bps` - The new gateway fee in basis points (must be <= 500).
///
/// # Errors
/// * `PayRogenError::FeeExceedsMaximum` - If `new_fee_bps > 500`.
pub fn handler(ctx: Context<UpdateGatewayFee>, new_fee_bps: u16) -> Result<()> {
    // Validate fee does not exceed maximum cap (Requirement 6.5)
    require!(
        new_fee_bps <= MAX_FEE_BPS,
        PayRogenError::FeeExceedsMaximum
    );

    // Update the gateway configuration with the new fee (Requirements 6.2, 6.3, 6.4)
    let gateway_config = &mut ctx.accounts.gateway_config;
    gateway_config.gateway_fee_bps = new_fee_bps;

    msg!(
        "Gateway fee updated to {} basis points ({}.{}%)",
        new_fee_bps,
        new_fee_bps / 100,
        new_fee_bps % 100
    );

    Ok(())
}
