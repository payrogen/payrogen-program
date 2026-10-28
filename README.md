# PayRogen Solana Program

The on-chain Solana program (Anchor) behind the PayRogen payment gateway. It enforces
atomic payment splitting, escrow, an on-chain gateway fee, and wallet freezing for a
non-custodial payment flow.

This repository is published for **security auditing and public review**.

## Program IDs

| Cluster | Program ID |
|---|---|
| Devnet | `AeJVPDMxcSvn89F5kgiita5cndcQvamEnCSvY38HeYFg` |

The same ID is declared in `programs/payrogen/src/lib.rs` (`declare_id!`) and in
`Anchor.toml`.

## Instructions

| Instruction | Who | Purpose |
|---|---|---|
| `initialize_gateway` | program upgrade authority | One-time setup of `GatewayConfig` (authority, treasury, fee). |
| `execute_split_payment` | payer | Atomic SPL token payment to up to 5 recipients; shares sum to 10000 bps and include the gateway fee. |
| `execute_sol_split_payment` | payer | Same, for native SOL. |
| `create_escrow` | buyer | Lock SPL tokens in a program-owned vault with a split config and timeout (max 90 days). |
| `release_escrow` | buyer, or gateway authority if disputed | Pay the stored split recipients and close the escrow. |
| `refund_escrow` | anyone after timeout, or gateway authority if disputed | Return the funds to the buyer and close the escrow. |
| `raise_dispute` | buyer or seller, before timeout | Pause the timeout until the dispute is resolved. |
| `freeze_wallet` | gateway authority | 24h freeze of a wallet's outgoing transfers through this program. |
| `update_gateway_fee` | gateway authority | Change the fee, capped at 500 bps (5%). |
| `propose_authority` / `accept_authority` | current / nominated authority | Two-step authority rotation. |
| `set_treasury` | gateway authority | Change the fee treasury. |

## Payment split model

Every payment is distributed in a **single atomic transaction** whose recipient shares
(in basis points) **sum to exactly 10000 (100%)**. There is no separate pre-deduction;
the gateway fee is one of the split recipients, and the program rejects any split that
pays the treasury less than the on-chain fee. A typical three-way split:

- **Gateway fee:** e.g. 1.5% (150 bps), from on-chain `GatewayConfig`
- **Platform commission:** e.g. 5% (500 bps), set by the integrating platform
- **Seller:** the remainder, e.g. 93.5% (9350 bps)

Each share is rounded down and the rounding remainder goes to the first recipient, so
exactly the requested amount moves. Each recipient account is checked against the
recipient named in the split.

## Repository layout

```
Anchor.toml                      # Anchor workspace config
Cargo.toml / Cargo.lock          # Rust workspace manifest + pinned deps
programs/payrogen/
  src/
    lib.rs                       # Entrypoint, account structs, events
    state.rs                     # Accounts and constants
    validation.rs                # Shared security checks (splits, fee, recipients, freeze)
    errors.rs                    # Program errors (append-only)
    instructions/                # One file per instruction handler
    tests.rs                     # Unit + property tests of validation.rs
tests-litesvm/                   # End-to-end tests against the compiled program (LiteSVM)
```

## Build and test

Requires the Solana toolchain and Anchor `1.1.2` (pinned in `Anchor.toml`).

```bash
anchor build
anchor run test      # unit/property tests + LiteSVM end-to-end suite
```

Do not run `anchor test` with the devnet provider configured in `Anchor.toml`; it
deploys the program.

## For auditors

- **[AUDIT.md](AUDIT.md):** trust model, escrow lifecycle, invariants, remediation log,
  and known limitations.
- **[SECURITY.md](SECURITY.md):** how to report vulnerabilities (private disclosure and
  safe harbor).

No private keys, secrets, or environment configuration are contained in this repository.

## License

See [LICENSE](LICENSE).
