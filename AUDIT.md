# PayRogen Program — Audit Notes

This document orients auditors to the trust model, privileged roles, invariants, and
known limitations of the PayRogen Solana program. It is a guide, not a guarantee.
Verify every claim against the source and reference the exact commit hash in findings.

- **Framework:** Anchor `1.1.2`
- **Program entrypoint and account structs:** `programs/payrogen/src/lib.rs`
- **State:** `programs/payrogen/src/state.rs`
- **Shared checks:** `programs/payrogen/src/validation.rs`
- **Errors:** `programs/payrogen/src/errors.rs`
- **Instruction handlers:** `programs/payrogen/src/instructions/`
- **Tests:** `programs/payrogen/src/tests.rs` (unit/property), `tests-litesvm/` (end-to-end
  against the compiled SBF binary)

## Accounts (PDAs)

| Account | Seeds | Purpose |
|---|---|---|
| `GatewayConfig` | `[b"gateway_config"]` | Singleton: `authority`, `treasury`, `gateway_fee_bps`, `bump`. Fixed 75-byte layout read at fixed offsets by clients. |
| `PendingAuthority` | `[b"pending_authority"]` | Nominee for the two-step authority rotation. |
| `EscrowAccount` | `[b"escrow", buyer, order_id]` | Escrow state: parties, amount, status, timeout, splits, `mint`, `vault_bump`. |
| Escrow vault (SPL token account) | `[b"vault", escrow]` | Holds escrowed tokens. Token authority = escrow PDA. Created by `create_escrow`, closed on release/refund. |
| `WalletFreeze` | `[b"freeze", wallet]` | 24h freeze of a wallet's outgoing transfers through this program. |

## Privileged roles and trust model

**Program upgrade authority.** Can replace the program code and therefore everything
below. It must be a multisig in production. It is also the only key that can call
`initialize_gateway`.

**Gateway authority (`GatewayConfig.authority`).** Can:

- `update_gateway_fee`: change the fee, capped on-chain at 500 bps (`MAX_FEE_BPS`).
- `set_treasury`: change the fee treasury.
- `propose_authority`: nominate a new authority (the nominee must sign `accept_authority`).
- `freeze_wallet`: freeze a wallet for 24h; it auto-expires.
- Resolve a **disputed** escrow, either by `release_escrow` (pay the stored split
  recipients) or by `refund_escrow` (return funds to the buyer).

The gateway authority **cannot**:

- move escrowed funds anywhere other than the split recipients fixed at escrow creation
  or the buyer's own token account for the escrow mint;
- release or refund an escrow that is not disputed;
- move funds in direct split payments (those are signed by the payer);
- set a fee above 5%.

**Treasury (`GatewayConfig.treasury`).** Must receive at least `gateway_fee_bps` of
every split payment and escrow. It is a stored pubkey, not a signer.

**Payer / buyer.** Signs their own payments and escrow funding. The buyer confirms
delivery with `release_escrow`.

**Seller.** Must be one of the escrow split recipients. Can raise a dispute.

## Escrow lifecycle

| From | Action | Who | Condition | To |
|---|---|---|---|---|
| — | `create_escrow` | buyer | not frozen; splits valid; timeout in (0, 90 days] | `Locked` |
| `Locked` | `release_escrow` | buyer | buyer not frozen | closed (paid out) |
| `Locked` | `refund_escrow` | anyone | `now >= timeout_at` | closed (refunded) |
| `Locked` | `raise_dispute` | buyer or seller | `now < timeout_at` | `Disputed` |
| `Disputed` | `release_escrow` | buyer or gateway authority | buyer not frozen (buyer path) | closed (paid out) |
| `Disputed` | `refund_escrow` | gateway authority | — | closed (refunded) |

A dispute suspends the permissionless timeout refund. On release or refund the whole
vault balance is moved (including any tokens donated to the vault), the vault is closed,
and the escrow account is closed. All rent returns to the buyer.

## Core invariants

1. **Splits.** 1 to 5 entries, no default pubkey, shares sum to exactly 10000 bps.
   Zero-bps entries are allowed and receive nothing (`validation::validate_splits`).
   Escrow splits also forbid duplicate recipients, the escrow PDA as a recipient, and
   require the seller to be a recipient.
2. **Gateway fee is enforced on-chain.** Entries whose recipient equals
   `GatewayConfig.treasury` must together carry at least `gateway_fee_bps`
   (`GatewayFeeNotPaid`). Checked in both split instructions and `create_escrow`.
3. **Recipients are bound to the instruction data or escrow state.**
   - SPL (`execute_split_payment`, `release_escrow`): `remaining_accounts[i]` must be
     writable, owned by the SPL Token program, an initialized token account whose
     owner is `splits[i].recipient` and whose mint is the payment mint (payer's mint, or
     `escrow.mint`).
   - SOL (`execute_sol_split_payment`): `remaining_accounts[i].key == splits[i].recipient`.
   - Refund: destination has `token::mint = escrow.mint` and
     `token::authority = escrow.buyer`; rent receiver must equal `escrow.buyer`.
4. **Exact amounts.** Each share is `floor(amount * bps / 10000)` (u128 intermediate);
   the remainder is added to the first entry, so exactly `amount` moves and no dust
   remains (`validation::compute_shares`, property-tested).
5. **Escrow custody.** The vault is created by the program at `[b"vault", escrow]` with
   the escrow PDA as authority and the recorded mint. Release and refund constrain the
   vault by seeds, bump, mint, and authority.
6. **Atomicity.** Every payout account is validated before any transfer. Any failing
   CPI reverts the whole transaction.
7. **Freeze.** `validation::assert_not_frozen` accepts an empty freeze PDA; otherwise it
   requires a program-owned `WalletFreeze` (discriminator checked) for the same wallet
   and rejects while `now <= expires_at`. Enforced in both split instructions,
   `create_escrow`, and buyer-initiated `release_escrow`.
8. **Initialization.** `initialize_gateway` requires the program's ProgramData account
   and checks `upgrade_authority_address == Some(signer)`.
9. **Authority gating.** Fee, treasury, proposal, and freeze instructions use
   `has_one = authority`. `accept_authority` requires the nominee's signature.

## Remediation log

The findings below came from an internal review of commit `fa426b8`. All are fixed in
this revision and covered by tests in `tests-litesvm/`. The split-payment and admin
tests were also run against a build of `fa426b8`: the H-1, H-2, M-1, M-3, and L-1 tests
fail there, and the merchant-ui-shaped two-way payment passes on both builds. The escrow
tests cannot run against `fa426b8` because its escrow account layout differs.

| ID | Severity | Finding | Fix | Tests |
|---|---|---|---|---|
| C-1 | Critical | `refund_escrow` did not check the destination token account, so after timeout anyone (or the authority at any time) could take the refund. | Destination constrained to `token::authority = escrow.buyer`, `token::mint = escrow.mint`. Authority refunds limited to disputed escrows. | `timeout_refund_cannot_be_redirected`, `gateway_authority_cannot_redirect_refund`, `refund_before_timeout_is_rejected_even_for_authority` |
| C-2 | Critical | `release_escrow` only checked that payout accounts were writable, so the buyer could pay themselves. | Each payout account must be an SPL account owned by the stored recipient with `escrow.mint`. | `release_rejects_redirected_payouts`, `authority_resolves_dispute_for_seller` |
| C-3 | Critical | `create_escrow` accepted any token account as the vault and stored no mint. | Program-created PDA vault (`[b"vault", escrow]`, authority = escrow); `mint` stored and enforced. | `create_rejects_attacker_controlled_vault`, `create_rejects_mint_mismatch`, `release_rejects_substituted_vault` |
| H-1 | High | `initialize_gateway` could be front-run after deployment. | Only the program upgrade authority can initialize. | `initialize_rejects_non_upgrade_authority`, `initialize_rejects_immutable_program` |
| H-2 | High | The gateway fee was not enforced on-chain. | Treasury entries must carry at least `gateway_fee_bps`. | `spl_split_enforces_gateway_fee`, `sol_split_rejects_substitution_and_fee_bypass`, `fee_increase_rejects_stale_split`, `create_validates_splits_fee_seller_and_timeout` |
| H-3 | High | `Disputed` was unreachable; the authority could refund any escrow at any time; docs said "auto-release". | Added `raise_dispute`; authority powers limited to disputed escrows; timeout refunds the buyer; docs aligned. | `dispute_*`, `authority_resolves_dispute_for_seller`, `buyer_can_concede_dispute` |
| M-1 | Medium | SPL split recipients were not bound to the instruction data. | Owner and mint checks on every recipient account. | `spl_split_rejects_recipient_account_substitution`, `spl_split_rejects_wrong_mint_and_non_token_accounts` |
| M-2 | Medium | `create_escrow` ignored freezes; freeze scope undocumented. | Freeze checked in `create_escrow` and buyer `release_escrow`; scope documented. | `create_rejects_frozen_buyer`, `frozen_buyer_cannot_release` |
| M-3 | Medium | No way to rotate the authority or treasury. | `propose_authority` / `accept_authority` (two-step) and `set_treasury`, without changing the `GatewayConfig` layout. | `two_step_authority_rotation`, `set_treasury_is_authority_only_and_enforced` |
| L-1 | Low | Rounding dust stayed with the payer or in the vault; the vault was never closed; donations could block closing. | Remainder to the first entry; full vault balance distributed; vault closed. | `spl_split_merchant_ui_three_way_exact_amounts`, `donated_tokens_cannot_block_release`, `prop_shares_conserve_amount` |
| L-2 | Low | Unbounded escrow timeout. | Capped at 90 days (`InvalidTimeout`). | `create_validates_splits_fee_seller_and_timeout` |
| L-3 | Low | Freeze state was read from raw bytes without owner or discriminator checks. | Owner, discriminator, and wallet checked; fails closed. | `spl_split_respects_freeze_and_expiry`, `sol_split_respects_freeze` |
| Info | Info | `amount == 0` reported as `InsufficientBalance`; `seller`/`merchant` unused; `InvalidMultiSig` unused; tests did not exercise handlers. | `InvalidAmount`; seller must be a recipient and may dispute; `merchant` documented as informational; `InvalidMultiSig` kept as reserved so error codes stay stable; LiteSVM suite added. | all |

## Client compatibility

- `execute_split_payment` and `execute_sol_split_payment` keep their discriminators,
  argument encoding, and account lists. Existing clients that already pass the
  treasury entry from `GatewayConfig` and the recipients' token accounts work unchanged.
- Clients must take the treasury from `GatewayConfig` (bytes 40..72) and the fee from
  bytes 72..74. A transaction built before a fee increase fails with `GatewayFeeNotPaid`
  and must be rebuilt.
- The escrow instructions changed their account lists (mint, vault PDA, freeze PDA,
  caller/buyer split). See the IDL.
- New error variants were appended (codes 6010 to 6017). Existing codes 6000 to 6009 did
  not move.

## Known limitations and non-goals

- **Freeze scope.** A freeze only blocks transfers through this program. The wallet
  owner can still sign plain SPL or System transfers elsewhere.
- **Token-2022 is not supported.** Mints and token accounts must belong to the classic
  SPL Token program.
- **Disputes have no on-chain deadline.** A disputed escrow waits for the gateway
  authority (or the buyer releasing). If the authority key is lost, rotate it in advance
  or upgrade the program.
- **Escrow re-creation.** After an escrow is released or refunded its PDA is closed, so
  the same `(buyer, order_id)` can be funded again. Off-chain systems must not treat the
  PDA address alone as a permanent order record.
- **Multisig is external.** The program enforces a single `authority` pubkey. Use a
  multisig vault (for example Squads) as both the gateway authority and the upgrade
  authority.
- **Deployment.** Fixes apply only once the upgraded binary is deployed. Check the
  deployed program's last-deployed slot against this commit before relying on them.

## Build and test

```bash
anchor build                                         # program + IDL
cargo test --manifest-path programs/payrogen/Cargo.toml   # unit + property tests
cargo test --manifest-path tests-litesvm/Cargo.toml       # end-to-end (LiteSVM)
# or both: anchor run test
```

Do not run `anchor test` with the devnet provider in `Anchor.toml`; it deploys.
To run the LiteSVM suite against another binary, set `PAYROGEN_SO=/path/to/payrogen.so`.
