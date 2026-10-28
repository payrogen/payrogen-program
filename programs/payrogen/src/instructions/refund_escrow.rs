//! # Refund Escrow Instruction Handler
//!
//! Returns the escrowed funds to the buyer, then closes the vault and the
//! escrow account (rent returned to the buyer).
//!
//! ## Who may refund
//! - Anyone, once a `Locked` escrow has reached `timeout_at` (permissionless
//!   crank, so a buyer is never stuck).
//! - The gateway authority, while the escrow is `Disputed` (dispute resolved
//!   in the buyer's favour). A dispute suspends the timeout path.
//!
//! ## Security
//! - The destination must be an SPL token account owned by `escrow.buyer`
//!   with `escrow.mint` (enforced by account constraints), so neither the
//!   caller nor the gateway authority can redirect a refund.
//! - The rent receiver must be `escrow.buyer`.
//! - The vault is constrained to the escrow's PDA vault, mint, and authority;
//!   its full balance is refunded and it is closed.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Transfer};

use crate::errors::PayRogenError;
use crate::state::{EscrowStatus, ESCROW_SEED};
use crate::EscrowRefunded;

/// Handler for the `refund_escrow` instruction.
pub fn handler(ctx: Context<crate::RefundEscrow>) -> Result<()> {
    let escrow = &ctx.accounts.escrow;
    let caller = ctx.accounts.caller.key();
    let now = Clock::get()?.unix_timestamp;

    // ─── Authorization ───────────────────────────────────────────────────────
    match escrow.status {
        EscrowStatus::Locked => {
            require!(now >= escrow.timeout_at, PayRogenError::EscrowNotTimedOut);
        }
        EscrowStatus::Disputed => {
            require_keys_eq!(
                caller,
                ctx.accounts.gateway_config.authority,
                PayRogenError::UnauthorizedSigner
            );
        }
        _ => return err!(PayRogenError::EscrowNotLocked),
    }

    // ─── Refund the full vault balance ───────────────────────────────────────
    let vault_balance = ctx.accounts.escrow_token_account.amount;
    require!(
        vault_balance >= escrow.amount,
        PayRogenError::InsufficientBalance
    );

    let buyer_key = escrow.buyer;
    let order_id = escrow.order_id;
    let bump = [escrow.bump];
    let seeds: &[&[u8]] = &[ESCROW_SEED, buyer_key.as_ref(), order_id.as_ref(), &bump];
    let signer_seeds: &[&[&[u8]]] = &[seeds];

    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.escrow_token_account.to_account_info(),
                to: ctx.accounts.buyer_token_account.to_account_info(),
                authority: ctx.accounts.escrow.to_account_info(),
            },
            signer_seeds,
        ),
        vault_balance,
    )?;

    token::close_account(CpiContext::new_with_signer(
        ctx.accounts.token_program.key(),
        CloseAccount {
            account: ctx.accounts.escrow_token_account.to_account_info(),
            destination: ctx.accounts.buyer_wallet.to_account_info(),
            authority: ctx.accounts.escrow.to_account_info(),
        },
        signer_seeds,
    ))?;

    emit!(EscrowRefunded {
        escrow: ctx.accounts.escrow.key(),
        refunded_by: caller,
        amount: vault_balance,
    });

    // Escrow account is closed by Anchor's `close = buyer_wallet` constraint on exit.
    ctx.accounts.escrow.status = EscrowStatus::Refunded;
    Ok(())
}
