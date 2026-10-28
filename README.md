# PayRogen Solana Program

The on-chain Solana program (Anchor) powering the PayRogen payment gateway. It enforces
atomic payment splitting, escrow, dynamic gateway fees, and wallet freezing for a
non-custodial, zero-knowledge payment flow.

This repository is published for **security auditing and public review**.

## Program IDs

| Cluster | Program ID |
|---|---|
| Devnet (localnet build) | `AeJVPDMxcSvn89F5kgiita5cndcQvamEnCSvY38HeYFg` |

> The ID in `Anchor.toml` under `[programs.devnet]` is the Anchor default placeholder; the
> deployed program ID is declared via `declare_id!` in `programs/payrogen/src/lib.rs`.

## Instructions

| Instruction | Purpose |
|---|---|
| `initialize_gateway` | One-time setup of the `GatewayConfig` PDA (admin authority, treasury, fee). |
| `execute_split_payment` | Atomic SPL-token distribution to N recipients; basis points must sum to 10000. |
| `execute_sol_split_payment` | Same, for native SOL. |
| `create_escrow` | Lock funds in an escrow PDA with a split config and timeout. |
| `release_escrow` | Distribute escrowed funds per the split config (buyer-confirmed). |
| `refund_escrow` | Return escrowed funds to the buyer on timeout / dispute. |
| `freeze_wallet` | Admin-only 24h freeze blocking outgoing transfers. |
| `update_gateway_fee` | Admin-only fee update, capped on-chain. |

## Payment split model

Every payment is distributed in a **single atomic transaction** whose recipient shares
(in basis points) **sum to exactly 10000 (100%)**. There is no separate pre-deduction —
the gateway fee is one of the split recipients. A typical three-way split:

- **Gateway fee** — e.g. 1.5% (150 bps), from on-chain `GatewayConfig`
- **Platform commission** — e.g. 5% (500 bps), set by the integrating platform
- **Seller** — the remainder, e.g. 93.5% (9350 bps)

Solana's runtime guarantees all CPI transfers in the transaction succeed together or the
whole transaction reverts (all-or-nothing).

## Repository layout

```
Anchor.toml                      # Anchor workspace config
Cargo.toml / Cargo.lock          # Rust workspace manifest + pinned deps
programs/payrogen/
  Cargo.toml
  src/
    lib.rs                       # Program entrypoint, account structs, shared types
    state.rs                     # GatewayConfig, EscrowAccount, WalletFreeze, SplitEntry
    errors.rs                    # Custom program errors
    instructions/                # One file per instruction handler
    tests.rs                     # Unit tests
```

## Build & test

Requires the Solana toolchain and Anchor (version pinned in `Anchor.toml`).

```bash
# Build the program
anchor build

# Run tests
anchor test
# or, for Rust unit tests only:
cargo test
```

## Security notes for auditors

- Split basis points are validated on-chain to sum to exactly `10000`.
- Escrow PDAs use `init` to prevent double-locking; accounts are closed on
  release/refund to reclaim rent.
- Admin-gated instructions (`freeze_wallet`, `update_gateway_fee`) enforce
  `has_one = authority` against the `GatewayConfig` PDA.
- The gateway fee is capped on-chain in `update_gateway_fee`.
- No private keys, secrets, or environment configuration are contained in this repository.

Please report findings via a GitHub issue or responsibly to the PayRogen team.

## License

See [LICENSE](LICENSE).
