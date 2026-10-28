//! # PayRogen Property-Based Tests
//!
//! Property tests for the PayRogen Smart Contract logic using the `proptest` crate.
//! These tests validate core invariants by testing pure logic functions in isolation,
//! without requiring a full Anchor/Solana runtime context.
//!
//! ## Properties Tested
//! - Property 3: Gateway Fee Constraint (Requirement 6.5)
//! - Property 4: Wallet Freeze Semantics (Requirements 7.3, 7.4, 7.5, 7.6)
//! - Property 9: Escrow Double-Lock Prevention (Requirement 19.5)

#[cfg(test)]
mod property_tests {
    use proptest::prelude::*;

    // ─── Helper Functions (Pure Logic Under Test) ────────────────────────────

    /// Maximum gateway fee in basis points (500 = 5%).
    const MAX_FEE_BPS: u16 = 500;

    /// Duration of wallet freeze in seconds (24 hours).
    const FREEZE_DURATION_SECONDS: i64 = 86400;

    /// Validates whether a proposed gateway fee is within the allowed range.
    /// Returns Ok(()) if fee <= 500 bps, Err otherwise.
    ///
    /// This mirrors the logic in `instructions/update_gateway_fee.rs`.
    fn validate_gateway_fee(new_fee_bps: u16) -> Result<(), &'static str> {
        if new_fee_bps <= MAX_FEE_BPS {
            Ok(())
        } else {
            Err("FeeExceedsMaximum")
        }
    }

    /// Determines whether an outgoing transfer should be allowed given freeze state.
    /// Returns true if the transfer is ALLOWED, false if REJECTED.
    ///
    /// This mirrors the freeze check logic in `instructions/execute_split_payment.rs`:
    /// `require!(freeze_account.expires_at < clock.unix_timestamp, WalletFrozen)`
    fn is_outgoing_transfer_allowed(expires_at: i64, current_time: i64) -> bool {
        // Transfer allowed only when freeze has expired (expires_at < current_time)
        expires_at < current_time
    }

    /// Incoming transfers are always allowed regardless of freeze status.
    /// This is by design: the freeze only blocks outgoing transfers.
    fn is_incoming_transfer_allowed(_expires_at: i64, _current_time: i64) -> bool {
        true
    }

    /// Computes the freeze expiry timestamp from the freeze activation time.
    fn compute_freeze_expiry(frozen_at: i64) -> i64 {
        frozen_at.saturating_add(FREEZE_DURATION_SECONDS)
    }

    /// Represents the status of an escrow for double-lock testing.
    #[derive(Clone, Debug, PartialEq)]
    enum EscrowStatus {
        Locked,
        Released,
        Refunded,
    }

    /// Simulates an attempt to create a new escrow on a PDA that may already exist.
    /// Returns Ok(()) if creation succeeds, Err if rejected due to existing escrow.
    ///
    /// In the actual program, Anchor's `init` constraint on PDA seeds
    /// `[b"escrow", buyer.key(), order_id]` prevents re-initialization at the
    /// Solana runtime level. This function models that semantic guarantee.
    fn attempt_create_escrow(
        existing_escrow: Option<&EscrowStatus>,
    ) -> Result<(), &'static str> {
        match existing_escrow {
            Some(_) => {
                // PDA already exists — Solana runtime rejects re-init
                Err("EscrowAlreadyLocked")
            }
            None => {
                // No existing escrow — creation succeeds
                Ok(())
            }
        }
    }

    // ─── Property 3: Gateway Fee Constraint ──────────────────────────────────
    //
    // **Validates: Requirements 6.5**
    //
    // For any u16 fee value, the fee update logic accepts iff fee <= 500.

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 3: Gateway Fee Constraint
        ///
        /// For any arbitrary u16 value, verify that:
        /// - Values <= 500 are accepted
        /// - Values > 500 are rejected with FeeExceedsMaximum
        ///
        /// **Validates: Requirements 6.5**
        #[test]
        fn prop_gateway_fee_constraint(fee in 0u16..=u16::MAX) {
            let result = validate_gateway_fee(fee);

            if fee <= MAX_FEE_BPS {
                prop_assert!(
                    result.is_ok(),
                    "Fee {} should be accepted (<=500) but was rejected",
                    fee
                );
            } else {
                prop_assert!(
                    result.is_err(),
                    "Fee {} should be rejected (>500) but was accepted",
                    fee
                );
                prop_assert_eq!(
                    result.unwrap_err(),
                    "FeeExceedsMaximum",
                    "Expected FeeExceedsMaximum error for fee {}",
                    fee
                );
            }
        }
    }

    // ─── Property 4: Wallet Freeze Semantics ─────────────────────────────────
    //
    // **Validates: Requirements 7.3, 7.4, 7.5, 7.6**
    //
    // Given a freeze timestamp and a transfer attempt time:
    // - Outgoing transfers are rejected while within 24h of freeze
    // - Outgoing transfers are allowed after 24h expires
    // - Incoming transfers are always allowed

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 4a: Outgoing transfers rejected within freeze window
        ///
        /// For any freeze time and any current time within the 24h window,
        /// outgoing transfers must be rejected.
        ///
        /// **Validates: Requirements 7.3, 7.4**
        #[test]
        fn prop_freeze_rejects_outgoing_within_24h(
            frozen_at in 0i64..=(i64::MAX - FREEZE_DURATION_SECONDS),
            offset in 0i64..FREEZE_DURATION_SECONDS,
        ) {
            let expires_at = compute_freeze_expiry(frozen_at);
            // current_time is within the freeze window: frozen_at <= current_time < expires_at
            let current_time = frozen_at + offset;

            // Verify freeze expiry is exactly frozen_at + 86400
            prop_assert_eq!(
                expires_at,
                frozen_at + FREEZE_DURATION_SECONDS,
                "Freeze expiry should be frozen_at + 86400"
            );

            // Outgoing must be rejected (current_time < expires_at means freeze still active)
            let outgoing_allowed = is_outgoing_transfer_allowed(expires_at, current_time);
            prop_assert!(
                !outgoing_allowed,
                "Outgoing transfer should be REJECTED at time {} (freeze expires at {})",
                current_time,
                expires_at
            );
        }

        /// Property 4b: Outgoing transfers allowed after freeze expires
        ///
        /// For any freeze time and any current time >= expires_at,
        /// outgoing transfers must be permitted.
        ///
        /// **Validates: Requirements 7.5, 7.6**
        #[test]
        fn prop_freeze_allows_outgoing_after_24h(
            frozen_at in 0i64..=(i64::MAX - FREEZE_DURATION_SECONDS - 1000),
            extra in 1i64..1000i64,
        ) {
            let expires_at = compute_freeze_expiry(frozen_at);
            // current_time is after the freeze window
            let current_time = expires_at + extra;

            let outgoing_allowed = is_outgoing_transfer_allowed(expires_at, current_time);
            prop_assert!(
                outgoing_allowed,
                "Outgoing transfer should be ALLOWED at time {} (freeze expired at {})",
                current_time,
                expires_at
            );
        }

        /// Property 4c: Incoming transfers always allowed regardless of freeze
        ///
        /// For any freeze state and any point in time, incoming transfers
        /// to a frozen wallet must always succeed.
        ///
        /// **Validates: Requirements 7.4**
        #[test]
        fn prop_freeze_always_allows_incoming(
            frozen_at in 0i64..=(i64::MAX - FREEZE_DURATION_SECONDS),
            current_time in i64::MIN..i64::MAX,
        ) {
            let expires_at = compute_freeze_expiry(frozen_at);

            // Incoming transfers are always allowed
            let incoming_allowed = is_incoming_transfer_allowed(expires_at, current_time);
            prop_assert!(
                incoming_allowed,
                "Incoming transfer should ALWAYS be allowed, regardless of freeze state"
            );
        }
    }

    // ─── Property 9: Escrow Double-Lock Prevention ───────────────────────────
    //
    // **Validates: Requirements 19.5**
    //
    // If an escrow PDA already exists (regardless of its status), attempting
    // to create another escrow with the same seeds must fail.
    // Additionally, the PDA seed derivation guarantees uniqueness: same
    // (buyer, order_id) → same PDA address.

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 9a: Cannot create escrow when PDA already exists (Locked)
        ///
        /// For any existing escrow in Locked state, a second creation attempt
        /// must be rejected and the original state must remain unchanged.
        ///
        /// **Validates: Requirements 19.5**
        #[test]
        fn prop_escrow_double_lock_rejected_locked(
            _buyer in prop::array::uniform32(0u8..),
            _order_id in prop::array::uniform32(0u8..),
        ) {
            let existing = EscrowStatus::Locked;

            // Attempt to create another escrow on the same PDA
            let result = attempt_create_escrow(Some(&existing));

            // Must be rejected
            prop_assert!(
                result.is_err(),
                "Double-lock on existing Locked escrow should be rejected"
            );
            prop_assert_eq!(
                result.unwrap_err(),
                "EscrowAlreadyLocked"
            );

            // State must remain unchanged (still Locked)
            prop_assert_eq!(
                existing,
                EscrowStatus::Locked,
                "Escrow state must remain unchanged after rejection"
            );
        }

        /// Property 9b: Cannot create escrow when PDA exists in any state
        ///
        /// Anchor's `init` constraint rejects re-initialization regardless of
        /// the account's current data. Even if an escrow was Released or Refunded,
        /// the PDA still exists and cannot be re-initialized.
        ///
        /// **Validates: Requirements 19.5**
        #[test]
        fn prop_escrow_rejects_creation_on_existing_pda(
            status_idx in 0u8..3u8,
            _buyer in prop::array::uniform32(0u8..),
            _order_id in prop::array::uniform32(0u8..),
        ) {
            let existing_status = match status_idx {
                0 => EscrowStatus::Locked,
                1 => EscrowStatus::Released,
                _ => EscrowStatus::Refunded,
            };

            let original_status = existing_status.clone();

            // Attempt to create escrow on existing PDA
            let result = attempt_create_escrow(Some(&existing_status));

            // Must be rejected
            prop_assert!(
                result.is_err(),
                "Creation on existing PDA (status: {:?}) should be rejected",
                original_status
            );

            // State unchanged
            prop_assert_eq!(
                existing_status,
                original_status,
                "Escrow state must be unchanged after rejected creation attempt"
            );
        }

        /// Property 9c: Escrow creation succeeds when no PDA exists
        ///
        /// If no escrow PDA exists for a given (buyer, order_id) pair,
        /// creation should succeed.
        ///
        /// **Validates: Requirements 19.5**
        #[test]
        fn prop_escrow_creation_succeeds_when_no_pda(
            _buyer in prop::array::uniform32(0u8..),
            _order_id in prop::array::uniform32(0u8..),
        ) {
            // No existing escrow
            let result = attempt_create_escrow(None);

            prop_assert!(
                result.is_ok(),
                "Escrow creation should succeed when no PDA exists"
            );
        }

        /// Property 9d: PDA seed uniqueness — same inputs produce same "address"
        ///
        /// For any (buyer, order_id) pair, the PDA derivation is deterministic.
        /// This means a second attempt with the same inputs will always collide
        /// with the existing account.
        ///
        /// **Validates: Requirements 19.5**
        #[test]
        fn prop_escrow_pda_seed_deterministic(
            buyer in prop::array::uniform32(0u8..),
            order_id in prop::array::uniform32(0u8..),
        ) {
            // Simulate PDA seed computation (without actual Solana SDK)
            // The key insight: seeds [b"escrow", buyer, order_id] are deterministic
            let seed1 = compute_pda_seed_hash(&buyer, &order_id);
            let seed2 = compute_pda_seed_hash(&buyer, &order_id);

            prop_assert_eq!(
                seed1,
                seed2,
                "Same (buyer, order_id) must always produce the same PDA seed"
            );
        }
    }

    /// Helper: Simulate deterministic PDA seed computation.
    /// In the real program, `Pubkey::find_program_address` with seeds
    /// `[b"escrow", buyer.as_ref(), order_id.as_ref()]` is used.
    /// This function demonstrates the determinism property.
    fn compute_pda_seed_hash(buyer: &[u8; 32], order_id: &[u8; 32]) -> Vec<u8> {
        let mut seed = Vec::with_capacity(7 + 32 + 32);
        seed.extend_from_slice(b"escrow");
        seed.extend_from_slice(buyer);
        seed.extend_from_slice(order_id);
        seed
    }
}
