//! # PayRogen Unit and Property Tests
//!
//! These tests call the program's real validation helpers
//! (`crate::validation`). End-to-end tests that execute every instruction
//! handler against the compiled SBF binary live in `tests-litesvm/`.

#[cfg(test)]
mod unit {
    use anchor_lang::prelude::*;
    use proptest::prelude::*;

    use crate::errors::PayRogenError;
    use crate::state::{
        EscrowAccount, EscrowStatus, GatewayConfig, SplitConfig, WalletFreeze, MAX_SPLITS,
    };
    use crate::validation::{compute_shares, validate_splits};
    use crate::SplitEntry;

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    fn entry(n: u8, bps: u16) -> SplitEntry {
        SplitEntry { recipient: key(n), basis_points: bps }
    }

    fn code(e: anchor_lang::error::Error) -> u32 {
        match e {
            anchor_lang::error::Error::AnchorError(a) => a.error_code_number,
            anchor_lang::error::Error::ProgramError(p) => panic!("unexpected program error {p:?}"),
        }
    }

    fn expect(res: Result<()>, err: PayRogenError) {
        let want: u32 = err.into();
        assert_eq!(code(res.expect_err("expected error")), want);
    }

    const TREASURY: u8 = 200;

    // ─── validate_splits ─────────────────────────────────────────────────────

    #[test]
    fn merchant_ui_shapes_are_accepted() {
        // 2-way: merchant 9850 + treasury 150
        validate_splits(&[entry(1, 9850), entry(TREASURY, 150)], &key(TREASURY), 150, true).unwrap();
        // 3-way: seller 9350 + treasury 150 + platform 500
        validate_splits(
            &[entry(1, 9350), entry(TREASURY, 150), entry(2, 500)],
            &key(TREASURY),
            150,
            true,
        )
        .unwrap();
        // fee 0 needs no treasury entry
        validate_splits(&[entry(1, 10_000)], &key(TREASURY), 0, true).unwrap();
    }

    #[test]
    fn split_shape_rejections() {
        let t = key(TREASURY);
        expect(validate_splits(&[], &t, 0, true), PayRogenError::InvalidSplitSum);
        let six: Vec<_> = (1..=6).map(|i| entry(i, if i == 1 { 5000 } else { 1000 })).collect();
        expect(validate_splits(&six, &t, 0, true), PayRogenError::InvalidSplitSum);
        expect(validate_splits(&[entry(1, 9999)], &t, 0, true), PayRogenError::InvalidSplitSum);
        // Zero-bps entries are allowed (merchant-ui sends a 0-bps treasury
        // entry when the fee is 0).
        validate_splits(&[entry(1, 10_000), entry(TREASURY, 0)], &t, 0, true).unwrap();
        expect(
            validate_splits(&[entry(1, 10_000), entry(TREASURY, 0)], &t, 1, true),
            PayRogenError::GatewayFeeNotPaid,
        );
        expect(
            validate_splits(&[entry(0, 10_000)], &t, 0, true),
            PayRogenError::InvalidRecipient,
        );
        expect(
            validate_splits(&[entry(1, 5_000), entry(1, 5_000)], &t, 0, false),
            PayRogenError::InvalidRecipient,
        );
        validate_splits(&[entry(1, 5_000), entry(1, 5_000)], &t, 0, true).unwrap();
        // u16 overflow attempt: 65535 * 2 != 10000 is caught by u32 sum
        expect(
            validate_splits(&[entry(1, u16::MAX), entry(2, u16::MAX)], &t, 0, true),
            PayRogenError::InvalidSplitSum,
        );
    }

    #[test]
    fn gateway_fee_enforced() {
        let t = key(TREASURY);
        expect(validate_splits(&[entry(1, 10_000)], &t, 150, true), PayRogenError::GatewayFeeNotPaid);
        expect(
            validate_splits(&[entry(1, 9_851), entry(TREASURY, 149)], &t, 150, true),
            PayRogenError::GatewayFeeNotPaid,
        );
        // Split across two treasury entries still counts.
        validate_splits(&[entry(1, 9_850), entry(TREASURY, 100), entry(TREASURY, 50)], &t, 150, true)
            .unwrap();
    }

    // ─── compute_shares ──────────────────────────────────────────────────────

    #[test]
    fn shares_examples() {
        assert_eq!(compute_shares(1_000_000, &[9850, 150]).unwrap(), vec![985_000, 15_000]);
        // 20_000_003 * 150 / 10000 = 3000 (floor), 500 bps = 1_000_000 (floor)
        assert_eq!(
            compute_shares(20_000_003, &[9350, 150, 500]).unwrap(),
            vec![18_700_003, 300_000, 1_000_000]
        );
        assert_eq!(compute_shares(1, &[9850, 150]).unwrap(), vec![1, 0]);
        assert_eq!(compute_shares(u64::MAX, &[10_000]).unwrap(), vec![u64::MAX]);
        assert!(compute_shares(10, &[9_000]).is_err());
        assert!(compute_shares(10, &[]).is_err());
    }

    fn bps_strategy() -> impl Strategy<Value = Vec<u16>> {
        // 1..=MAX_SPLITS positive shares summing to 10000.
        prop::collection::vec(1u16..=10_000, 1..=MAX_SPLITS).prop_filter_map("sum", |mut v| {
            let n = v.len() as u32;
            let total: u32 = v.iter().map(|b| *b as u32).sum();
            // Scale into 10000 keeping every entry >= 1.
            let mut acc = 0u32;
            for b in v.iter_mut() {
                let scaled = ((*b as u32) * (10_000 - n) / total) + 1;
                *b = scaled as u16;
                acc += scaled;
            }
            if acc > 10_000 {
                return None;
            }
            v[0] += (10_000 - acc) as u16;
            Some(v)
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// Shares always sum to exactly `amount`; each non-first share is the
        /// exact floor; the first gets floor + remainder (< number of entries).
        #[test]
        fn prop_shares_conserve_amount(amount in any::<u64>(), bps in bps_strategy()) {
            let shares = compute_shares(amount, &bps).unwrap();
            let total: u128 = shares.iter().map(|s| *s as u128).sum();
            prop_assert_eq!(total, amount as u128);
            let floor = |b: u16| ((amount as u128) * (b as u128) / 10_000) as u64;
            for (i, b) in bps.iter().enumerate().skip(1) {
                prop_assert_eq!(shares[i], floor(*b));
            }
            let remainder = shares[0] - floor(bps[0]);
            prop_assert!((remainder as usize) < bps.len());
        }

        /// The treasury never receives less than floor(amount * fee / 10000)
        /// when validate_splits accepted the configuration.
        #[test]
        fn prop_fee_floor_guaranteed(amount in any::<u64>(), fee in 0u16..=500, extra in 0u16..=500) {
            let treasury_bps = fee + extra;
            let splits = [entry(1, 10_000 - treasury_bps), entry(TREASURY, treasury_bps)];
            let splits: Vec<_> = splits.into_iter().filter(|s| s.basis_points > 0).collect();
            validate_splits(&splits, &key(TREASURY), fee, true).unwrap();
            let bps: Vec<u16> = splits.iter().map(|s| s.basis_points).collect();
            let shares = compute_shares(amount, &bps).unwrap();
            let treasury_share: u64 = splits.iter().zip(shares.iter())
                .filter(|(s, _)| s.recipient == key(TREASURY)).map(|(_, v)| *v).sum();
            prop_assert!(treasury_share as u128 >= (amount as u128) * (fee as u128) / 10_000);
        }

        /// Any configuration whose treasury share is below the fee is rejected.
        #[test]
        fn prop_underpaid_fee_rejected(fee in 1u16..=500, short in 1u16..=500) {
            prop_assume!(short <= fee);
            let paid = fee - short;
            let mut splits = vec![entry(1, 10_000 - paid)];
            if paid > 0 { splits.push(entry(TREASURY, paid)); }
            expect(validate_splits(&splits, &key(TREASURY), fee, true), PayRogenError::GatewayFeeNotPaid);
        }
    }

    // ─── Layout guards (off-chain clients read fixed offsets) ────────────────

    #[test]
    fn account_sizes_match_serialized_layout() {
        assert_eq!(GatewayConfig::LEN, 75);
        assert_eq!(WalletFreeze::LEN, 57);
        assert_eq!(EscrowAccount::LEN, 366);

        let cfg = GatewayConfig { authority: key(1), treasury: key(2), gateway_fee_bps: 150, bump: 254 };
        let mut buf = Vec::new();
        cfg.try_serialize(&mut buf).unwrap();
        assert_eq!(buf.len(), GatewayConfig::LEN);
        assert_eq!(&buf[40..72], key(2).as_ref(), "treasury offset used by merchant-ui");
        assert_eq!(u16::from_le_bytes([buf[72], buf[73]]), 150, "fee offset used by merchant-ui");

        let freeze = WalletFreeze { wallet: key(3), frozen_at: 1, expires_at: 2, bump: 1 };
        let mut buf = Vec::new();
        freeze.try_serialize(&mut buf).unwrap();
        assert_eq!(buf.len(), WalletFreeze::LEN);

        let escrow = EscrowAccount {
            buyer: key(1),
            seller: key(2),
            merchant: key(3),
            amount: 9,
            order_id: [4; 32],
            status: EscrowStatus::Disputed,
            locked_at: 5,
            timeout_at: 6,
            split_count: 1,
            splits: [SplitConfig { recipient: key(7), basis_points: 10_000 }; MAX_SPLITS],
            bump: 250,
            mint: key(8),
            vault_bump: 251,
        };
        let mut buf = Vec::new();
        escrow.try_serialize(&mut buf).unwrap();
        assert_eq!(buf.len(), EscrowAccount::LEN);
        assert_eq!(buf[144], 3, "status offset");
        assert_eq!(buf[332], 250, "bump offset");
        assert_eq!(&buf[333..365], key(8).as_ref(), "mint offset");
        assert_eq!(buf[365], 251, "vault_bump offset");
    }
}
