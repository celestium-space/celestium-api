use std::{
    io,
    io::prelude::*,
    convert::Infallible,
    env,
    fs::read,
    fs::File,
    path::PathBuf,
    sync::Arc
};

// external
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use tokio::sync::Mutex;
use warp::{filters::ws::Message, ws::WebSocket, Filter, Rejection, Reply};
use cached::proc_macro::cached;

// here
use celestium::{
    block::Block,
    block_hash::BlockHash,
    serialize::{DynamicSized, Serialize},
    transaction::Transaction,
    wallet::{Wallet, BinaryWallet, DEFAULT_N_THREADS, DEFAULT_PAR_WORK},
};

type SharedWallet = Arc<Mutex<Wallet>>;

#[tokio::main]
async fn main() {
    // TODO: do this properly, from disk
    // initialize wallet
    let wallet = match load_wallet() {
        Ok(w) => { w }
        Err(e) => {
            println!("Failed loading wallet: {}", e);
            match generate_wallet() {
                Ok(w) => { w }
                Err(e) => { panic!("Failed generating wallet: {}", e) }
            }
        }
    };

    let shared_wallet = Arc::new(Mutex::new(wallet));

    // configure ws route
    let ws_route = warp::path::end()
        .and(warp::ws())
        .and(with_wallet(shared_wallet.clone()))
        .and_then(ws_handler)
        .with(warp::cors().allow_any_origin());

    // GO! GO! GO!
    println!("Starting server");
    warp::serve(ws_route).run(([0, 0, 0, 0], 8000)).await;
}


/* WALLET PERSISTENCE */

#[cached]
fn wallet_dir() -> PathBuf {
    // return path to data on filesystem
    // memoized because we don't need to make the syscalls errytim
    let path = PathBuf::from(
        env::var("CELESTIUM_DATA_DIR")
            .map(|s| s.to_string())
            .unwrap_or("/data".to_string())
    );
    assert!(path.exists(), "Celestium data path doesn't exist!");
    assert!(path.is_dir(), "Celestium data path is not a directory!");
    path
}

fn load_wallet() -> Result<Wallet, String> {
    // load wallet from disk. fails if anything is missing
    println!("Trying to load wallet from disk.");
    let dir: PathBuf = wallet_dir();
    let load = |filename: &str| read(dir.join(filename)).map_err(|e| e.to_string());
    Wallet::from_binary(&BinaryWallet {
        blockchain_bin:             load("blockchain")?,
        pk_bin:                     load("pk")?,
        sk_bin:                     load("sk")?,
        mf_branches_bin:            load("mf_branches")?,
        mf_leafs_bin:               load("mf_leafs")?,
        unspent_outputs_bin:        load("unspent_outputs")?,
        root_lookup_bin:            load("root_lookup")?,
        off_chain_transactions_bin: load("off_chain_transactions")?,
    }, false)
}

fn save_wallet(wallet: &Wallet) -> Result<(), String> {
    // write members of the wallet struct to disk
    // overwrites whatever was in the way
    println!("Writing wallet to disk.");
    let dir = wallet_dir();
    let wallet_bin = wallet.to_binary()?;
    let save = |filename: &str, data: Vec<u8>|
        File::create(dir.join(filename))
        .map(|mut f| f.write_all(&data).map_err(|e| e.to_string()))
        .map_err(|e| e.to_string());
    save("blockchain", wallet_bin.blockchain_bin)??;
    save("pk", wallet_bin.pk_bin)??;
    save("sk", wallet_bin.sk_bin)??;
    save("mf_branches_bin", wallet_bin.mf_branches_bin)??;
    save("mf_leafs_bin", wallet_bin.mf_leafs_bin)??;
    save("mf_unspent_outputs_bin", wallet_bin.unspent_outputs_bin)??;
    save("root_lookup_bin", wallet_bin.root_lookup_bin)??;
    save("off_chain_transactions_bin", wallet_bin.off_chain_transactions_bin)??;
    Ok(())
}

fn generate_wallet() -> Result<Wallet, String> {
    // make new wallet, write it to disk
    println!("Generating new wallet.");
    let (pk, sk) = Wallet::generate_ec_keys();
    let wallet = Wallet::new(pk, sk, true);
    save_wallet(&wallet)?;
    Ok(wallet)
}


/* WARP STUFF */

fn with_wallet(
    wallet: SharedWallet,
) -> impl Filter<Extract = (SharedWallet,), Error = Infallible> + Clone {
    // warp filters - how do they work?
    warp::any().map(move || wallet.clone())
}

async fn ws_handler(ws: warp::ws::Ws, wallet: SharedWallet) -> Result<impl Reply, Rejection> {
    // weird boilerplate because I don't know why
    // this async function seems to just pass stuff on to another async function
    // but I don't know how to inline it 🤷
    Ok(ws.on_upgrade(move |socket| client_connection(socket, wallet)))
}

async fn client_connection(ws: warp::ws::WebSocket, wallet: SharedWallet) {
    // keeps a client connection open, pass along incoming messages
    println!("establishing client connection... {:?}", ws);
    let (mut sender, mut receiver) = ws.split();
    while let Some(body) = receiver.next().await {
        let message = match body {
            Ok(msg) => msg,
            Err(e) => {
                println!("error reading message on websocket: {}", e);
                break;
            }
        };
        handle_ws_message(message, &mut sender, wallet.clone()).await;
    }
}

async fn ws_error(errmsg: String, sender: &mut SplitSink<WebSocket, Message>) {
    println!("{}", errmsg);
    sender.send(Message::text(errmsg)).await.unwrap();
}

async fn handle_ws_message(
    message: Message,
    sender: &mut SplitSink<WebSocket, Message>,
    wallet: SharedWallet,
) {
    // this is the function that actually receives a message
    // validate it, add it to the blockchain, then exit.

    if !message.is_binary() {
        ws_error(format!("Error: expected binary transaction."), sender).await;
        return;
    }

    // parse binary transaction
    let bin_transaction = message.as_bytes();
    let transaction = match Transaction::from_serialized(&bin_transaction, &mut 0) {
        Ok(transaction) => transaction,
        Err(e) => {
            ws_error(format!("Error: Could not parse transaction: {}", e), sender).await;
            return;
        }
    };

    {   // transaction parses! get the shared wallet.
        let mut real_wallet = wallet.lock().await;

        // add transaction to queue
        if let Err(e) = real_wallet.add_off_chain_transaction(*transaction) {
            ws_error(format!("Error: Could not add transaction: {}", e), sender).await;
        }

        // store new wallet
        if let Err(e) = save_wallet(&real_wallet) {
            ws_error(
                format!("Error: valid transaction couldn't be stored: {}", e),
                sender
            ).await;
        }

        drop(real_wallet)
    }   // mutex lock released
}

fn celestium_example() {
    // don't need this ad verbatim - just for reference
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
