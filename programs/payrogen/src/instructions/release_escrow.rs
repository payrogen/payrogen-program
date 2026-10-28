//! # Release Escrow Instruction Handler
//!
//! Distributes escrowed funds to the recipients stored at creation, then
//! closes the vault and the escrow account (rent returned to the buyer).
//!
//! ## Who may release
//! - The buyer, while the escrow is `Locked` or `Disputed` (delivery confirmed,
//!   or buyer concedes a dispute). Rejected while the buyer's wallet is frozen.
//! - The gateway authority, only while the escrow is `Disputed` (dispute
//!   resolved in the seller's favour).
//!
//! ## Security
//! - `remaining_accounts[i]` must be an SPL token account owned by
//!   `splits[i].recipient` holding `escrow.mint`; the caller cannot redirect
//!   any share.
//! - The vault is constrained to the escrow's PDA vault, mint, and authority.
//! - The whole vault balance is distributed (rounding remainder to the first
//!   entry) and the vault is closed, so no dust or rent is left behind.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Transfer};

use crate::errors::PayRogenError;
use crate::state::{EscrowStatus, ESCROW_SEED};
use crate::validation::{assert_not_frozen, compute_shares, validate_recipient_token_account};
use crate::EscrowReleased;

/// Handler for the `release_escrow` instruction.
pub fn handler<'info>(ctx: Context<'info, crate::ReleaseEscrow<'info>>) -> Result<()> {
    let escrow = &ctx.accounts.escrow;
    let caller = ctx.accounts.authority.key();
    let now = Clock::get()?.unix_timestamp;

    // ─── Authorization ───────────────────────────────────────────────────────
    if caller == escrow.buyer {
        require!(
            escrow.status == EscrowStatus::Locked || escrow.status == EscrowStatus::Disputed,
            PayRogenError::EscrowNotLocked
        );
        assert_not_frozen(
            &ctx.accounts.buyer_freeze_account.to_account_info(),
            &escrow.buyer,
            now,
        )?;
    } else if caller == ctx.accounts.gateway_config.authority {
        require!(
            escrow.status == EscrowStatus::Disputed,
            PayRogenError::EscrowNotDisputed
        );
    } else {
        return err!(PayRogenError::UnauthorizedSigner);
    }

    // ─── Validate every payout account before moving funds ───────────────────
    let splits = escrow.active_splits();
    require!(
        ctx.remaining_accounts.len() == splits.len(),
        PayRogenError::InvalidRecipient
    );
    for (split, account) in splits.iter().zip(ctx.remaining_accounts.iter()) {
        validate_recipient_token_account(account, &split.recipient, &escrow.mint)?;
    }

    // ─── Distribute the full vault balance ───────────────────────────────────
    let vault_balance = ctx.accounts.escrow_token_account.amount;
    require!(
        vault_balance >= escrow.amount,
        PayRogenError::InsufficientBalance
    );
    let bps: Vec<u16> = splits.iter().map(|s| s.basis_points).collect();
    let shares = compute_shares(vault_balance, &bps)?;

    let buyer_key = escrow.buyer;
    let order_id = escrow.order_id;
    let bump = [escrow.bump];
    let seeds: &[&[u8]] = &[ESCROW_SEED, buyer_key.as_ref(), order_id.as_ref(), &bump];
    let signer_seeds: &[&[&[u8]]] = &[seeds];

    for (share, account) in shares.iter().zip(ctx.remaining_accounts.iter()) {
        if *share == 0 {
            continue;
        }
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.escrow_token_account.to_account_info(),
                to: account.to_account_info(),
                authority: ctx.accounts.escrow.to_account_info(),
            },
            signer_seeds,
        );
        token::transfer(cpi_ctx, *share)?;
    }

    // ─── Close the (now empty) vault; rent back to the buyer ─────────────────
    token::close_account(CpiContext::new_with_signer(
        ctx.accounts.token_program.key(),
        CloseAccount {
            account: ctx.accounts.escrow_token_account.to_account_info(),
            destination: ctx.accounts.buyer.to_account_info(),
            authority: ctx.accounts.escrow.to_account_info(),
        },
        signer_seeds,
    ))?;

    emit!(EscrowReleased {
        escrow: ctx.accounts.escrow.key(),
        released_by: caller,
        amount: vault_balance,
    });

    // Escrow account is closed by Anchor's `close = buyer` constraint on exit.
    ctx.accounts.escrow.status = EscrowStatus::Released;
    Ok(())
}
