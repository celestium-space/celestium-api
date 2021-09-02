use std::{convert::Infallible, env, fs::read, fs::File, io::prelude::*, path::PathBuf, sync::Arc};

// external
use cached::proc_macro::cached;
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use tokio::sync::Mutex;
use warp::{filters::ws::Message, ws::WebSocket, Filter, Rejection, Reply};

// here
use celestium::{
    serialize::Serialize,
    transaction::{self, Transaction},
    wallet::{BinaryWallet, Wallet},
};

mod canvas;

type SharedWallet = Arc<Mutex<Box<Wallet>>>;
type SharedCanvas = Arc<Mutex<canvas::Canvas>>;

#[repr(u8)]
#[derive(FromPrimitive)]
enum CMDOpcodes {
    EntireImage = 0,
    UpdatePixel = 1,
    Error = 2,
    Transaction = 3,
}

#[tokio::main]
async fn main() {
    // initialize wallet
    let wallet = match load_wallet() {
        Ok(w) => w,
        Err(e) => {
            println!("Failed loading wallet: {}", e);
            match generate_wallet() {
                Ok(w) => w,
                Err(e) => {
                    panic!("Failed generating wallet: {}", e)
                }
            }
        }
    };
    let shared_wallet = Arc::new(Mutex::new(Box::new(wallet)));

    // initialize empty canvas
    println!("Initializing canvas...");
    let canvas = canvas::Canvas::new_test();
    let shared_canvas = Arc::new(Mutex::new(canvas));
    println!("Canvas initialized.");

    // configure ws route
    let ws_route = warp::path::end()
        .and(warp::ws())
        .and(with_wallet(shared_wallet.clone()))
        .and(with_canvas(shared_canvas.clone()))
        .and_then(ws_handler)
        .with(warp::cors().allow_any_origin());

    // GO! GO! GO!
    println!("Starting server");
    let port = env::var("API_PORT")
        .unwrap_or_else(|_| 8000.to_string())
        .parse::<u16>()
        .unwrap();
    warp::serve(ws_route).run(([0, 0, 0, 0], port)).await;
}

/* WALLET PERSISTENCE */

#[cached]
fn wallet_dir() -> PathBuf {
    // return path to data on filesystem
    // memoized because we don't need to make the syscalls errytim
    let path =
        PathBuf::from(env::var("CELESTIUM_DATA_DIR").unwrap_or_else(|_| "/data".to_string()));
    assert!(path.exists(), "Celestium data path doesn't exist!");
    assert!(path.is_dir(), "Celestium data path is not a directory!");
    path
}

fn load_wallet() -> Result<Wallet, String> {
    // load wallet from disk. fails if anything is missing
    println!("Trying to load wallet from disk.");
    let dir: PathBuf = wallet_dir();
    let load = |filename: &str| read(dir.join(filename)).map_err(|e| e.to_string());
    Wallet::from_binary(
        &BinaryWallet {
            blockchain_bin: load("blockchain")?,
            pk_bin: load("pk")?,
            sk_bin: load("sk")?,
            mf_branches_bin: load("mf_branches")?,
            mf_leafs_bin: load("mf_leafs")?,
            unspent_outputs_bin: load("unspent_outputs")?,
            root_lookup_bin: load("root_lookup")?,
            off_chain_transactions_bin: load("off_chain_transactions")?,
        },
        false,
    )
}

fn save_wallet(wallet: &Wallet) -> Result<(), String> {
    // write members of the wallet struct to disk
    // overwrites whatever was in the way
    println!("Writing wallet to disk.");
    let dir = wallet_dir();
    let wallet_bin = wallet.to_binary()?;
    let save = |filename: &str, data: Vec<u8>| {
        File::create(dir.join(filename))
            .map(|mut f| f.write_all(&data).map_err(|e| e.to_string()))
            .map_err(|e| e.to_string())
    };
    save("blockchain", wallet_bin.blockchain_bin)??;
    save("pk", wallet_bin.pk_bin)??;
    save("sk", wallet_bin.sk_bin)??;
    save("mf_branches", wallet_bin.mf_branches_bin)??;
    save("mf_leafs", wallet_bin.mf_leafs_bin)??;
    save("unspent_outputs", wallet_bin.unspent_outputs_bin)??;
    save("root_lookup", wallet_bin.root_lookup_bin)??;
    save(
        "off_chain_transactions",
        wallet_bin.off_chain_transactions_bin,
    )??;
    println!("Wrote wallet to disk.");
    Ok(())
}

fn generate_wallet() -> Result<Wallet, String> {
    // make new wallet, write it to disk
    // TODO: this should probably panic if there's a partial wallet in the way
    println!("Generating new wallet.");
    let wallet = Wallet::generate_init_blockchain(true)?;
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

fn with_canvas(
    canvas: SharedCanvas,
) -> impl Filter<Extract = (SharedCanvas,), Error = Infallible> + Clone {
    // warp filters - how do they work?
    warp::any().map(move || canvas.clone())
}

async fn ws_handler(
    ws: warp::ws::Ws,
    wallet: SharedWallet,
    canvas: SharedCanvas,
) -> Result<impl Reply, Rejection> {
    // weird boilerplate because I don't know why
    // this async function seems to just pass stuff on to another async function
    // but I don't know how to inline it 🤷
    Ok(ws.on_upgrade(move |socket| client_connection(socket, wallet, canvas)))
}

macro_rules! ws_error {
    // print the error, send it out over websockets
    // and exit function early
    ($sender: expr, $errmsg: expr) => {
        println!("{}", $errmsg);
        $sender.send(Message::text($errmsg)).await.unwrap();
        return;
    };
}

async fn client_connection(ws: warp::ws::WebSocket, wallet: SharedWallet, canvas: SharedCanvas) {
    // keeps a client connection open, pass along incoming messages
    println!("Establishing client connection... {:?}", ws);
    let (mut sender, mut receiver) = ws.split();
    {
        // get current canvas, send it to the new client
        let real_canvas = canvas.lock().await;
        let mut initial_pic = real_canvas.to_owned().serialize_colors();
        initial_pic.insert(0, CMDOpcodes::EntireImage as u8);
        drop(real_canvas);
        // canvas released
        if let Err(e) = sender.send(Message::binary(initial_pic)).await {
            ws_error!(sender, format!("Error sending canvas: {}", e));
        }
    }

    while let Some(body) = receiver.next().await {
        let message = match body {
            Ok(msg) => msg,
            Err(e) => {
                println!("error reading message on websocket: {}", e);
                break;
            }
        };
        handle_ws_message(message, &mut sender, wallet.clone(), canvas.clone()).await;
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
    canvas: SharedCanvas,
) {
    // this is the function that actually receives a message
    // validate it, add it to the blockchain, then exit.

    if !message.is_binary() {
        ws_error!(sender, "Error: expected binary WS message.".to_string());
    }

    let binary_message = message.as_bytes();
    match FromPrimitive::from_u8(binary_message[0]) {
        Some(CMDOpcodes::Transaction) => {
            parse_transaction(&binary_message[1..], sender, wallet, canvas)
        }
        _ => {
            ws_error!(
                sender,
                format!("Unexpeted CMD Opcode {}", binary_message[0])
            );
        }
    }
    .await;
}

async fn parse_transaction(
    bin_transaction: &[u8],
    sender: &mut SplitSink<WebSocket, Message>,
    wallet: SharedWallet,
    canvas: SharedCanvas,
) {
    // parse binary transaction
    let transaction = match Transaction::from_serialized(bin_transaction, &mut 0) {
        Ok(transaction) => transaction,
        Err(e) => {
            ws_error!(sender, format!("Error: Could not parse transaction: {}", e));
        }
    };

    {
        // transaction parses! get the shared wallet.
        let mut real_wallet = wallet.lock().await;

        // add transaction to queue
        if let Err(e) = real_wallet.add_off_chain_transaction(*transaction.clone()) {
            ws_error(format!("Error: Could not add transaction: {}", e), sender).await;
        }

        // store new wallet
        if let Err(e) = save_wallet(&real_wallet) {
            ws_error(
                format!("Error: valid transaction couldn't be stored: {}", e),
                sender,
            )
            .await;
        }

        drop(real_wallet)
    } // mutex lock released

    // if the transaction has a base message, it's likely a pixel NFT
    // XXX: this will change later, when we add voting NFTs
    if let Ok(base_transaction_message) = transaction.get_base_transaction_message() {
        // get canvas and add the new transaction
        let real_canvas = canvas.lock().await;
        if let Err(e) = transaction_to_canvas(real_canvas.to_owned(), base_transaction_message) {
            ws_error!(sender, format!("Error: {}", e));
        }
        drop(real_canvas);
    } // shared canvas released

    // TODO inform all the other ws clients about the new pixel
}

fn transaction_to_canvas(
    canvas: canvas::Canvas,
    base_transaction_message: [u8; transaction::BASE_TRANSACTION_MESSAGE_LEN],
) -> Result<(), String> {
    // extract and parse pixel data
    let mut pixeldata = [0u8; 37];
    pixeldata.copy_from_slice(&base_transaction_message[0..38]);
    let (x, y, pixel): (usize, usize, canvas::Pixel) = canvas::Canvas::parse_pixel(pixeldata)?;

    // add it to the canvas
    canvas.set_pixel(x, y, pixel)?;
    Ok(())
}
