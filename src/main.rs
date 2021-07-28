use std::{convert::Infallible, env, fs::File, path::Path, sync::Arc};

// external
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use tokio::sync::Mutex;
use warp::{filters::ws::Message, ws::WebSocket, Filter, Rejection, Reply};

// here
use celestium::{
    block::Block,
    block_hash::BlockHash,
    serialize::{DynamicSized, Serialize},
    transaction::Transaction,
    wallet::{Wallet, DEFAULT_N_THREADS, DEFAULT_PAR_WORK},
};

type SharedWallet = Arc<Mutex<Wallet>>;

// Path::new(wallet_dir()) - der hvor du skal bruge den
fn wallet_dir() -> String {
    env::var("CELESTIUM_DATA_DIR")
        .map(|s| s.to_string())
        .unwrap_or("/data".to_string())
}

fn load_wallet() -> Option<Wallet> {
    // TODO load wallet from disk
    println!("Attempting to load state from disk.");
    let dir = wallet_dir();

    // blockchain
    // let blockchain =

    // pk
    // sk
    // mf_branches
    // mf_leafs
    // root_lookup
    // off_chain_transactions
    None
}

fn save_wallet(wallet: Wallet) {
    // TODO
}

#[tokio::main]
async fn main() {
    // TODO: do this properly, from disk
    // initialize wallet
    let (pk, sk) = Wallet::generate_ec_keys();
    let wallet = Wallet::new(pk, sk, true);
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

    // it parses! add transaction to queue
    if let Err(e) = wallet.lock().await.add_off_chain_transaction(*transaction) {
        ws_error(format!("Error: Could not add transaction: {}", e), sender).await;
    }
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
