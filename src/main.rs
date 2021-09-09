use std::{
    collections::HashMap,
    convert::Infallible,
    env,
    fs::read,
    fs::File,
    io::prelude::*,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

// external
use cached::proc_macro::cached;
use futures::{SinkExt, StreamExt, TryFutureExt};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::{filters::ws::Message, Filter, Rejection, Reply};

// here
use crate::canvas::PIXEL_HASH_SIZE;
use celestium::{
    merkle_forest::HASH_SIZE,
    serialize::Serialize,
    transaction::{Transaction, BASE_TRANSACTION_MESSAGE_LEN},
    wallet::{BinaryWallet, Wallet},
};

mod canvas;

////////////////////////////////////////////////// !!! !!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
type SharedCanvas = Arc<Mutex<canvas::Canvas>>; // !!! IMPORTANT! Always lock SharedCanvas before SharedWallet  !!!
type SharedWallet = Arc<Mutex<Box<Wallet>>>; ///// !!! IMPORTANT! to await potential dead locks (if using both) !!!
type Clients = Arc<RwLock<HashMap<usize, mpsc::UnboundedSender<Message>>>>;

static NEXT_USER_ID: AtomicUsize = AtomicUsize::new(1);

#[repr(u8)]
#[derive(FromPrimitive)]
enum CMDOpcodes {
    Error = 0x00,
    GetEntireImage = 0x01,
    EntireImage = 0x02,
    UpdatePixel = 0x03,
    UnminedTransaction = 0x04,
    MinedTransaction = 0x05,
    GetPixelData = 0x06,
    PixelData = 0x07,
    GetStoreItems = 0x08,
    StoreItems = 0x09,
    BuyStoreItem = 0x0a,
    GetUserData = 0x0b,
    UserData = 0x0c,
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
    print!("Initializing canvas...");
    let canvas = canvas::Canvas::new_test();
    let shared_canvas = Arc::new(Mutex::new(canvas));
    println!(" Done!");

    // Keep track of all connected users, key is usize, value
    // is a websocket sender.
    let clients = Clients::default();
    // Turn our "state" into a new Filter...
    let clients = warp::any().map(move || clients.clone());

    // configure ws route
    let ws_route = warp::path::end()
        .and(warp::ws())
        .and(with_wallet(shared_wallet))
        .and(with_canvas(shared_canvas))
        .and(clients)
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
    clients: Clients,
) -> Result<impl Reply, Rejection> {
    // weird boilerplate because I don't know why
    // this async function seems to just pass stuff on to another async function
    // but I don't know how to inline it 🤷
    Ok(ws.on_upgrade(move |socket| client_connection(socket, wallet, canvas, clients)))
}

macro_rules! ws_error {
    // print the error, send it out over websockets
    // and exit function early
    ($sender: expr, $errmsg: expr) => {
        println!("{}", $errmsg);
        $sender.send(Message::text($errmsg)).unwrap();
        return;
    };
}

macro_rules! unwrap_or_ws_error {
    ($sender: expr, $result: expr) => {
        match $result {
            Ok(r) => r,
            Err(e) => {
                ws_error!($sender, e);
            }
        }
    };
}

async fn client_connection(
    ws: warp::ws::WebSocket,
    wallet: SharedWallet,
    canvas: SharedCanvas,
    clients: Clients,
) {
    // keeps a client connection open, pass along incoming messages
    println!("Establishing client connection... {:?}", ws);
    let (mut sender, mut receiver) = ws.split();

    let my_id = NEXT_USER_ID.fetch_add(1, Ordering::Relaxed);

    let (tx, rx) = mpsc::unbounded_channel();
    let mut rx = UnboundedReceiverStream::new(rx);

    tokio::task::spawn(async move {
        while let Some(message) = rx.next().await {
            sender
                .send(message)
                .unwrap_or_else(|e| {
                    println!("websocket send error: {}", e);
                })
                .await;
        }
    });

    // Save the sender in our list of connected users.
    clients.write().await.insert(my_id, tx);

    while let Some(body) = receiver.next().await {
        let message = match body {
            Ok(msg) => msg,
            Err(e) => {
                println!("error reading message on websocket: {}", e);
                break;
            }
        };
        handle_ws_message(message, my_id, wallet.clone(), canvas.clone(), &clients).await;
    }

    clients.write().await.remove(&my_id);
}

async fn handle_ws_message(
    message: Message,
    my_id: usize,
    wallet: SharedWallet,
    canvas: SharedCanvas,
    clients: &Clients,
) {
    // this is the function that actually receives a message
    // validate it, add it to the blockchain, then exit.

    let real_clients = clients.read().await;
    let sender = real_clients.get(&my_id).unwrap();

    if !message.is_binary() {
        ws_error!(sender, "Expected binary WS message.".to_string());
    }

    let binary_message = message.as_bytes();
    match FromPrimitive::from_u8(binary_message[0]) {
        Some(CMDOpcodes::GetEntireImage) => {
            let real_canvas = canvas.lock().await;
            let mut entire_image = real_canvas.serialize_colors();
            entire_image.insert(0, CMDOpcodes::EntireImage as u8);
            drop(real_canvas);
            if let Err(e) = sender.send(Message::binary(entire_image)) {
                ws_error!(sender, format!("Error sending canvas: {}", e));
            }
        }
        Some(CMDOpcodes::MinedTransaction) => {
            parse_transaction(&binary_message[1..], sender, wallet, canvas, clients).await
        }
        Some(CMDOpcodes::GetPixelData) => {
            parse_get_pixel_data(&binary_message[1..], sender, wallet, canvas).await
        }
        _ => {
            ws_error!(
                sender,
                format!("Unexpeted CMD Opcode {}", binary_message[0])
            );
        }
    }
}

async fn parse_get_pixel_data(
    bin_parameters: &[u8],
    sender: &mpsc::UnboundedSender<Message>,
    wallet: SharedWallet,
    canvas: SharedCanvas,
) {
    if bin_parameters.len() != 4 {
        ws_error!(
            sender,
            format!(
                "Expected message len of {} for CMD Opcode {:x} (Get pixel hash) got {}",
                5,
                CMDOpcodes::GetPixelData as u8,
                bin_parameters.len()
            )
        );
    }
    let mut pixel_hash_response = [0u8; 1 + PIXEL_HASH_SIZE + HASH_SIZE];
    pixel_hash_response[0] = CMDOpcodes::PixelData as u8;
    {
        let real_canvas = canvas.lock().await;
        let x: usize = ((bin_parameters[0] as usize) << 8) + (bin_parameters[1] as usize);
        let y: usize = ((bin_parameters[2] as usize) << 8) + (bin_parameters[3] as usize);
        let pixel = unwrap_or_ws_error!(sender, real_canvas.get_pixel(x, y));
        pixel_hash_response[1..PIXEL_HASH_SIZE + 1].copy_from_slice(&pixel.hash);
        drop(real_canvas);
    }
    {
        let real_wallet = wallet.lock().await;
        let block_hash = real_wallet.get_head_hash();
        pixel_hash_response[PIXEL_HASH_SIZE + 1..].copy_from_slice(&block_hash);
        drop(real_wallet);
    }

    if let Err(e) = sender.send(Message::binary(pixel_hash_response)) {
        println!("Could not send pixel hash: {}", e);
    };
}

async fn parse_transaction(
    bin_transaction: &[u8],
    sender: &mpsc::UnboundedSender<Message>,
    wallet: SharedWallet,
    canvas: SharedCanvas,
    clients: &Clients,
) {
    let transaction = unwrap_or_ws_error!(
        sender,
        Transaction::from_serialized(bin_transaction, &mut 0)
    );

    let mut real_wallet = wallet.lock().await;
    if let Err(e) = real_wallet.add_off_chain_transaction(*transaction.clone()) {
        ws_error!(sender, e);
    }

    if let Err(e) = save_wallet(&real_wallet) {
        ws_error!(sender, e);
    }
    drop(real_wallet);

    let base_transaction_message =
        unwrap_or_ws_error!(sender, transaction.get_base_transaction_message())
            as [u8; BASE_TRANSACTION_MESSAGE_LEN];

    let mut real_canvas = canvas.lock().await;
    let (x, y, pixel) = unwrap_or_ws_error!(
        sender,
        canvas::Canvas::parse_pixel(base_transaction_message)
    );
    let current_pixel_hash = unwrap_or_ws_error!(sender, real_canvas.get_pixel(x, y))
        .hash
        .to_vec();
    let new_pixel_back_ref_hash = base_transaction_message[..PIXEL_HASH_SIZE].to_vec();
    if current_pixel_hash != new_pixel_back_ref_hash {
        ws_error!(
            sender,
            format!(
                "Got wrong pixel hash expected {:x?} got {:x?}",
                current_pixel_hash, new_pixel_back_ref_hash,
            )
        );
    } else {
        unwrap_or_ws_error!(sender, real_canvas.set_pixel(x, y, pixel));
        let mut update_pixel_binary_message = [0u8; 6];
        update_pixel_binary_message[0] = CMDOpcodes::UpdatePixel as u8;
        update_pixel_binary_message[1..]
            .copy_from_slice(&base_transaction_message[PIXEL_HASH_SIZE..]);
        println!("Len: {}", clients.read().await.len());
        for tx in clients.read().await.values() {
            if let Err(e) = tx.send(Message::binary(update_pixel_binary_message)) {
                println!("Could not send updated pixel: {}", e);
            }
        }
    }
    drop(real_canvas);
}
