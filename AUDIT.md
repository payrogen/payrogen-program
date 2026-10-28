# PayRogen Program — Audit Notes

This document orients auditors to the trust model, privileged roles, invariants, and
known assumptions of the PayRogen Solana program. It is a guide, not a guarantee —
please verify every claim against the source.

- **Framework:** Anchor `1.1.2`
- **Program entrypoint:** `programs/payrogen/src/lib.rs`
- **State:** `programs/payrogen/src/state.rs`
- **Errors:** `programs/payrogen/src/errors.rs`
- **Instruction handlers:** `programs/payrogen/src/instructions/`

## Accounts (PDAs)

| Account | Seeds | Purpose |
|---|---|---|
| `GatewayConfig` | `[b"gateway_config"]` | Singleton. Stores `authority`, `treasury`, `gateway_fee_bps`, `bump`. |
| `EscrowAccount` | `[b"escrow", buyer, order_id]` | Holds locked funds + split config for one order. |
| `WalletFreeze` | `[b"freeze", wallet]` | Time-boxed freeze blocking a wallet's outgoing transfers. |

## Privileged roles & trust model

- **Gateway authority (`GatewayConfig.authority`, intended to be an Admin multisig).**
  Set once at `initialize_gateway`. It is the only party that can:
  - `update_gateway_fee` — change the fee, **capped on-chain at 500 bps (5%)**
    (`MAX_FEE_BPS` in `update_gateway_fee.rs`); values above the cap are rejected.
  - `freeze_wallet` — freeze a wallet for **24h (86400s)**; auto-expires with no explicit
    unfreeze. Enforced via `has_one = authority` on `GatewayConfig`.
  - Initiate `refund_escrow` (in addition to anyone, once the escrow has timed out).
- **Treasury (`GatewayConfig.treasury`).** Receives the gateway's share of each split.
  It is a stored pubkey, not a signer.
- **Payer / Buyer.** Must sign their own payments and escrow funding.

Auditors should assume the authority is a trusted-but-constrained admin: it can set fees
(within the cap) and freeze wallets, but **cannot** redirect or seize user funds outside
the documented instructions, and cannot exceed the fee cap.

## Core invariants to verify

1. **Splits sum to exactly 10000 bps (100%).** Enforced in `execute_split_payment`,
   `execute_sol_split_payment`, and `create_escrow` (`InvalidSplitSum`). There is no
   separate fee pre-deduction — the gateway fee is one split recipient.
2. **Atomicity.** All transfers in a split happen in one transaction; any failing CPI
   reverts the whole transaction (no partial distribution).
3. **Split count bound.** `MAX_SPLITS = 5`; `remaining_accounts.len()` must equal the
   number of split entries.
4. **Non-zero amount.** `amount > 0` is required.
5. **Escrow uniqueness / no double-lock.** `create_escrow` uses Anchor `init`, so an
   escrow PDA for a given `(buyer, order_id)` can only be created once.
6. **Escrow close.** `release_escrow` and `refund_escrow` close the escrow account
   (`close = ...`), returning rent; verify funds are fully distributed before close.
7. **Freeze enforcement.** `execute_split_payment` / `execute_sol_split_payment` read the
   optional `WalletFreeze` PDA and reject if `expires_at >= now` (`WalletFrozen`).
8. **Authority gating.** `update_gateway_fee` and `freeze_wallet` use
   `has_one = authority` against `GatewayConfig`.

## Areas that warrant particular attention

- **`remaining_accounts` handling** in the split instructions: ordering, writability
  checks, and that each recipient token account corresponds to the intended split entry.
- **Rounding** in basis-point math (`amount * bps / 10000` using `u128` intermediate):
  confirm dust/rounding cannot be exploited or cause the sum of transfers to exceed
  `amount`.
- **`freeze_account` as `UncheckedAccount`** in the split instructions — manual
  deserialization and length checks before reading `expires_at`.
- **Escrow release authorization** — who may trigger release vs. refund, and the
  timeout boundary condition (`timeout_at`).
- **`init_if_needed`** on `WalletFreeze` (re-freeze path) — confirm no reinitialization
  footgun.

## Known assumptions / non-goals

- The program trusts that recipient token accounts passed in `remaining_accounts` are
  valid SPL token accounts for the intended mint; off-chain components (the gateway)
  construct these. Auditors should assess the on-chain consequences of malformed inputs.
- The admin authority is assumed to be a multisig in production; the program enforces a
  single `authority` pubkey and does not implement multisig itself.
- Program IDs: see `lib.rs` `declare_id!` and `Anchor.toml`.

## Build & test

```bash
anchor build
anchor test        # or: cargo test   (Rust unit tests in src/tests.rs)
```

Please reference the exact commit hash in any finding.
