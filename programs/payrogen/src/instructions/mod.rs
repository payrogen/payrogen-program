//! # Instruction Handlers
//!
//! This module contains the business logic for each PayRogen smart contract instruction.
//! Each sub-module exports a `handler` function invoked by the corresponding instruction
//! entry point in `lib.rs`.

pub mod authority;
pub mod create_escrow;
pub mod execute_sol_split_payment;
pub mod execute_split_payment;
pub mod freeze_wallet;
pub mod initialize_gateway;
pub mod raise_dispute;
pub mod refund_escrow;
pub mod release_escrow;
pub mod update_gateway_fee;
