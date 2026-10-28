//! Direct split payments (SPL + SOL) and gateway administration.
//!
//! Covers audit findings H-1 (initialize front-running), H-2 (fee not
//! enforced), M-1 (SPL recipients not bound), M-3 (authority/treasury
//! rotation), L-1 (rounding dust), L-3 (freeze parsing), and the merchant-ui
//! compatibility contract.

mod common;

use common::*;
use solana_address::Address;
use solana_keypair::Keypair;
use solana_signer::Signer;

const FEE: u16 = 150; // 1.5%, as on devnet

/// Merchant-ui split order: merchant, treasury (fee), optional platform.
fn merchant_ui_splits(env: &Env, merchant: Address, platform: Option<(Address, u16)>) -> Vec<Split> {
    let platform_bps = platform.map(|p| p.1).unwrap_or(0);
    let mut s = vec![(merchant, 10_000 - env.fee_bps - platform_bps), (env.treasury, env.fee_bps)];
    if let Some(p) = platform {
        s.push(p);
    }
    s
}

// ─── H-1: initialize_gateway ─────────────────────────────────────────────────

#[test]
fn initialize_rejects_non_upgrade_authority() {
    let upgrade_authority = Keypair::new();
    let mut svm = new_svm(Some(&upgrade_authority.pubkey()));
    let attacker = funded_keypair(&mut svm);
    let res = send(
        &mut svm,
        &[ix_initialize_gateway(attacker.pubkey(), attacker.pubkey(), 0)],
        &attacker,
        &[],
    );
    assert_err(&res, "UnauthorizedSigner");
    assert!(svm.get_account(&config_pda()).is_none());
}

#[test]
fn initialize_rejects_immutable_program() {
    let mut svm = new_svm(None);
    let someone = funded_keypair(&mut svm);
    let res = send(
        &mut svm,
        &[ix_initialize_gateway(someone.pubkey(), someone.pubkey(), 0)],
        &someone,
        &[],
    );
    assert_err(&res, "UnauthorizedSigner");
}

#[test]
fn initialize_by_upgrade_authority_and_only_once() {
    let mut env = setup(FEE);
    let cfg = read_config(&env.svm);
    assert_eq!(cfg.authority, env.authority.pubkey());
    assert_eq!(cfg.treasury, env.treasury);
    assert_eq!(cfg.fee_bps, FEE);

    let res = send(
        &mut env.svm,
        &[ix_initialize_gateway(env.authority.pubkey(), Address::new_unique(), 0)],
        &env.authority,
        &[],
    );
    assert!(res.is_err(), "re-initialization must fail");
    assert_eq!(read_config(&env.svm).treasury, env.treasury);
}

#[test]
fn initialize_rejects_fee_above_cap() {
    let authority = Keypair::new();
    let mut svm = new_svm(Some(&authority.pubkey()));
    svm.airdrop(&authority.pubkey(), 10 * LAMPORTS_PER_SOL).unwrap();
    let res = send(
        &mut svm,
        &[ix_initialize_gateway(authority.pubkey(), Address::new_unique(), 501)],
        &authority,
        &[],
    );
    assert_err(&res, "FeeExceedsMaximum");
}

// ─── SPL split payment ───────────────────────────────────────────────────────

struct SplFixture {
    payer: Keypair,
    payer_ata: Address,
    mint: Address,
    merchant: Address,
    merchant_ata: Address,
    treasury_ata: Address,
    platform: Address,
    platform_ata: Address,
}

fn spl_fixture(env: &mut Env, payer_balance: u64) -> SplFixture {
    let payer = funded_keypair(&mut env.svm);
    let mint = create_mint(&mut env.svm, 6);
    let payer_ata = create_token_account(&mut env.svm, &mint, &payer.pubkey(), payer_balance);
    let merchant = Address::new_unique();
    let merchant_ata = create_token_account(&mut env.svm, &mint, &merchant, 0);
    let treasury = env.treasury;
    let treasury_ata = create_token_account(&mut env.svm, &mint, &treasury, 0);
    let platform = Address::new_unique();
    let platform_ata = create_token_account(&mut env.svm, &mint, &platform, 0);
    SplFixture { payer, payer_ata, mint, merchant, merchant_ata, treasury_ata, platform, platform_ata }
}

#[test]
fn spl_split_merchant_ui_three_way_exact_amounts() {
    let mut env = setup(FEE);
    let f = spl_fixture(&mut env, 50_000_000);
    // Odd amount so every share rounds down; remainder must go to the merchant.
    let amount = 20_000_003u64;
    let splits = merchant_ui_splits(&env, f.merchant, Some((f.platform, 500)));
    let ix = ix_split_payment(
        f.payer.pubkey(),
        f.payer_ata,
        amount,
        &splits,
        &[f.merchant_ata, f.treasury_ata, f.platform_ata],
    );
    let res = send(&mut env.svm, &[ix], &f.payer, &[]);
    assert_ok(&res);
    // merchant-ui does not set a compute budget, so the instruction plus a
    // memo and up to three ATA creations must fit in the 200k default.
    let cu = res.unwrap().compute_units_consumed;
    println!("execute_split_payment (3-way) CU: {cu}");
    assert!(cu < 60_000, "split payment used {cu} CU");

    let treasury = token_balance(&env.svm, &f.treasury_ata);
    let platform = token_balance(&env.svm, &f.platform_ata);
    let merchant = token_balance(&env.svm, &f.merchant_ata);
    assert_eq!(treasury, amount * 150 / 10_000);
    assert_eq!(platform, amount * 500 / 10_000);
    assert_eq!(merchant, amount - treasury - platform, "remainder goes to the first entry");
    assert_eq!(token_balance(&env.svm, &f.payer_ata), 50_000_000 - amount, "payer debited exactly");
}

#[test]
fn spl_split_merchant_ui_two_way() {
    let mut env = setup(FEE);
    let f = spl_fixture(&mut env, 1_000_000);
    let splits = merchant_ui_splits(&env, f.merchant, None);
    let ix = ix_split_payment(f.payer.pubkey(), f.payer_ata, 1_000_000, &splits, &[f.merchant_ata, f.treasury_ata]);
    assert_ok(&send(&mut env.svm, &[ix], &f.payer, &[]));
    assert_eq!(token_balance(&env.svm, &f.merchant_ata), 985_000);
    assert_eq!(token_balance(&env.svm, &f.treasury_ata), 15_000);
}

#[test]
fn spl_split_rejects_recipient_account_substitution() {
    let mut env = setup(FEE);
    let f = spl_fixture(&mut env, 1_000_000);
    let attacker = Address::new_unique();
    let attacker_ata = create_token_account(&mut env.svm, &f.mint, &attacker, 0);
    let splits = merchant_ui_splits(&env, f.merchant, None);

    // Instruction data names the merchant, but the account is the attacker's.
    let ix = ix_split_payment(f.payer.pubkey(), f.payer_ata, 1_000_000, &splits, &[attacker_ata, f.treasury_ata]);
    assert_err(&send(&mut env.svm, &[ix], &f.payer, &[]), "InvalidRecipient");

    // Treasury share redirected.
    let ix = ix_split_payment(f.payer.pubkey(), f.payer_ata, 1_000_000, &splits, &[f.merchant_ata, attacker_ata]);
    assert_err(&send(&mut env.svm, &[ix], &f.payer, &[]), "InvalidRecipient");

    assert_eq!(token_balance(&env.svm, &f.payer_ata), 1_000_000);
    assert_eq!(token_balance(&env.svm, &attacker_ata), 0);
}

#[test]
fn spl_split_rejects_wrong_mint_and_non_token_accounts() {
    let mut env = setup(FEE);
    let f = spl_fixture(&mut env, 1_000_000);
    let other_mint = create_mint(&mut env.svm, 6);
    let merchant_other_mint = create_token_account(&mut env.svm, &other_mint, &f.merchant, 0);
    let splits = merchant_ui_splits(&env, f.merchant, None);

    let ix = ix_split_payment(f.payer.pubkey(), f.payer_ata, 1_000_000, &splits, &[merchant_other_mint, f.treasury_ata]);
    assert_err(&send(&mut env.svm, &[ix], &f.payer, &[]), "InvalidRecipient");

    // A plain wallet instead of a token account.
    let ix = ix_split_payment(f.payer.pubkey(), f.payer_ata, 1_000_000, &splits, &[f.merchant, f.treasury_ata]);
    assert_err(&send(&mut env.svm, &[ix], &f.payer, &[]), "InvalidRecipient");
}

#[test]
fn spl_split_enforces_gateway_fee() {
    let mut env = setup(FEE);
    let f = spl_fixture(&mut env, 1_000_000);

    // No treasury entry at all.
    let ix = ix_split_payment(f.payer.pubkey(), f.payer_ata, 1_000_000, &[(f.merchant, 10_000)], &[f.merchant_ata]);
    assert_err(&send(&mut env.svm, &[ix], &f.payer, &[]), "GatewayFeeNotPaid");

    // Treasury entry below the configured fee.
    let splits = vec![(f.merchant, 9_901), (env.treasury, 99)];
    let ix = ix_split_payment(f.payer.pubkey(), f.payer_ata, 1_000_000, &splits, &[f.merchant_ata, f.treasury_ata]);
    assert_err(&send(&mut env.svm, &[ix], &f.payer, &[]), "GatewayFeeNotPaid");

    // Paying more than the fee is allowed.
    let splits = vec![(f.merchant, 9_800), (env.treasury, 200)];
    let ix = ix_split_payment(f.payer.pubkey(), f.payer_ata, 1_000_000, &splits, &[f.merchant_ata, f.treasury_ata]);
    assert_ok(&send(&mut env.svm, &[ix], &f.payer, &[]));
}

#[test]
fn spl_split_input_validation() {
    let mut env = setup(FEE);
    let f = spl_fixture(&mut env, 1_000_000);
    let splits = merchant_ui_splits(&env, f.merchant, None);
    let accts = [f.merchant_ata, f.treasury_ata];

    let ix = ix_split_payment(f.payer.pubkey(), f.payer_ata, 0, &splits, &accts);
    assert_err(&send(&mut env.svm, &[ix], &f.payer, &[]), "InvalidAmount");

    let bad_sum = vec![(f.merchant, 9_000), (env.treasury, FEE)];
    let ix = ix_split_payment(f.payer.pubkey(), f.payer_ata, 1_000, &bad_sum, &accts);
    assert_err(&send(&mut env.svm, &[ix], &f.payer, &[]), "InvalidSplitSum");

    let ix = ix_split_payment(f.payer.pubkey(), f.payer_ata, 1_000, &splits, &[f.merchant_ata]);
    assert_err(&send(&mut env.svm, &[ix], &f.payer, &[]), "InvalidRecipient");

    let ix = ix_split_payment(f.payer.pubkey(), f.payer_ata, 1_000_001, &splits, &accts);
    assert_err(&send(&mut env.svm, &[ix], &f.payer, &[]), "InsufficientBalance");
}

/// With the fee set to 0, merchant-ui still emits a 0-bps treasury entry.
#[test]
fn spl_split_zero_fee_merchant_ui_shape() {
    let mut env = setup(0);
    let f = spl_fixture(&mut env, 1_000_000);
    let splits = merchant_ui_splits(&env, f.merchant, None);
    assert_eq!(splits[1].1, 0);
    let ix = ix_split_payment(f.payer.pubkey(), f.payer_ata, 1_000_000, &splits, &[f.merchant_ata, f.treasury_ata]);
    assert_ok(&send(&mut env.svm, &[ix], &f.payer, &[]));
    assert_eq!(token_balance(&env.svm, &f.merchant_ata), 1_000_000);
    assert_eq!(token_balance(&env.svm, &f.treasury_ata), 0);
}

#[test]
fn spl_split_respects_freeze_and_expiry() {
    let mut env = setup(FEE);
    let f = spl_fixture(&mut env, 1_000_000);
    let splits = merchant_ui_splits(&env, f.merchant, None);
    let accts = [f.merchant_ata, f.treasury_ata];
    let authority = env.authority.insecure_clone();

    assert_ok(&send(&mut env.svm, &[ix_freeze_wallet(authority.pubkey(), f.payer.pubkey())], &authority, &[]));

    let ix = ix_split_payment(f.payer.pubkey(), f.payer_ata, 1_000, &splits, &accts);
    assert_err(&send(&mut env.svm, &[ix.clone()], &f.payer, &[]), "WalletFrozen");

    warp_seconds(&mut env.svm, DAY + 1);
    assert_ok(&send(&mut env.svm, &[ix], &f.payer, &[]));
}

#[test]
fn freeze_requires_gateway_authority() {
    let mut env = setup(FEE);
    let attacker = funded_keypair(&mut env.svm);
    let victim = Address::new_unique();
    let res = send(&mut env.svm, &[ix_freeze_wallet(attacker.pubkey(), victim)], &attacker, &[]);
    assert_err(&res, "ConstraintHasOne");
}

// ─── SOL split payment ───────────────────────────────────────────────────────

#[test]
fn sol_split_merchant_ui_layout_exact_amounts() {
    let mut env = setup(FEE);
    let payer = funded_keypair(&mut env.svm);
    let merchant = Address::new_unique();
    let platform = Address::new_unique();
    // Pre-fund recipients so they are rent-exempt system accounts.
    for a in [merchant, platform, env.treasury] {
        env.svm.airdrop(&a, LAMPORTS_PER_SOL).unwrap();
    }
    let amount = 1_000_000_007u64;
    let splits = merchant_ui_splits(&env, merchant, Some((platform, 500)));
    let before = [lamports(&env.svm, &merchant), lamports(&env.svm, &env.treasury), lamports(&env.svm, &platform)];

    let ix = ix_sol_split_payment(payer.pubkey(), amount, &splits, &[merchant, env.treasury, platform]);
    assert_ok(&send(&mut env.svm, &[ix], &payer, &[]));

    let fee = amount * 150 / 10_000;
    let plat = amount * 500 / 10_000;
    assert_eq!(lamports(&env.svm, &env.treasury) - before[1], fee);
    assert_eq!(lamports(&env.svm, &platform) - before[2], plat);
    assert_eq!(lamports(&env.svm, &merchant) - before[0], amount - fee - plat);
}

#[test]
fn sol_split_rejects_substitution_and_fee_bypass() {
    let mut env = setup(FEE);
    let payer = funded_keypair(&mut env.svm);
    let merchant = Address::new_unique();
    let attacker = Address::new_unique();
    env.svm.airdrop(&env.treasury.clone(), LAMPORTS_PER_SOL).unwrap();
    let splits = merchant_ui_splits(&env, merchant, None);

    let ix = ix_sol_split_payment(payer.pubkey(), LAMPORTS_PER_SOL, &splits, &[attacker, env.treasury]);
    assert_err(&send(&mut env.svm, &[ix], &payer, &[]), "InvalidRecipient");

    let ix = ix_sol_split_payment(payer.pubkey(), LAMPORTS_PER_SOL, &[(merchant, 10_000)], &[merchant]);
    assert_err(&send(&mut env.svm, &[ix], &payer, &[]), "GatewayFeeNotPaid");
}

#[test]
fn sol_split_respects_freeze() {
    let mut env = setup(FEE);
    let payer = funded_keypair(&mut env.svm);
    let merchant = Address::new_unique();
    env.svm.airdrop(&env.treasury.clone(), LAMPORTS_PER_SOL).unwrap();
    let authority = env.authority.insecure_clone();
    assert_ok(&send(&mut env.svm, &[ix_freeze_wallet(authority.pubkey(), payer.pubkey())], &authority, &[]));

    let splits = merchant_ui_splits(&env, merchant, None);
    let ix = ix_sol_split_payment(payer.pubkey(), LAMPORTS_PER_SOL, &splits, &[merchant, env.treasury]);
    assert_err(&send(&mut env.svm, &[ix], &payer, &[]), "WalletFrozen");
}

// ─── Admin: fee, treasury, authority rotation ────────────────────────────────

#[test]
fn update_fee_authority_and_cap() {
    let mut env = setup(FEE);
    let attacker = funded_keypair(&mut env.svm);
    assert_err(&send(&mut env.svm, &[ix_update_gateway_fee(attacker.pubkey(), 0)], &attacker, &[]), "ConstraintHasOne");

    let authority = env.authority.insecure_clone();
    assert_err(&send(&mut env.svm, &[ix_update_gateway_fee(authority.pubkey(), 501)], &authority, &[]), "FeeExceedsMaximum");
    assert_ok(&send(&mut env.svm, &[ix_update_gateway_fee(authority.pubkey(), 200)], &authority, &[]));
    assert_eq!(read_config(&env.svm).fee_bps, 200);
}

#[test]
fn fee_increase_rejects_stale_split() {
    let mut env = setup(FEE);
    let f = spl_fixture(&mut env, 1_000_000);
    let stale = merchant_ui_splits(&env, f.merchant, None); // built at 150 bps
    let authority = env.authority.insecure_clone();
    assert_ok(&send(&mut env.svm, &[ix_update_gateway_fee(authority.pubkey(), 200)], &authority, &[]));
    let ix = ix_split_payment(f.payer.pubkey(), f.payer_ata, 1_000, &stale, &[f.merchant_ata, f.treasury_ata]);
    assert_err(&send(&mut env.svm, &[ix], &f.payer, &[]), "GatewayFeeNotPaid");
}

#[test]
fn two_step_authority_rotation() {
    let mut env = setup(FEE);
    let old = env.authority.insecure_clone();
    let new = funded_keypair(&mut env.svm);
    let attacker = funded_keypair(&mut env.svm);

    // Only the current authority can propose.
    assert_err(
        &send(&mut env.svm, &[ix_propose_authority(attacker.pubkey(), attacker.pubkey())], &attacker, &[]),
        "ConstraintHasOne",
    );
    assert_ok(&send(&mut env.svm, &[ix_propose_authority(old.pubkey(), new.pubkey())], &old, &[]));

    // Proposal alone changes nothing.
    assert_eq!(read_config(&env.svm).authority, old.pubkey());

    // Only the nominee can accept.
    assert_err(
        &send(&mut env.svm, &[ix_accept_authority(attacker.pubkey(), old.pubkey())], &attacker, &[]),
        "UnauthorizedSigner",
    );
    assert_ok(&send(&mut env.svm, &[ix_accept_authority(new.pubkey(), old.pubkey())], &new, &[]));
    assert_eq!(read_config(&env.svm).authority, new.pubkey());
    assert!(account_closed(&env.svm, &pending_authority_pda()));

    // Old authority lost its powers; new authority has them.
    assert_err(&send(&mut env.svm, &[ix_update_gateway_fee(old.pubkey(), 0)], &old, &[]), "ConstraintHasOne");
    assert_ok(&send(&mut env.svm, &[ix_update_gateway_fee(new.pubkey(), 0)], &new, &[]));
}

#[test]
fn set_treasury_is_authority_only_and_enforced() {
    let mut env = setup(FEE);
    let f = spl_fixture(&mut env, 1_000_000);
    let attacker = funded_keypair(&mut env.svm);
    assert_err(
        &send(&mut env.svm, &[ix_set_treasury(attacker.pubkey(), attacker.pubkey())], &attacker, &[]),
        "ConstraintHasOne",
    );

    let authority = env.authority.insecure_clone();
    assert_err(
        &send(&mut env.svm, &[ix_set_treasury(authority.pubkey(), Address::default())], &authority, &[]),
        "InvalidAuthority",
    );
    let new_treasury = Address::new_unique();
    assert_ok(&send(&mut env.svm, &[ix_set_treasury(authority.pubkey(), new_treasury)], &authority, &[]));
    assert_eq!(read_config(&env.svm).treasury, new_treasury);

    // Paying the old treasury no longer satisfies the fee.
    let old_splits = merchant_ui_splits(&env, f.merchant, None);
    let ix = ix_split_payment(f.payer.pubkey(), f.payer_ata, 1_000, &old_splits, &[f.merchant_ata, f.treasury_ata]);
    assert_err(&send(&mut env.svm, &[ix], &f.payer, &[]), "GatewayFeeNotPaid");
}
