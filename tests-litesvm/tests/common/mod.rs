//! Shared harness for the PayRogen LiteSVM tests.
//!
//! Everything here is a black-box client: instructions are encoded by hand
//! (Anchor discriminator + Borsh args) exactly like the production
//! merchant-ui does, and the program under test is the real SBF binary at
//! `../target/deploy/payrogen.so`.

#![allow(dead_code)]

use litesvm::types::TransactionResult;
use litesvm::LiteSVM;
use sha2::{Digest, Sha256};
use solana_account::Account;
use solana_address::Address;
use solana_clock::Clock;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction::Transaction;

pub const PROGRAM_ID: Address =
    Address::from_str_const("AeJVPDMxcSvn89F5kgiita5cndcQvamEnCSvY38HeYFg");
pub const TOKEN_PROGRAM_ID: Address =
    Address::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const SYSTEM_PROGRAM_ID: Address = Address::from_str_const("11111111111111111111111111111111");
pub const BPF_LOADER_UPGRADEABLE_ID: Address =
    Address::from_str_const("BPFLoaderUpgradeab1e11111111111111111111111");

pub const LAMPORTS_PER_SOL: u64 = 1_000_000_000;
pub const DAY: i64 = 86_400;

// ─── Encoding helpers ────────────────────────────────────────────────────────

pub fn ix_disc(name: &str) -> [u8; 8] {
    let h = Sha256::digest(format!("global:{name}").as_bytes());
    h[..8].try_into().unwrap()
}

pub fn pda(seeds: &[&[u8]]) -> Address {
    Address::find_program_address(seeds, &PROGRAM_ID).0
}

pub fn config_pda() -> Address {
    pda(&[b"gateway_config"])
}
pub fn freeze_pda(wallet: &Address) -> Address {
    pda(&[b"freeze", wallet.as_ref()])
}
pub fn escrow_pda(buyer: &Address, order_id: &[u8; 32]) -> Address {
    pda(&[b"escrow", buyer.as_ref(), order_id.as_ref()])
}
pub fn vault_pda(escrow: &Address) -> Address {
    pda(&[b"vault", escrow.as_ref()])
}
pub fn pending_authority_pda() -> Address {
    pda(&[b"pending_authority"])
}

/// (recipient, bps)
pub type Split = (Address, u16);

fn encode_splits(data: &mut Vec<u8>, splits: &[Split]) {
    data.extend_from_slice(&(splits.len() as u32).to_le_bytes());
    for (r, bps) in splits {
        data.extend_from_slice(r.as_ref());
        data.extend_from_slice(&bps.to_le_bytes());
    }
}

fn w(a: Address) -> AccountMeta {
    AccountMeta::new(a, false)
}
fn r(a: Address) -> AccountMeta {
    AccountMeta::new_readonly(a, false)
}
fn ws(a: Address) -> AccountMeta {
    AccountMeta::new(a, true)
}
fn rs(a: Address) -> AccountMeta {
    AccountMeta::new_readonly(a, true)
}

// ─── Instruction builders (account order == IDL / merchant-ui) ───────────────

pub fn ix_initialize_gateway(authority: Address, treasury: Address, fee_bps: u16) -> Instruction {
    let mut data = ix_disc("initialize_gateway").to_vec();
    data.extend_from_slice(&fee_bps.to_le_bytes());
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            ws(authority),
            r(treasury),
            w(config_pda()),
            r(PROGRAM_ID),
            r(programdata_address()),
            r(SYSTEM_PROGRAM_ID),
        ],
        data,
    }
}

/// Same layout as merchant-ui `solana-tx/route.ts` (SPL branch, minus memo).
pub fn ix_split_payment(
    payer: Address,
    payer_ata: Address,
    amount: u64,
    splits: &[Split],
    recipient_accounts: &[Address],
) -> Instruction {
    let mut data = ix_disc("execute_split_payment").to_vec();
    data.extend_from_slice(&amount.to_le_bytes());
    encode_splits(&mut data, splits);
    let mut accounts = vec![
        ws(payer),
        w(payer_ata),
        r(config_pda()),
        r(freeze_pda(&payer)),
        r(TOKEN_PROGRAM_ID),
    ];
    accounts.extend(recipient_accounts.iter().map(|a| w(*a)));
    Instruction { program_id: PROGRAM_ID, accounts, data }
}

/// Same layout as merchant-ui `solana-tx/route.ts` (SOL branch).
pub fn ix_sol_split_payment(
    payer: Address,
    amount: u64,
    splits: &[Split],
    recipient_accounts: &[Address],
) -> Instruction {
    let mut data = ix_disc("execute_sol_split_payment").to_vec();
    data.extend_from_slice(&amount.to_le_bytes());
    encode_splits(&mut data, splits);
    let mut accounts = vec![
        ws(payer),
        r(config_pda()),
        r(freeze_pda(&payer)),
        r(SYSTEM_PROGRAM_ID),
    ];
    accounts.extend(recipient_accounts.iter().map(|a| w(*a)));
    Instruction { program_id: PROGRAM_ID, accounts, data }
}

pub struct EscrowArgs {
    pub buyer: Address,
    pub seller: Address,
    pub merchant: Address,
    pub mint: Address,
    pub buyer_token_account: Address,
    pub order_id: [u8; 32],
    pub amount: u64,
    pub timeout_seconds: i64,
    pub splits: Vec<Split>,
}

pub fn ix_create_escrow(a: &EscrowArgs) -> Instruction {
    ix_create_escrow_with_vault(a, vault_pda(&escrow_pda(&a.buyer, &a.order_id)))
}

pub fn ix_create_escrow_with_vault(a: &EscrowArgs, vault: Address) -> Instruction {
    let mut data = ix_disc("create_escrow").to_vec();
    data.extend_from_slice(&a.order_id);
    data.extend_from_slice(&a.amount.to_le_bytes());
    data.extend_from_slice(&a.timeout_seconds.to_le_bytes());
    encode_splits(&mut data, &a.splits);
    let escrow = escrow_pda(&a.buyer, &a.order_id);
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            ws(a.buyer),
            r(a.seller),
            r(a.merchant),
            r(a.mint),
            w(escrow),
            w(a.buyer_token_account),
            w(vault),
            r(freeze_pda(&a.buyer)),
            r(config_pda()),
            r(TOKEN_PROGRAM_ID),
            r(SYSTEM_PROGRAM_ID),
        ],
        data,
    }
}

pub fn ix_release_escrow(
    caller: Address,
    buyer: Address,
    escrow: Address,
    vault: Address,
    recipient_accounts: &[Address],
) -> Instruction {
    let mut accounts = vec![
        rs(caller),
        w(buyer),
        w(escrow),
        w(vault),
        r(freeze_pda(&buyer)),
        r(config_pda()),
        r(TOKEN_PROGRAM_ID),
    ];
    accounts.extend(recipient_accounts.iter().map(|a| w(*a)));
    Instruction { program_id: PROGRAM_ID, accounts, data: ix_disc("release_escrow").to_vec() }
}

pub fn ix_refund_escrow(
    caller: Address,
    escrow: Address,
    vault: Address,
    buyer_token_account: Address,
    buyer_wallet: Address,
) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            rs(caller),
            w(escrow),
            w(vault),
            w(buyer_token_account),
            w(buyer_wallet),
            r(config_pda()),
            r(TOKEN_PROGRAM_ID),
        ],
        data: ix_disc("refund_escrow").to_vec(),
    }
}

pub fn ix_raise_dispute(caller: Address, escrow: Address) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![rs(caller), w(escrow)],
        data: ix_disc("raise_dispute").to_vec(),
    }
}

pub fn ix_freeze_wallet(authority: Address, wallet: Address) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            ws(authority),
            r(config_pda()),
            w(freeze_pda(&wallet)),
            r(wallet),
            r(SYSTEM_PROGRAM_ID),
        ],
        data: ix_disc("freeze_wallet").to_vec(),
    }
}

pub fn ix_update_gateway_fee(authority: Address, fee_bps: u16) -> Instruction {
    let mut data = ix_disc("update_gateway_fee").to_vec();
    data.extend_from_slice(&fee_bps.to_le_bytes());
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![ws(authority), w(config_pda())],
        data,
    }
}

pub fn ix_propose_authority(authority: Address, new_authority: Address) -> Instruction {
    let mut data = ix_disc("propose_authority").to_vec();
    data.extend_from_slice(new_authority.as_ref());
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            ws(authority),
            r(config_pda()),
            w(pending_authority_pda()),
            r(SYSTEM_PROGRAM_ID),
        ],
        data,
    }
}

pub fn ix_accept_authority(new_authority: Address, previous_authority: Address) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            rs(new_authority),
            w(config_pda()),
            w(pending_authority_pda()),
            w(previous_authority),
        ],
        data: ix_disc("accept_authority").to_vec(),
    }
}

pub fn ix_set_treasury(authority: Address, new_treasury: Address) -> Instruction {
    let mut data = ix_disc("set_treasury").to_vec();
    data.extend_from_slice(new_treasury.as_ref());
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![rs(authority), w(config_pda())],
        data,
    }
}

pub fn ix_spl_transfer(from: Address, to: Address, owner: Address, amount: u64) -> Instruction {
    // SPL Token `Transfer` = instruction tag 3 + u64 amount.
    let mut data = vec![3u8];
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: TOKEN_PROGRAM_ID,
        accounts: vec![w(from), w(to), rs(owner)],
        data,
    }
}

// ─── Environment ─────────────────────────────────────────────────────────────

pub fn programdata_address() -> Address {
    Address::find_program_address(&[PROGRAM_ID.as_ref()], &BPF_LOADER_UPGRADEABLE_ID).0
}

pub struct Env {
    pub svm: LiteSVM,
    /// Program upgrade authority and gateway authority.
    pub authority: Keypair,
    pub treasury: Address,
    pub fee_bps: u16,
}

/// Program binary under test. `PAYROGEN_SO` overrides the default path, which
/// is useful for running the suite against another build (e.g. to confirm a
/// regression test fails on the vulnerable version).
pub fn program_bytes() -> Vec<u8> {
    let path = std::env::var("PAYROGEN_SO").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../target/deploy/payrogen.so").to_string()
    });
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!("{path}: {e}. Build the program first: `anchor build` (or cargo build-sbf).")
    })
}

/// Loads the program as an upgradeable program whose upgrade authority is
/// `upgrade_authority` (None = immutable).
pub fn new_svm(upgrade_authority: Option<&Address>) -> LiteSVM {
    let mut svm = LiteSVM::new();
    svm.add_program(PROGRAM_ID, &program_bytes()).unwrap();

    // LiteSVM creates ProgramData with no upgrade authority; patch it in.
    // Layout: u32 tag (3) | u64 slot | u8 option | [u8; 32] authority | elf
    let pd_addr = programdata_address();
    let mut pd = svm.get_account(&pd_addr).expect("programdata");
    assert_eq!(u32::from_le_bytes(pd.data[0..4].try_into().unwrap()), 3);
    match upgrade_authority {
        Some(a) => {
            pd.data[12] = 1;
            pd.data[13..45].copy_from_slice(a.as_ref());
        }
        None => {
            pd.data[12] = 0;
            pd.data[13..45].fill(0);
        }
    }
    svm.set_account(pd_addr, pd).unwrap();
    svm
}

pub fn funded_keypair(svm: &mut LiteSVM) -> Keypair {
    let kp = Keypair::new();
    svm.airdrop(&kp.pubkey(), 100 * LAMPORTS_PER_SOL).unwrap();
    kp
}

/// Upgradeable program + initialized gateway (authority == upgrade authority).
pub fn setup(fee_bps: u16) -> Env {
    let authority = Keypair::new();
    let mut svm = new_svm(Some(&authority.pubkey()));
    svm.airdrop(&authority.pubkey(), 100 * LAMPORTS_PER_SOL).unwrap();
    let treasury = Address::new_unique();
    let res = send(
        &mut svm,
        &[ix_initialize_gateway(authority.pubkey(), treasury, fee_bps)],
        &authority,
        &[],
    );
    assert_ok(&res);
    Env { svm, authority, treasury, fee_bps }
}

pub fn send(
    svm: &mut LiteSVM,
    ixs: &[Instruction],
    payer: &Keypair,
    extra_signers: &[&Keypair],
) -> TransactionResult {
    let mut signers: Vec<&Keypair> = vec![payer];
    signers.extend_from_slice(extra_signers);
    let tx = Transaction::new_signed_with_payer(
        ixs,
        Some(&payer.pubkey()),
        &signers,
        svm.latest_blockhash(),
    );
    let res = svm.send_transaction(tx);
    svm.expire_blockhash();
    res
}

pub fn assert_ok(res: &TransactionResult) {
    if let Err(e) = res {
        panic!("expected success, got {:?}\nlogs:\n{}", e.err, e.meta.logs.join("\n"));
    }
}

/// Asserts the transaction failed with the named Anchor/PayRogen error code.
pub fn assert_err(res: &TransactionResult, code_name: &str) {
    match res {
        Ok(meta) => panic!(
            "expected failure `{code_name}`, but transaction succeeded\nlogs:\n{}",
            meta.logs.join("\n")
        ),
        Err(e) => {
            let needle = format!("Error Code: {code_name}.");
            assert!(
                e.meta.logs.iter().any(|l| l.contains(&needle)),
                "expected `{code_name}`, got {:?}\nlogs:\n{}",
                e.err,
                e.meta.logs.join("\n")
            );
        }
    }
}

pub fn now(svm: &LiteSVM) -> i64 {
    svm.get_sysvar::<Clock>().unix_timestamp
}

pub fn warp_seconds(svm: &mut LiteSVM, seconds: i64) {
    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp += seconds;
    clock.slot += 1;
    svm.set_sysvar::<Clock>(&clock);
}

// ─── SPL token fixtures (packed directly; Token program is LiteSVM default) ──

pub fn create_mint(svm: &mut LiteSVM, decimals: u8) -> Address {
    let mint = Address::new_unique();
    let mut data = vec![0u8; 82];
    data[0..4].copy_from_slice(&1u32.to_le_bytes()); // mint_authority: Some
    data[4..36].copy_from_slice(Address::new_unique().as_ref());
    data[36..44].copy_from_slice(&u64::MAX.to_le_bytes()); // supply
    data[44] = decimals;
    data[45] = 1; // is_initialized
    let lamports = svm.minimum_balance_for_rent_exemption(82);
    svm.set_account(
        mint,
        Account { lamports, data, owner: TOKEN_PROGRAM_ID, executable: false, rent_epoch: 0 },
    )
    .unwrap();
    mint
}

pub fn create_token_account(svm: &mut LiteSVM, mint: &Address, owner: &Address, amount: u64) -> Address {
    let address = Address::new_unique();
    let mut data = vec![0u8; 165];
    data[0..32].copy_from_slice(mint.as_ref());
    data[32..64].copy_from_slice(owner.as_ref());
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data[108] = 1; // AccountState::Initialized
    let lamports = svm.minimum_balance_for_rent_exemption(165);
    svm.set_account(
        address,
        Account { lamports, data, owner: TOKEN_PROGRAM_ID, executable: false, rent_epoch: 0 },
    )
    .unwrap();
    address
}

pub fn token_balance(svm: &LiteSVM, token_account: &Address) -> u64 {
    let acc = svm.get_account(token_account).expect("token account exists");
    u64::from_le_bytes(acc.data[64..72].try_into().unwrap())
}

pub fn lamports(svm: &LiteSVM, address: &Address) -> u64 {
    svm.get_account(address).map(|a| a.lamports).unwrap_or(0)
}

pub fn account_closed(svm: &LiteSVM, address: &Address) -> bool {
    match svm.get_account(address) {
        None => true,
        Some(a) => a.lamports == 0 && a.data.is_empty(),
    }
}

// ─── Program account decoding (fixed offsets, as off-chain clients use) ──────

pub struct ConfigView {
    pub authority: Address,
    pub treasury: Address,
    pub fee_bps: u16,
}

pub fn read_config(svm: &LiteSVM) -> ConfigView {
    let acc = svm.get_account(&config_pda()).expect("config");
    assert_eq!(acc.data.len(), 75, "GatewayConfig layout must stay 75 bytes");
    ConfigView {
        authority: Address::try_from(&acc.data[8..40]).unwrap(),
        treasury: Address::try_from(&acc.data[40..72]).unwrap(),
        fee_bps: u16::from_le_bytes(acc.data[72..74].try_into().unwrap()),
    }
}

/// Escrow status byte: offset 8 + 32*3 + 8 + 32 = 144.
pub fn escrow_status(svm: &LiteSVM, escrow: &Address) -> u8 {
    svm.get_account(escrow).expect("escrow").data[144]
}

/// Escrow mint: appended after `bump` (offset 332).
pub fn escrow_mint(svm: &LiteSVM, escrow: &Address) -> Address {
    Address::try_from(&svm.get_account(escrow).expect("escrow").data[333..365]).unwrap()
}

pub const STATUS_LOCKED: u8 = 0;
pub const STATUS_DISPUTED: u8 = 3;
