//! # Raise Dispute Instruction Handler
//!
//! Moves a `Locked` escrow to `Disputed`. Only the buyer or the seller may
//! raise a dispute, and only before `timeout_at` (after the timeout the buyer
//! is already entitled to a refund, so a late dispute could only be used to
//! block it).
//!
//! While disputed, the permissionless timeout refund is suspended. The escrow
//! is resolved by:
//! - the buyer releasing it (`release_escrow`), or
//! - the gateway authority releasing it (`release_escrow`) or refunding it
//!   (`refund_escrow`).
//!
//! In every case funds can only go to the stored split recipients or back to
//! the buyer; the authority cannot direct them anywhere else.

use anchor_lang::prelude::*;

use crate::errors::PayRogenError;
use crate::state::EscrowStatus;
use crate::DisputeRaised;

/// Handler for the `raise_dispute` instruction.
pub fn handler(ctx: Context<crate::RaiseDispute>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let caller = ctx.accounts.caller.key();
    let escrow = &mut ctx.accounts.escrow;

    require!(
        caller == escrow.buyer || caller == escrow.seller,
        PayRogenError::UnauthorizedSigner
    );
    require!(
        escrow.status == EscrowStatus::Locked,
        PayRogenError::EscrowNotLocked
    );
    require!(now < escrow.timeout_at, PayRogenError::EscrowTimeout);

    escrow.status = EscrowStatus::Disputed;

    emit!(DisputeRaised {
        escrow: escrow.key(),
        raised_by: caller,
    });
    Ok(())
}
