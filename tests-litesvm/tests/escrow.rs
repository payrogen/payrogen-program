//! Escrow lifecycle and attack tests.
//!
//! Covers audit findings C-1 (refund destination), C-2 (release
//! destinations), C-3 (unvalidated vault/mint), H-2 (fee on escrow), H-3
//! (dispute flow), M-2 (freeze on escrow), L-1 (dust / vault close),
//! L-2 (timeout cap).

mod common;

use common::*;
use solana_address::Address;
use solana_keypair::Keypair;
use solana_signer::Signer;

const FEE: u16 = 150;
const AMOUNT: u64 = 10_000_003; // odd: exercises rounding remainder

struct Fx {
    env: Env,
    buyer: Keypair,
    seller: Keypair,
    merchant: Address,
    mint: Address,
    buyer_ata: Address,
    seller_ata: Address,
    treasury_ata: Address,
    platform: Address,
    platform_ata: Address,
    order_id: [u8; 32],
}

impl Fx {
    fn new() -> Self {
        let mut env = setup(FEE);
        let buyer = funded_keypair(&mut env.svm);
        let seller = funded_keypair(&mut env.svm);
        let merchant = Address::new_unique();
        let mint = create_mint(&mut env.svm, 6);
        let buyer_ata = create_token_account(&mut env.svm, &mint, &buyer.pubkey(), 100_000_000);
        let seller_ata = create_token_account(&mut env.svm, &mint, &seller.pubkey(), 0);
        let treasury = env.treasury;
        let treasury_ata = create_token_account(&mut env.svm, &mint, &treasury, 0);
        let platform = Address::new_unique();
        let platform_ata = create_token_account(&mut env.svm, &mint, &platform, 0);
        Fx {
            env,
            buyer,
            seller,
            merchant,
            mint,
            buyer_ata,
            seller_ata,
            treasury_ata,
            platform,
            platform_ata,
            order_id: [7u8; 32],
        }
    }

    fn splits(&self) -> Vec<Split> {
        vec![(self.seller.pubkey(), 10_000 - FEE - 500), (self.env.treasury, FEE), (self.platform, 500)]
    }

    fn args(&self) -> EscrowArgs {
        EscrowArgs {
            buyer: self.buyer.pubkey(),
            seller: self.seller.pubkey(),
            merchant: self.merchant,
            mint: self.mint,
            buyer_token_account: self.buyer_ata,
            order_id: self.order_id,
            amount: AMOUNT,
            timeout_seconds: 7 * DAY,
            splits: self.splits(),
        }
    }

    fn escrow(&self) -> Address {
        escrow_pda(&self.buyer.pubkey(), &self.order_id)
    }
    fn vault(&self) -> Address {
        vault_pda(&self.escrow())
    }
    fn recipients(&self) -> [Address; 3] {
        [self.seller_ata, self.treasury_ata, self.platform_ata]
    }

    fn create(&mut self) {
        let ix = ix_create_escrow(&self.args());
        let buyer = self.buyer.insecure_clone();
        assert_ok(&send(&mut self.env.svm, &[ix], &buyer, &[]));
    }

    fn release_by(&mut self, caller: &Keypair, recipients: &[Address]) -> litesvm::types::TransactionResult {
        let ix = ix_release_escrow(caller.pubkey(), self.buyer.pubkey(), self.escrow(), self.vault(), recipients);
        send(&mut self.env.svm, &[ix], caller, &[])
    }

    fn refund_by(&mut self, caller: &Keypair, buyer_token_account: Address) -> litesvm::types::TransactionResult {
        let ix = ix_refund_escrow(caller.pubkey(), self.escrow(), self.vault(), buyer_token_account, self.buyer.pubkey());
        send(&mut self.env.svm, &[ix], caller, &[])
    }

    fn dispute_by(&mut self, caller: &Keypair) -> litesvm::types::TransactionResult {
        let ix = ix_raise_dispute(caller.pubkey(), self.escrow());
        send(&mut self.env.svm, &[ix], caller, &[])
    }

    fn assert_paid_out(&self, total: u64) {
        let svm = &self.env.svm;
        let treasury = token_balance(svm, &self.treasury_ata);
        let platform = token_balance(svm, &self.platform_ata);
        let seller = token_balance(svm, &self.seller_ata);
        assert_eq!(treasury, total * FEE as u64 / 10_000);
        assert_eq!(platform, total * 500 / 10_000);
        assert_eq!(seller, total - treasury - platform);
    }

    fn assert_closed(&self) {
        assert!(account_closed(&self.env.svm, &self.escrow()), "escrow closed");
        assert!(account_closed(&self.env.svm, &self.vault()), "vault closed");
    }
}

// ─── create_escrow ───────────────────────────────────────────────────────────

#[test]
fn create_locks_funds_in_program_vault() {
    let mut fx = Fx::new();
    fx.create();
    assert_eq!(token_balance(&fx.env.svm, &fx.vault()), AMOUNT);
    assert_eq!(token_balance(&fx.env.svm, &fx.buyer_ata), 100_000_000 - AMOUNT);
    assert_eq!(escrow_status(&fx.env.svm, &fx.escrow()), STATUS_LOCKED);
    assert_eq!(escrow_mint(&fx.env.svm, &fx.escrow()), fx.mint);

    // Vault: token account for the escrow mint, owned by the escrow PDA.
    let vault = fx.env.svm.get_account(&fx.vault()).unwrap();
    assert_eq!(vault.owner, TOKEN_PROGRAM_ID);
    assert_eq!(&vault.data[0..32], fx.mint.as_ref());
    assert_eq!(&vault.data[32..64], fx.escrow().as_ref());

    // Double lock for the same order is rejected.
    let ix = ix_create_escrow(&fx.args());
    let buyer = fx.buyer.insecure_clone();
    assert!(send(&mut fx.env.svm, &[ix], &buyer, &[]).is_err());
}

#[test]
fn create_rejects_attacker_controlled_vault() {
    let mut fx = Fx::new();
    let attacker = Address::new_unique();
    let attacker_ata = create_token_account(&mut fx.env.svm, &fx.mint, &attacker, 0);
    let ix = ix_create_escrow_with_vault(&fx.args(), attacker_ata);
    let buyer = fx.buyer.insecure_clone();
    let res = send(&mut fx.env.svm, &[ix], &buyer, &[]);
    assert_err(&res, "ConstraintSeeds");
    assert_eq!(token_balance(&fx.env.svm, &attacker_ata), 0);
}

#[test]
fn create_rejects_mint_mismatch() {
    let mut fx = Fx::new();
    let other_mint = create_mint(&mut fx.env.svm, 6);
    let mut args = fx.args();
    args.mint = other_mint;
    let buyer = fx.buyer.insecure_clone();
    let res = send(&mut fx.env.svm, &[ix_create_escrow(&args)], &buyer, &[]);
    assert_err(&res, "ConstraintTokenMint");
}

#[test]
fn create_validates_splits_fee_seller_and_timeout() {
    let mut fx = Fx::new();
    let buyer = fx.buyer.insecure_clone();
    let seller = fx.seller.pubkey();

    let mut a = fx.args();
    a.splits = vec![(seller, 10_000)];
    assert_err(&send(&mut fx.env.svm, &[ix_create_escrow(&a)], &buyer, &[]), "GatewayFeeNotPaid");

    let mut a = fx.args();
    a.splits = vec![(fx.platform, 10_000 - FEE), (fx.env.treasury, FEE)];
    assert_err(&send(&mut fx.env.svm, &[ix_create_escrow(&a)], &buyer, &[]), "SellerNotInSplits");

    let mut a = fx.args();
    a.splits = vec![(seller, 5_000 - FEE), (seller, 5_000), (fx.env.treasury, FEE)];
    assert_err(&send(&mut fx.env.svm, &[ix_create_escrow(&a)], &buyer, &[]), "InvalidRecipient");

    let mut a = fx.args();
    a.splits = vec![(seller, 5_000 - FEE), (fx.escrow(), 5_000), (fx.env.treasury, FEE)];
    assert_err(&send(&mut fx.env.svm, &[ix_create_escrow(&a)], &buyer, &[]), "InvalidRecipient");

    let mut a = fx.args();
    a.timeout_seconds = 90 * DAY + 1;
    assert_err(&send(&mut fx.env.svm, &[ix_create_escrow(&a)], &buyer, &[]), "InvalidTimeout");

    let mut a = fx.args();
    a.timeout_seconds = 0;
    assert_err(&send(&mut fx.env.svm, &[ix_create_escrow(&a)], &buyer, &[]), "InvalidTimeout");

    let mut a = fx.args();
    a.amount = 0;
    assert_err(&send(&mut fx.env.svm, &[ix_create_escrow(&a)], &buyer, &[]), "InvalidAmount");

    let mut a = fx.args();
    a.timeout_seconds = 90 * DAY;
    assert_ok(&send(&mut fx.env.svm, &[ix_create_escrow(&a)], &buyer, &[]));
}

#[test]
fn create_rejects_frozen_buyer() {
    let mut fx = Fx::new();
    let authority = fx.env.authority.insecure_clone();
    assert_ok(&send(&mut fx.env.svm, &[ix_freeze_wallet(authority.pubkey(), fx.buyer.pubkey())], &authority, &[]));
    let buyer = fx.buyer.insecure_clone();
    let ix = ix_create_escrow(&fx.args());
    let res = send(&mut fx.env.svm, &[ix], &buyer, &[]);
    assert_err(&res, "WalletFrozen");
}

// ─── release_escrow ──────────────────────────────────────────────────────────

#[test]
fn buyer_release_pays_splits_and_closes_everything() {
    let mut fx = Fx::new();
    fx.create();
    let buyer_lamports_before = lamports(&fx.env.svm, &fx.buyer.pubkey());
    let buyer = fx.buyer.insecure_clone();
    let recipients = fx.recipients();
    assert_ok(&fx.release_by(&buyer, &recipients));

    fx.assert_paid_out(AMOUNT);
    fx.assert_closed();
    // Buyer paid the fee for this tx but got escrow + vault rent back.
    assert!(lamports(&fx.env.svm, &fx.buyer.pubkey()) > buyer_lamports_before);
}

#[test]
fn release_rejects_redirected_payouts() {
    let mut fx = Fx::new();
    fx.create();
    let buyer = fx.buyer.insecure_clone();
    let attacker = Address::new_unique();
    let attacker_ata = create_token_account(&mut fx.env.svm, &fx.mint, &attacker, 0);

    // Seller share to the buyer's own account (the original C-2 exploit).
    let res = fx.release_by(&buyer, &[fx.buyer_ata, fx.treasury_ata, fx.platform_ata]);
    assert_err(&res, "InvalidRecipient");
    // Treasury share to a third party.
    let res = fx.release_by(&buyer, &[fx.seller_ata, attacker_ata, fx.platform_ata]);
    assert_err(&res, "InvalidRecipient");
    // Right owner, wrong mint.
    let other_mint = create_mint(&mut fx.env.svm, 6);
    let seller_other = create_token_account(&mut fx.env.svm, &other_mint, &fx.seller.pubkey(), 0);
    let res = fx.release_by(&buyer, &[seller_other, fx.treasury_ata, fx.platform_ata]);
    assert_err(&res, "InvalidRecipient");
    // Wrong count.
    let res = fx.release_by(&buyer, &[fx.seller_ata, fx.treasury_ata]);
    assert_err(&res, "InvalidRecipient");

    assert_eq!(token_balance(&fx.env.svm, &fx.vault()), AMOUNT);
    assert_eq!(escrow_status(&fx.env.svm, &fx.escrow()), STATUS_LOCKED);
}

#[test]
fn release_rejects_substituted_vault() {
    let mut fx = Fx::new();
    fx.create();
    // A second escrow by the same buyer gives a real, funded, program vault
    // that is nevertheless the wrong one for the first escrow.
    let mut other = fx.args();
    other.order_id = [9u8; 32];
    let buyer = fx.buyer.insecure_clone();
    assert_ok(&send(&mut fx.env.svm, &[ix_create_escrow(&other)], &buyer, &[]));
    let other_vault = vault_pda(&escrow_pda(&fx.buyer.pubkey(), &other.order_id));

    let ix = ix_release_escrow(buyer.pubkey(), buyer.pubkey(), fx.escrow(), other_vault, &fx.recipients());
    assert_err(&send(&mut fx.env.svm, &[ix], &buyer, &[]), "ConstraintSeeds");
}

#[test]
fn release_authorization() {
    let mut fx = Fx::new();
    fx.create();
    let stranger = funded_keypair(&mut fx.env.svm);
    let seller = fx.seller.insecure_clone();
    let authority = fx.env.authority.insecure_clone();
    let recipients = fx.recipients();

    assert_err(&fx.release_by(&stranger, &recipients), "UnauthorizedSigner");
    assert_err(&fx.release_by(&seller, &recipients), "UnauthorizedSigner");
    // Gateway authority cannot release an undisputed escrow.
    assert_err(&fx.release_by(&authority, &recipients), "EscrowNotDisputed");
    assert_eq!(token_balance(&fx.env.svm, &fx.vault()), AMOUNT);
}

#[test]
fn frozen_buyer_cannot_release() {
    let mut fx = Fx::new();
    fx.create();
    let authority = fx.env.authority.insecure_clone();
    assert_ok(&send(&mut fx.env.svm, &[ix_freeze_wallet(authority.pubkey(), fx.buyer.pubkey())], &authority, &[]));
    let buyer = fx.buyer.insecure_clone();
    let recipients = fx.recipients();
    assert_err(&fx.release_by(&buyer, &recipients), "WalletFrozen");
}

#[test]
fn donated_tokens_cannot_block_release() {
    let mut fx = Fx::new();
    fx.create();
    // Anyone can send tokens to the vault; that must not brick close_account.
    let donor = funded_keypair(&mut fx.env.svm);
    let donor_ata = create_token_account(&mut fx.env.svm, &fx.mint, &donor.pubkey(), 1_000);
    let vault = fx.vault();
    assert_ok(&send(&mut fx.env.svm, &[ix_spl_transfer(donor_ata, vault, donor.pubkey(), 1_000)], &donor, &[]));

    let buyer = fx.buyer.insecure_clone();
    let recipients = fx.recipients();
    assert_ok(&fx.release_by(&buyer, &recipients));
    fx.assert_paid_out(AMOUNT + 1_000);
    fx.assert_closed();
}

// ─── refund_escrow ───────────────────────────────────────────────────────────

#[test]
fn refund_before_timeout_is_rejected_even_for_authority() {
    let mut fx = Fx::new();
    fx.create();
    let stranger = funded_keypair(&mut fx.env.svm);
    let authority = fx.env.authority.insecure_clone();
    let buyer_ata = fx.buyer_ata;
    assert_err(&fx.refund_by(&stranger, buyer_ata), "EscrowNotTimedOut");
    assert_err(&fx.refund_by(&authority, buyer_ata), "EscrowNotTimedOut");
}

#[test]
fn timeout_refund_cannot_be_redirected() {
    let mut fx = Fx::new();
    fx.create();
    warp_seconds(&mut fx.env.svm, 7 * DAY);
    let attacker = funded_keypair(&mut fx.env.svm);
    let attacker_ata = create_token_account(&mut fx.env.svm, &fx.mint, &attacker.pubkey(), 0);

    // The original C-1 exploit: refund to the caller's account.
    assert_err(&fx.refund_by(&attacker, attacker_ata), "ConstraintTokenOwner");
    // Buyer-owned account with the wrong mint.
    let other_mint = create_mint(&mut fx.env.svm, 6);
    let buyer_other = create_token_account(&mut fx.env.svm, &other_mint, &fx.buyer.pubkey(), 0);
    assert_err(&fx.refund_by(&attacker, buyer_other), "ConstraintTokenMint");
    // Rent receiver must be the buyer.
    let ix = ix_refund_escrow(attacker.pubkey(), fx.escrow(), fx.vault(), fx.buyer_ata, attacker.pubkey());
    assert_err(&send(&mut fx.env.svm, &[ix], &attacker, &[]), "UnauthorizedSigner");

    assert_eq!(token_balance(&fx.env.svm, &attacker_ata), 0);

    // Permissionless crank to the correct destination succeeds.
    let buyer_ata = fx.buyer_ata;
    assert_ok(&fx.refund_by(&attacker, buyer_ata));
    assert_eq!(token_balance(&fx.env.svm, &fx.buyer_ata), 100_000_000);
    fx.assert_closed();
}

#[test]
fn gateway_authority_cannot_redirect_refund() {
    let mut fx = Fx::new();
    fx.create();
    let seller = fx.seller.insecure_clone();
    assert_ok(&fx.dispute_by(&seller));
    let authority = fx.env.authority.insecure_clone();
    let authority_ata = create_token_account(&mut fx.env.svm, &fx.mint, &authority.pubkey(), 0);
    assert_err(&fx.refund_by(&authority, authority_ata), "ConstraintTokenOwner");
    assert_eq!(token_balance(&fx.env.svm, &fx.vault()), AMOUNT);
}

// ─── Disputes ────────────────────────────────────────────────────────────────

#[test]
fn dispute_suspends_timeout_and_authority_refunds() {
    let mut fx = Fx::new();
    fx.create();
    let seller = fx.seller.insecure_clone();
    assert_ok(&fx.dispute_by(&seller));
    assert_eq!(escrow_status(&fx.env.svm, &fx.escrow()), STATUS_DISPUTED);

    warp_seconds(&mut fx.env.svm, 30 * DAY);
    let stranger = funded_keypair(&mut fx.env.svm);
    let buyer_ata = fx.buyer_ata;
    assert_err(&fx.refund_by(&stranger, buyer_ata), "UnauthorizedSigner");

    let authority = fx.env.authority.insecure_clone();
    assert_ok(&fx.refund_by(&authority, buyer_ata));
    assert_eq!(token_balance(&fx.env.svm, &fx.buyer_ata), 100_000_000);
    fx.assert_closed();
}

#[test]
fn authority_resolves_dispute_for_seller() {
    let mut fx = Fx::new();
    fx.create();
    let buyer = fx.buyer.insecure_clone();
    assert_ok(&fx.dispute_by(&buyer));
    let authority = fx.env.authority.insecure_clone();

    // Authority still cannot redirect payouts.
    let authority_ata = create_token_account(&mut fx.env.svm, &fx.mint, &authority.pubkey(), 0);
    assert_err(&fx.release_by(&authority, &[authority_ata, fx.treasury_ata, fx.platform_ata]), "InvalidRecipient");

    let recipients = fx.recipients();
    assert_ok(&fx.release_by(&authority, &recipients));
    fx.assert_paid_out(AMOUNT);
    fx.assert_closed();
}

#[test]
fn buyer_can_concede_dispute() {
    let mut fx = Fx::new();
    fx.create();
    let seller = fx.seller.insecure_clone();
    assert_ok(&fx.dispute_by(&seller));
    let buyer = fx.buyer.insecure_clone();
    let recipients = fx.recipients();
    assert_ok(&fx.release_by(&buyer, &recipients));
    fx.assert_paid_out(AMOUNT);
}

#[test]
fn dispute_rules() {
    let mut fx = Fx::new();
    fx.create();
    let stranger = funded_keypair(&mut fx.env.svm);
    let authority = fx.env.authority.insecure_clone();
    assert_err(&fx.dispute_by(&stranger), "UnauthorizedSigner");
    assert_err(&fx.dispute_by(&authority), "UnauthorizedSigner");

    let seller = fx.seller.insecure_clone();
    assert_ok(&fx.dispute_by(&seller));
    // Cannot dispute twice.
    let buyer = fx.buyer.insecure_clone();
    assert_err(&fx.dispute_by(&buyer), "EscrowNotLocked");
}

#[test]
fn dispute_after_timeout_is_rejected() {
    let mut fx = Fx::new();
    fx.create();
    warp_seconds(&mut fx.env.svm, 7 * DAY);
    let seller = fx.seller.insecure_clone();
    assert_err(&fx.dispute_by(&seller), "EscrowTimeout");
}
