use celestium::{
    block::Block,
    block_hash::BlockHash,
    serialize::{DynamicSized, Serialize},
    transaction::Transaction,
    wallet::{Wallet, DEFAULT_N_THREADS, DEFAULT_PAR_WORK},
};
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::time::Instant;
use std::{fs::remove_file, fs::File, io::prelude::*};

fn main() {
    let (pk, sk) = Wallet::generate_ec_keys();
    let mut wallet = Wallet::new(pk, sk, true);
    let bin_transaction = vec![0u8; 32]; // Got from Webesocket
    match Transaction::from_serialized(&bin_transaction, &mut 0) {
        Ok(transaction) => {
            if let Err(e) = wallet.add_off_chain_transaction(*transaction) {
                println!("Could not add transaction: {}", e);
            }
        }
        Err(e) => println!("Could not parse transaction: {}", e),
    }
}
