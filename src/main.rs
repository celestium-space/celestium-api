use chrono::Utc;
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    convert::Infallible,
    env,
    fs::read,
    fs::File,
    io::prelude::*,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering::Relaxed},
        Arc,
    },
    time::Instant,
};

// external
use cached::proc_macro::cached;
use futures::{SinkExt, StreamExt, TryFutureExt};
use mongodb::{
    bson::{doc, oid::ObjectId},
    options::ClientOptions,
    sync::{Client, Collection, Database},
};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use rand::Rng;
use rayon::ThreadPoolBuilder;
use secp256k1::PublicKey;
use serde::{Deserialize, Serialize as SerdeSerialize};
use sha3::{Digest, Sha3_256};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::{filters::ws::Message, Filter, Rejection, Reply};

// here
use crate::canvas::{Pixel, PIXEL_HASH_SIZE};
use celestium::{
    block_hash::BlockHash,
    ec_key_serialization::PUBLIC_KEY_COMPRESSED_SIZE,
    serialize::{DynamicSized, Serialize},
    transaction::{self, Transaction, BASE_TRANSACTION_MESSAGE_LEN},
    transaction_hash::TransactionHash,
    transaction_input::TransactionInput,
    transaction_output::TransactionOutput,
    transaction_value::TransactionValue,
    transaction_varuint::TransactionVarUint,
    wallet::{BinaryWallet, Wallet, DEFAULT_N_THREADS, DEFAULT_PAR_WORK, HASH_SIZE},
};

mod canvas;

type SharedCanvas = Arc<RwLock<canvas::Canvas>>;
type SharedWallet = Arc<RwLock<Wallet>>;
type SharedFloatingOutputs = Arc<RwLock<HashSet<(TransactionHash, usize)>>>;
type WSClients = Arc<RwLock<HashMap<usize, mpsc::UnboundedSender<Message>>>>;
type SharedLastSavedTime = Arc<Mutex<i64>>;

#[allow(non_snake_case)]
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StoreItem {
    _id: ObjectId,
    // a: String,
    // A1: String,
    // A2: String,
    // A3: String,
    // ad: String,
    // albedo: Option<f64>,
    // BV: String,
    // class: String,
    // closeness: Option<i32>,
    // condition_code: String,
    // data_arc: f64,
    // diameter: Option<f64>,
    // diameter_sigma: Option<f64>,
    // DT: String,
    // dv: f64,
    // e: f64,
    // epoch: f64,
    // epoch_cal: f64,
    // epoch_mjd: f64,
    // equinox: String,
    // est_diameter: f64,
    // extent: String,
    // first_obs: String,
    full_name: String,
    // G: String,
    // GM: String,
    // H: String,
    // H_sigma: Option<f64>,
    // i: f64,
    // id: String,
    // inexact: bool,
    // IR: String,
    // K1: String,
    // K2: String,
    // last_obs: String,
    // M1: String,
    // M2: String,
    // ma: String,
    // moid: String,
    // moid_jup: String,
    // moid_ld: String,
    // n: String,
    // n_del_obs_used: String,
    // n_dop_obs_used: String,
    // n_obs_used: f64,
    // name: String,
    // neo: String,
    // om: f64,
    // orbit_id: String,
    // PC: String,
    // pdes: String,
    // per: String,
    // per_y: String,
    // pha: String,
    // prefix: String,
    price: Option<f64>,
    // producer: String,
    profit: Option<f64>,
    // prov_des: String,
    // q: f64,
    // rms: String,
    // rot_per: Option<f64>,
    // saved: f64,
    // score: f64,
    // sigma_a: String,
    // sigma_ad: String,
    // sigma_e: String,
    // sigma_i: String,
    // sigma_ma: String,
    // sigma_n: String,
    // sigma_om: String,
    // sigma_per: String,
    // sigma_q: String,
    // sigma_tp: String,
    // sigma_w: String,
    // spec: String,
    // spec_B: String,
    // spec_T: String,
    // spkid: f64,
    // t_jup: String,
    // tp: f64,
    // tp_cal: f64,
    // two_body: String,
    // UB: String,
    // w: f64,
    store_value_in_dust: String,
    debris_intldes: String,
}

#[allow(non_snake_case)]
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct DebrisItem {
    _id: String,
    INTLDES: String,
    OBJECT_NAME: String,
    OBJECT_TYPE: String,
    TLE_LINE1: String,
    TLE_LINE2: String,
}

#[derive(SerdeSerialize, Deserialize, Debug)]
struct GetStoreItemData {
    image_url: String,
    store_value_in_dust: String,
}

#[derive(SerdeSerialize, Deserialize, Debug)]
struct UserData {
    balance: String,
    owned_store_items: Vec<StoreItem>,
    owned_debris: Vec<DebrisItem>,
}

static NEXT_USER_ID: AtomicUsize = AtomicUsize::new(1);
static DUST_PER_CEL: u128 = 10000000000000000000000000000000;
static DEFAULT_MONGODB_STORE_COLLECTION_NAME: &str = "asteroids";
static DEFAULT_MONGODB_DEBRIS_COLLECTION_NAME: &str = "debris";

#[repr(u8)]
#[derive(FromPrimitive, Debug)]
enum CMDOpcodes {
    GetEntireImage = 0x01,
    EntireImage = 0x02,
    UpdatePixel = 0x03,
    UnminedTransaction = 0x04,
    MinedTransaction = 0x05,
    GetPixelData = 0x06,
    PixelData = 0x07,
    GetStoreItem = 0x08,
    StoreItem = 0x09,
    BuyStoreItem = 0x0a,
    GetUserData = 0x0b,
    UserData = 0x0c,
}

#[tokio::main]
async fn main() {
    // connect to MongoDB
    let mongodb_connection_string = env::var("MONGODB_CONNECTION_STRING")
        .unwrap_or("mongodb://admin:admin@localhost/".to_string());

    let mongodb_database_name = env::var("MONGO_DATABASE_NAME").unwrap_or("asterank".to_string());

    let mut client_options = match ClientOptions::parse(&mongodb_connection_string) {
        Ok(c) => c,
        Err(e) => {
            println!("Could not create mongo client options: {}", e);
            return;
        }
    };

    // Manually set an option
    client_options.app_name = Some("celestium".to_string());
    // Get a handle to the cluster
    let mongodb_client = match Client::with_options(client_options) {
        Ok(c) => c,
        Err(e) => {
            println!("Could not set app name: {}", e);
            return;
        }
    };

    // Ping the server to see if you can connect to the cluster
    let database = mongodb_client.database(&mongodb_database_name);
    if let Err(e) = database.run_command(doc! {"ping": 1}, None) {
        println!(
            "Could not ping database \"{}\": {}",
            mongodb_database_name, e
        );
        return;
    };

    let store_collection_name: String = env::var("MONGODB_STORE_COLLECTION_NAME")
        .unwrap_or(DEFAULT_MONGODB_STORE_COLLECTION_NAME.to_string());

    if database
        .list_collection_names(doc! {"name": &store_collection_name})
        .unwrap()
        .len()
        != 1
    {
        println!(
            "Could not find collection \"{}\" in database \"{}\"",
            &store_collection_name, mongodb_database_name
        );
        return;
    }

    println!(
        "MongoDB Connected successfully, collections: {:?}",
        database.list_collection_names(doc! {}).unwrap()
    );
    let database = warp::any().map(move || database.clone());

    // initialize wallet
    let wallet = match load_wallet() {
        Ok(w) => w,
        Err(e) => {
            println!("Failed loading wallet: {}", e);
            match generate_wallet().await {
                Ok(w) => w,
                Err(e) => {
                    panic!("Failed generating wallet: {}", e)
                }
            }
        }
    };

    // initialize empty canvas
    println!("Initializing canvas...");
    let mut canvas = canvas::Canvas::new_test();
    let off_chain_transactions = wallet.off_chain_transactions.clone();

    let mut to_throw_away: HashSet<[u8; PIXEL_HASH_SIZE]> = HashSet::new();
    let mut candidates: HashMap<[u8; PIXEL_HASH_SIZE], (u16, u16, Pixel)> = HashMap::new();
    for (_, transaction) in off_chain_transactions {
        if transaction.is_id_base_transaction() {
            match canvas::Canvas::parse_pixel(transaction.get_base_transaction_message().unwrap()) {
                Ok((x, y, pixel)) => {
                    to_throw_away.insert(pixel.back_hash);
                    if candidates.remove(&pixel.back_hash).is_some() {
                        to_throw_away.insert(pixel.back_hash);
                    }
                    let hash: [u8; PIXEL_HASH_SIZE] = pixel.hash(x as u16, y as u16);
                    if !to_throw_away.contains(&hash) {
                        candidates.insert(hash, (x as u16, y as u16, pixel));
                    }
                }
                Err(e) => {
                    println!(
                        "WARNING: Could not decode one transaction base message: {}",
                        e
                    );
                }
            }
        }
    }
    println!("Found {} initial candidates", candidates.len());
    let mut actual_candidates = 0usize;
    for (x, y, p) in candidates.values() {
        if to_throw_away.contains(&p.back_hash) || p.back_hash.is_empty() {
            actual_candidates += 1;
            canvas
                .set_pixel((*x).into(), (*y).into(), p.clone())
                .unwrap();
        }
    }
    println!("Found {} acutal candidates", actual_candidates);
    let shared_canvas = Arc::new(RwLock::new(canvas));
    println!("Canvas initialized");

    let shared_wallet = Arc::new(RwLock::new(wallet));

    // Keep track of all connected users, key is usize, value
    // is a websocket sender.
    let ws_clients = WSClients::default();
    // Turn our "state" into a new Filter...
    let ws_clients = warp::any().map(move || ws_clients.clone());

    let floating_outputs = Arc::new(RwLock::new(HashSet::new()));

    let last_save_time = Arc::new(Mutex::new(Utc::now().timestamp()));

    // configure ws route
    let ws_route = warp::path::end()
        .and(warp::ws())
        .and(with_wallet(shared_wallet))
        .and(with_canvas(shared_canvas))
        .and(with_floating_outputs(floating_outputs))
        .and(ws_clients)
        .and(database)
        .and(with_last_save_time(last_save_time))
        .and_then(ws_handler)
        .with(warp::cors().allow_any_origin());

    // GO! GO! GO!
    println!("Starting server");
    let port = env::var("API_PORT")
        .unwrap_or(8000.to_string())
        .parse::<u16>()
        .unwrap();
    warp::serve(ws_route).run(([0, 0, 0, 0], port)).await;
}

/* WALLET PERSISTENCE */

#[cached]
fn wallet_dir() -> PathBuf {
    // return path to data on filesystem
    // memoized because we don't need to make the syscalls errytim
    let path = PathBuf::from(env::var("CELESTIUM_DATA_DIR").unwrap_or("/data".to_string()));
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
            on_chain_transactions_bin: load("on_chain_transactions")?,
            unspent_outputs_bin: load("unspent_outputs")?,
            nft_lookups_bin: load("nft_lookups")?,
            off_chain_transactions_bin: load("off_chain_transactions")?,
        },
        env::var("RELOAD_UNSPENT_OUTPUTS").is_ok(),
        env::var("IGNORE_OFF_CHAIN_TRANSACTIONS").is_ok(),
    )
}

const MIN_SAVE_INTERVAL_S: i64 = 60 * 5;

async fn save_wallet(
    wallet: &SharedWallet,
    last_save_time: &SharedLastSavedTime,
) -> Result<(), String> {
    // write members of the wallet struct to disk
    // overwrites whatever was in the way

    let mut real_last_save_time = last_save_time.lock().await;
    let now = Utc::now().timestamp();
    let elapsed = now - *real_last_save_time;
    if elapsed > MIN_SAVE_INTERVAL_S {
        *real_last_save_time = now;
    } else {
        println!(
            "Skipping wallet write: it has not been {}s since last write ({}s elapsed)",
            MIN_SAVE_INTERVAL_S, elapsed
        );
        drop(real_last_save_time);
        return Ok(());
    }
    drop(real_last_save_time);

    println!("Writing wallet to disk.");
    let dir = wallet_dir();
    let wallet_bin = wallet.read().await.to_binary()?;
    let save = |filename: &str, data: Vec<u8>| {
        File::create(dir.join(filename))
            .map(|mut f| f.write_all(&data).map_err(|e| e.to_string()))
            .map_err(|e| e.to_string())
    };
    save("blockchain", wallet_bin.blockchain_bin)??;
    save("pk", wallet_bin.pk_bin)??;
    save("sk", wallet_bin.sk_bin)??;
    save(
        "on_chain_transactions",
        wallet_bin.on_chain_transactions_bin,
    )??;
    save("unspent_outputs", wallet_bin.unspent_outputs_bin)??;
    save("nft_lookups", wallet_bin.nft_lookups_bin)??;
    save(
        "off_chain_transactions",
        wallet_bin.off_chain_transactions_bin,
    )??;
    println!(
        "Wallet written to disk, took {}s",
        Utc::now().timestamp() - now
    );
    Ok(())
}

async fn generate_wallet() -> Result<Wallet, String> {
    // make new wallet, write it to disk
    // TODO: this should probably panic if there's a partial wallet in the way
    println!("Generating new wallet.");
    let wallet = Wallet::generate_init_blockchain()?;
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
    save(
        "on_chain_transactions",
        wallet_bin.on_chain_transactions_bin,
    )??;
    save("unspent_outputs", wallet_bin.unspent_outputs_bin)??;
    save("nft_lookups", wallet_bin.nft_lookups_bin)??;
    save(
        "off_chain_transactions",
        wallet_bin.off_chain_transactions_bin,
    )??;
    Ok(wallet)
}

/* WARP STUFF */

fn with_wallet(
    wallet: SharedWallet,
) -> impl Filter<Extract = (SharedWallet,), Error = Infallible> + Clone {
    warp::any().map(move || wallet.clone())
}

fn with_canvas(
    canvas: SharedCanvas,
) -> impl Filter<Extract = (SharedCanvas,), Error = Infallible> + Clone {
    warp::any().map(move || canvas.clone())
}
fn with_floating_outputs(
    floating_outputs: SharedFloatingOutputs,
) -> impl Filter<Extract = (SharedFloatingOutputs,), Error = Infallible> + Clone {
    warp::any().map(move || floating_outputs.clone())
}

fn with_last_save_time(
    last_save_time: SharedLastSavedTime,
) -> impl Filter<Extract = (SharedLastSavedTime,), Error = Infallible> + Clone {
    warp::any().map(move || last_save_time.clone())
}

async fn ws_handler(
    ws: warp::ws::Ws,
    wallet: SharedWallet,
    canvas: SharedCanvas,
    floating_outputs: SharedFloatingOutputs,
    clients: WSClients,
    database: Database,
    last_save_time: SharedLastSavedTime,
) -> Result<impl Reply, Rejection> {
    // weird boilerplate because I don't know why
    // this async function seems to just pass stuff on to another async function
    // but I don't know how to inline it 🤷
    Ok(ws.on_upgrade(move |socket| {
        client_connection(
            socket,
            wallet,
            canvas,
            floating_outputs,
            clients,
            database,
            last_save_time,
        )
    }))
}

macro_rules! ws_error {
    // print the error, send it out over websockets
    // and exit function early
    ($sender: expr, $errmsg: expr) => {
        println!("{}", $errmsg);
        if !$sender.is_closed() {
            $sender.send(Message::text($errmsg.to_string())).unwrap();
        } else {
            println!(
                "WARNING: A connection was closed unexpectedly before being able to send an error"
            );
        }
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
    floating_outputs: SharedFloatingOutputs,
    clients: WSClients,
    database: Database,
    last_save_time: SharedLastSavedTime,
) {
    // keeps a client connection open, pass along incoming messages
    println!("Establishing client connection... {:?}", ws);
    let (mut sender, mut receiver) = ws.split();
    let my_id = NEXT_USER_ID.fetch_add(1, Relaxed);
    let (tx, rx) = mpsc::unbounded_channel();
    let mut rx = UnboundedReceiverStream::new(rx);

    // Save the sender in our list of connected users.

    clients.write().await.insert(my_id, tx);
    println!("Current clients: {}", clients.read().await.len());

    tokio::task::spawn(async move {
        while let Some(message) = rx.next().await {
            sender
                .send(message)
                .unwrap_or_else(|e| {
                    println!("Websocket send error: {}", e);
                })
                .await;
        }
    });

    while let Some(body) = receiver.next().await {
        let message = match body {
            Ok(msg) => msg,
            Err(e) => {
                println!("error reading message on websocket: {}", e);
                break;
            }
        };
        handle_ws_message(
            message,
            my_id,
            &wallet,
            &canvas,
            &floating_outputs,
            &clients,
            &database,
            &last_save_time,
        )
        .await;
    }

    clients.write().await.remove(&my_id);
    println!("Current clients: {}", clients.read().await.len());
}

async fn handle_ws_message(
    message: Message,
    my_id: usize,
    wallet: &SharedWallet,
    canvas: &SharedCanvas,
    floating_outputs: &SharedFloatingOutputs,
    clients: &WSClients,
    database: &Database,
    last_save_time: &SharedLastSavedTime,
) {
    // this is the function that actually receives a message
    // validate it, add it to the blockchain, then exit.
    let sender = clients.read().await.get(&my_id).unwrap().clone();

    if !message.is_binary() {
        ws_error!(sender, "Expected binary WS message.".to_string());
    }

    let binary_message = message.as_bytes();
    match FromPrimitive::from_u8(binary_message[0]) {
        Some(CMDOpcodes::GetEntireImage) => {
            let mut entire_image = canvas.read().await.serialize_colors();

            entire_image.insert(0, CMDOpcodes::EntireImage as u8);
            if !sender.is_closed() {
                if let Err(e) = sender.send(Message::binary(entire_image)) {
                    ws_error!(sender, format!("Error sending canvas: {}", e));
                }
            } else {
                println!("WARNING: A connection was closed unexpectedly before being able to send the canvas");
            }
        }
        Some(CMDOpcodes::MinedTransaction) => {
            parse_transaction(
                &binary_message[1..],
                &sender,
                wallet,
                canvas,
                database,
                floating_outputs,
                clients,
                last_save_time,
            )
            .await
        }
        Some(CMDOpcodes::GetPixelData) => {
            parse_get_pixel_data(
                &binary_message[1..],
                &sender,
                wallet,
                canvas,
                floating_outputs,
            )
            .await
        }
        Some(CMDOpcodes::BuyStoreItem) => {
            parse_buy_store_item(
                &binary_message[1..],
                &sender,
                wallet,
                database,
                last_save_time,
            )
            .await
        }
        Some(CMDOpcodes::GetStoreItem) => {
            parse_get_store_item(&binary_message[1..], &sender, database).await
        }
        Some(CMDOpcodes::GetUserData) => {
            parse_get_user_data(&binary_message[1..], &sender, wallet, database).await
        }
        _ => {
            ws_error!(
                sender,
                format!("Unexpeted CMD Opcode {}", binary_message[0])
            );
        }
    }
}

async fn get_or_create_nft(
    id_hash: [u8; HASH_SIZE],
    their_pk: PublicKey,
    wallet: &SharedWallet,
    padded_message: [u8; 33],
    last_save_time: &SharedLastSavedTime,
) -> Result<(BlockHash, TransactionHash, TransactionVarUint), String> {
    let nft = wallet.read().await.lookup_nft(id_hash);
    let head_hash = wallet.read().await.get_head_hash();
    let str_message = std::str::from_utf8(
        &padded_message[..padded_message.iter().position(|arr| arr == &0u8).unwrap()],
    )
    .unwrap();
    match nft {
        Some(n) => Ok(n),
        None => {
            println!(
                "{} trying to buy \"{}\", which does not yet exist among the known already mined IDs, creating it",
                their_pk, str_message,
            );
            let t = Transaction::new_id_base_transaction(
                head_hash,
                padded_message,
                TransactionOutput::new(TransactionValue::new_id_transfer(id_hash)?, their_pk),
            )?;
            // We have to drop the wallet lock while mining,
            // so other clients can still use the server

            println!("Starting mining NFT for \"{}\"...", str_message);
            let start = Instant::now();
            let t = Wallet::mine_transaction(
                DEFAULT_N_THREADS,
                DEFAULT_PAR_WORK,
                t,
                &ThreadPoolBuilder::new()
                    .num_threads(DEFAULT_N_THREADS as usize)
                    .build()
                    .unwrap(),
            )?;
            println!(
                "Mining of NFT {} done, took {:?}",
                str_message,
                start.elapsed()
            );

            // Mining done and we can retake the wallet lock
            wallet.write().await.add_off_chain_transaction(&t)?;

            let n = wallet.read().await.lookup_nft(id_hash).unwrap();
            save_wallet(&wallet, &last_save_time).await?;
            Ok(n)
        }
    }
}

async fn parse_buy_store_item(
    bin_parameters: &[u8],
    sender: &mpsc::UnboundedSender<Message>,
    wallet: &SharedWallet,
    database: &Database,
    last_save_time: &SharedLastSavedTime,
) {
    let mut pk = [0u8; PUBLIC_KEY_COMPRESSED_SIZE];
    pk.copy_from_slice(&bin_parameters[..PUBLIC_KEY_COMPRESSED_SIZE]);
    let their_pk = *unwrap_or_ws_error!(sender, PublicKey::from_serialized(&pk, &mut 0));

    let item_name = match std::str::from_utf8(&bin_parameters[PUBLIC_KEY_COMPRESSED_SIZE..]) {
        Ok(i) => i,
        Err(e) => {
            ws_error!(sender, format!("Could not parse object name: {}", e));
        }
    };

    let store_collection_name: String = env::var("MONGODB_STORE_COLLECTION_NAME")
        .unwrap_or(DEFAULT_MONGODB_STORE_COLLECTION_NAME.to_string());
    let store_collection = database.collection::<StoreItem>(&store_collection_name);
    let debris_collection_name: String = env::var("MONGODB_DEBRIS_COLLECTION_NAME")
        .unwrap_or(DEFAULT_MONGODB_DEBRIS_COLLECTION_NAME.to_string());
    let debris_collection = database.collection::<DebrisItem>(&debris_collection_name);

    let item = match store_collection.find_one(doc! {"full_name": item_name.to_string()}, None) {
        Ok(Some(c)) => c,
        Err(e) => {
            ws_error!(
                sender,
                format!(
                    "Could not find Store Item with full_name \"{}\": {}",
                    item_name, e
                )
            );
        }
        _ => {
            ws_error!(
                sender,
                format!("Could not find Store Item with full_name \"{}\"", item_name)
            );
        }
    };

    let intldes = item.debris_intldes.as_str();
    let debris = match debris_collection.find_one(doc! {"_id": intldes}, None) {
        Ok(Some(c)) => c,
        Err(e) => {
            ws_error!(
                sender,
                format!(
                    "Could not find Debris Item with INTLDES \"{}\": {}",
                    intldes, e
                )
            );
        }
        _ => {
            ws_error!(
                sender,
                format!("Could not find Debris Item with INTLDES \"{}\"", intldes)
            );
        }
    };

    let store_value_in_dust = unwrap_or_ws_error!(sender, item.store_value_in_dust.parse::<u128>());
    let path = format!(
        "../celestium-frontend/public/videos-full/{}.mp4",
        item.full_name
    );

    let mut file = unwrap_or_ws_error!(sender, File::open(path));
    let mut video_bytes = vec![];
    unwrap_or_ws_error!(sender, file.read_to_end(&mut video_bytes));
    let full_name_bytes = item.full_name.as_bytes();
    let id_digest = &[full_name_bytes, &[0u8], video_bytes.as_ref()].concat();
    let store_item_id_hash = *Sha3_256::digest(id_digest).as_ref();
    let our_pk = unwrap_or_ws_error!(sender, wallet.read().await.get_pk());

    let mut padded_message = [0u8; transaction::BASE_TRANSACTION_MESSAGE_LEN];
    padded_message[0..full_name_bytes.len()].copy_from_slice(full_name_bytes);
    let (store_item_nft_block_hash, store_item_nft_transaction_hash, store_item_nft_index) = unwrap_or_ws_error!(
        sender,
        get_or_create_nft(
            store_item_id_hash,
            their_pk,
            wallet,
            padded_message,
            last_save_time,
        )
        .await
    );
    store_collection.update_one(
        doc! {"_id": item._id},
        doc! {"$set": {"id_hash": hex::encode(store_item_id_hash)}},
        None,
    );

    match wallet.read().await.unspent_outputs.get(&our_pk) {
        Some(our_unspent_outputs) => {
            if our_unspent_outputs
                .get(&(
                    store_item_nft_block_hash.clone(),
                    store_item_nft_transaction_hash.clone(),
                    store_item_nft_index.clone(),
                ))
                .is_none()
            {
                ws_error!(sender, "NFT not owned by us");
            };
        }
        None => {
            ws_error!(sender, "NFT not owned by us");
        }
    }

    let intldes_bytes = debris.INTLDES.as_bytes();
    let intldes_hash = *Sha3_256::digest(intldes_bytes).as_ref();
    let mut padded_message = [0u8; transaction::BASE_TRANSACTION_MESSAGE_LEN];
    padded_message[..intldes_bytes.len()].copy_from_slice(intldes_bytes);
    let (debris_nft_block_hash, debris_nft_transaction_hash, debris_nft_index) = unwrap_or_ws_error!(
        sender,
        get_or_create_nft(
            intldes_hash,
            their_pk,
            wallet,
            padded_message,
            last_save_time
        )
        .await
    );
    debris_collection.update_one(
        doc! {"_id": debris._id},
        doc! {"$set": {"id_hash": hex::encode(intldes_hash)}},
        None,
    );

    match wallet.read().await.unspent_outputs.get(&our_pk) {
        Some(our_unspent_outputs) => {
            if our_unspent_outputs
                .get(&(
                    debris_nft_block_hash.clone(),
                    debris_nft_transaction_hash.clone(),
                    debris_nft_index.clone(),
                ))
                .is_none()
            {
                ws_error!(sender, "Debris NFT not owned by us");
            };
        }
        None => {
            ws_error!(sender, "Debris NFT not owned by us");
        }
    }

    // Create the transaction to Store Item and Debris to client and value to us
    let transaction = {
        let value = unwrap_or_ws_error!(
            sender,
            TransactionValue::new_coin_transfer(store_value_in_dust, 0)
        );

        let (dust, mut inputs) = unwrap_or_ws_error!(
            sender,
            wallet
                .read()
                .await
                .collect_for_coin_transfer(&value, their_pk, HashSet::new())
        );

        if dust < store_value_in_dust {
            ws_error!(
                sender,
                format!(
                    "[0x{}] does not own enough dust to spend, found {} needed {}",
                    hex::encode(their_pk.serialize()),
                    dust,
                    store_value_in_dust,
                )
            );
        }

        let mut outputs = vec![TransactionOutput::new(
            unwrap_or_ws_error!(
                sender,
                TransactionValue::new_coin_transfer(
                    unwrap_or_ws_error!(sender, item.store_value_in_dust.parse::<u128>()),
                    0
                )
            ),
            our_pk,
        )];

        let change = dust - store_value_in_dust;
        if change > 0 {
            outputs.push(TransactionOutput::new(
                unwrap_or_ws_error!(sender, TransactionValue::new_coin_transfer(change, 0)),
                their_pk,
            ));
        }

        inputs.push(TransactionInput::new(
            store_item_nft_block_hash,
            store_item_nft_transaction_hash,
            store_item_nft_index,
        ));
        inputs.push(TransactionInput::new(
            debris_nft_block_hash,
            debris_nft_transaction_hash,
            debris_nft_index,
        ));

        outputs.push(TransactionOutput::new(
            unwrap_or_ws_error!(
                sender,
                TransactionValue::new_id_transfer(store_item_id_hash)
            ),
            their_pk,
        ));
        outputs.push(TransactionOutput::new(
            unwrap_or_ws_error!(sender, TransactionValue::new_id_transfer(intldes_hash)),
            their_pk,
        ));

        let mut transaction = unwrap_or_ws_error!(sender, Transaction::new(inputs, outputs));

        unwrap_or_ws_error!(
            sender,
            transaction.sign(
                wallet.read().await.get_sk().unwrap(),
                transaction.count_inputs() - 2,
            )
        );
        unwrap_or_ws_error!(
            sender,
            transaction.sign(
                wallet.read().await.get_sk().unwrap(),
                transaction.count_inputs() - 1,
            )
        );

        transaction
    };

    let mut response_data = vec![0u8; transaction.serialized_len() + 1];
    response_data[0] = CMDOpcodes::UnminedTransaction as u8;

    unwrap_or_ws_error!(
        sender,
        transaction.serialize_into(&mut response_data, &mut 1)
    );
    if !sender.is_closed() {
        if let Err(e) = sender.send(Message::binary(response_data)) {
            println!("Could not send buy store item transactions: {}", e);
        }
    } else {
        println!(
            "WARNING: A connection was closed unexpectedly before being able to send store item"
        );
    }
}

async fn parse_get_store_item(
    bin_parameters: &[u8],
    sender: &mpsc::UnboundedSender<Message>,
    database: &Database,
) {
    let mut rng = rand::thread_rng();
    let img_nr = rng.gen_range(0..3);
    // let filename = format!("./images/{}.jpg", img_nr);
    // let mut f = File::open(&filename).expect("no file found");
    // let metadata = fs::metadata(&filename).expect("unable to read metadata");
    let image_url = format!("/images/{}.gif", img_nr);

    let item_name = match std::str::from_utf8(&bin_parameters) {
        Ok(i) => i,
        Err(e) => {
            ws_error!(sender, format!("Could not parse object name: {}", e));
        }
    };

    let store_collection_name: String = env::var("MONGODB_STORE_COLLECTION_NAME")
        .unwrap_or(DEFAULT_MONGODB_STORE_COLLECTION_NAME.to_string());
    let store_collection = database.collection::<StoreItem>(&store_collection_name);
    let item = match store_collection.find_one(doc! {"full_name": item_name.to_string()}, None) {
        Ok(Some(c)) => c,
        Err(e) => {
            ws_error!(
                sender,
                format!("Could not find Store Item with ID \"{}\": {}", item_name, e)
            );
        }
        _ => {
            ws_error!(
                sender,
                format!("Could not find Store Item with ID \"{}\"", item_name)
            );
        }
    };
    let get_store_item_data = GetStoreItemData {
        image_url,
        store_value_in_dust: item.store_value_in_dust,
    };

    let json = unwrap_or_ws_error!(sender, serde_json::to_string(&get_store_item_data));
    let bin_json = json.as_bytes();
    let mut buffer = vec![0; (bin_json.len() + 1) as usize];
    buffer[0] = CMDOpcodes::StoreItem as u8;
    buffer[1..].copy_from_slice(bin_json);

    if !sender.is_closed() {
        if let Err(e) = sender.send(Message::binary(buffer)) {
            println!("Could not send store item: {}", e);
        }
    } else {
        println!(
            "WARNING: A connection was closed unexpectedly before being able to send store item"
        );
    }
}

async fn parse_get_user_data(
    bin_parameters: &[u8],
    sender: &mpsc::UnboundedSender<Message>,
    wallet: &SharedWallet,
    database: &Database,
) {
    let mut pk = [0u8; PUBLIC_KEY_COMPRESSED_SIZE];
    pk.copy_from_slice(&bin_parameters[..PUBLIC_KEY_COMPRESSED_SIZE]);
    let their_pk = *unwrap_or_ws_error!(sender, PublicKey::from_serialized(&pk, &mut 0));

    println!(
        "Got user data request for: [0x{}]",
        hex::encode(their_pk.serialize())
    );

    let (balance, owned_ids) =
        unwrap_or_ws_error!(sender, wallet.read().await.get_balance(their_pk));

    println!("Balance: {} | Owned ids: {}", balance, owned_ids.len());

    let store_collection_name: String = env::var("MONGODB_STORE_COLLECTION_NAME")
        .unwrap_or(DEFAULT_MONGODB_STORE_COLLECTION_NAME.to_string());
    let store_collection = database.collection::<StoreItem>(&store_collection_name);
    let debris_collection_name: String = env::var("MONGODB_DEBRIS_COLLECTION_NAME")
        .unwrap_or(DEFAULT_MONGODB_DEBRIS_COLLECTION_NAME.to_string());
    let debris_collection = database.collection::<DebrisItem>(&debris_collection_name);

    let owned_ids: Vec<String> = owned_ids
        .iter()
        .map(|id| hex::encode(id.get_id().unwrap()))
        .collect();

    let user_data = UserData {
        balance: balance.to_string(),
        owned_store_items: store_collection
            .find(doc! {"id_hash": {"$in": &owned_ids}}, None)
            .into_iter()
            .flatten()
            .flatten()
            .collect(),
        owned_debris: debris_collection
            .find(doc! {"_id": {"$in": &owned_ids}}, None)
            .into_iter()
            .flatten()
            .flatten()
            .collect(),
    };

    let json = unwrap_or_ws_error!(sender, serde_json::to_string(&user_data));
    let bin_json = json.as_bytes();
    let mut buffer = vec![0; (bin_json.len() + 1) as usize];
    buffer[0] = CMDOpcodes::UserData as u8;
    buffer[1..].copy_from_slice(bin_json);

    if !sender.is_closed() {
        if let Err(e) = sender.send(Message::binary(buffer)) {
            println!("Could not send user data: {}", e);
        };
    } else {
        println!(
            "WARNING: A connection was closed unexpectedly before being able to send user data"
        );
    }
}

async fn parse_get_pixel_data(
    bin_parameters: &[u8],
    sender: &mpsc::UnboundedSender<Message>,
    wallet: &SharedWallet,
    canvas: &SharedCanvas,
    floating_outputs: &SharedFloatingOutputs,
) {
    if bin_parameters.len() != 4 + PUBLIC_KEY_COMPRESSED_SIZE {
        ws_error!(
            sender,
            format!(
                "Expected message len of {} for CMD Opcode {:x} (Get pixel hash) got {}",
                5 + PUBLIC_KEY_COMPRESSED_SIZE,
                CMDOpcodes::GetPixelData as u8,
                bin_parameters.len()
            )
        );
    }

    // Parse client parameters
    let mut i = 0;
    let x: usize = ((bin_parameters[i] as usize) << 8) + (bin_parameters[i + 1] as usize);
    i += 2;
    let y: usize = ((bin_parameters[i] as usize) << 8) + (bin_parameters[i + 1] as usize);
    i += 2;
    let mut their_pk = [0u8; PUBLIC_KEY_COMPRESSED_SIZE];
    their_pk.copy_from_slice(&bin_parameters[i..i + PUBLIC_KEY_COMPRESSED_SIZE]);
    let their_pk = *unwrap_or_ws_error!(sender, PublicKey::from_serialized(&their_pk, &mut 0));

    let pixel_hash =
        unwrap_or_ws_error!(sender, canvas.read().await.get_pixel(x, y)).hash(x as u16, y as u16);

    let our_pk = unwrap_or_ws_error!(sender, wallet.read().await.get_pk());
    let our_sk = unwrap_or_ws_error!(sender, wallet.read().await.get_sk());
    let head_hash = wallet.read().await.get_head_hash();

    // Value to transfer to pixel miner
    let value = unwrap_or_ws_error!(sender, TransactionValue::new_coin_transfer(DUST_PER_CEL, 0));
    let (dust, inputs) = {
        // So we do not hold the wallet lock while trying to communicate over ws
        let floating_outputs_clone = floating_outputs.read().await.clone();

        let (dust, inputs) = unwrap_or_ws_error!(
            sender,
            wallet
                .read()
                .await
                .collect_for_coin_transfer(&value, our_pk, floating_outputs_clone,)
        );

        for input in &inputs {
            floating_outputs.write().await.insert((
                input.transaction_hash.clone(),
                input.output_index.get_value(),
            ));
        }
        (dust, inputs)
    };

    // Add output transferring "value" to pixel miner
    let mut outputs = vec![TransactionOutput::new(value, their_pk)];

    let needed_dust = DUST_PER_CEL;
    match dust.cmp(&(needed_dust)) {
        Ordering::Greater => {
            // Output transferring unused dust (if there is unused dust)
            outputs.push(TransactionOutput::new(
                unwrap_or_ws_error!(
                    sender,
                    TransactionValue::new_coin_transfer(dust - needed_dust, 0)
                ),
                our_pk,
            ));
        }
        Ordering::Less => {
            ws_error!(
                sender,
                format!(
                    "Could not find enough dust to pay you, only found {} expected at least {}",
                    dust, needed_dust
                )
            );
        }
        Ordering::Equal => (),
    }

    let mut transaction = unwrap_or_ws_error!(sender, Transaction::new(inputs, outputs));

    for i in 0..transaction.count_inputs() {
        unwrap_or_ws_error!(sender, transaction.sign(our_sk, i));
    }

    // Create response vector
    let mut pixel_hash_response =
        vec![0u8; 1 + PIXEL_HASH_SIZE + HASH_SIZE + transaction.serialized_len()];

    // Create response pixel data
    i = 0;
    pixel_hash_response[i] = CMDOpcodes::PixelData as u8;
    i += 1;

    // Add pixel hash to response
    pixel_hash_response[i..i + PIXEL_HASH_SIZE].copy_from_slice(&pixel_hash);
    i += PIXEL_HASH_SIZE;

    // Add block hash to response
    unwrap_or_ws_error!(
        sender,
        head_hash.serialize_into(&mut pixel_hash_response, &mut i)
    );

    // Add transaction to response
    unwrap_or_ws_error!(
        sender,
        transaction.serialize_into(&mut pixel_hash_response, &mut i)
    );

    if !sender.is_closed() {
        // Send response
        println!("Sending pixel hash response");
        if let Err(e) = sender.send(Message::binary(pixel_hash_response)) {
            println!("Could not send pixel hash: {}", e);
        };
        println!("Sent pixel hash response");
    } else {
        println!(
            "WARNING: A connection was closed unexpectedly before being able to send pixel hash"
        );
    }
}

async fn parse_transaction(
    bin_transaction: &[u8],
    sender: &mpsc::UnboundedSender<Message>,
    wallet: &SharedWallet,
    canvas: &SharedCanvas,
    database: &Database,
    floating_outputs: &SharedFloatingOutputs,
    clients: &WSClients,
    last_save_time: &SharedLastSavedTime,
) {
    let mut i = 0;
    let transaction_count = *unwrap_or_ws_error!(
        sender,
        TransactionVarUint::from_serialized(bin_transaction, &mut i)
    );
    if transaction_count.get_value() == 1 {
        println!("Got 1 transaction");
        let transaction = unwrap_or_ws_error!(
            sender,
            Transaction::from_serialized(bin_transaction, &mut i)
        );

        let store_collection_name: String = env::var("MONGODB_STORE_COLLECTION_NAME")
            .unwrap_or_else(|_| DEFAULT_MONGODB_STORE_COLLECTION_NAME.to_string());
        let store_collection = database.collection::<StoreItem>(&store_collection_name);

        for o in transaction.get_outputs() {
            if let Ok(id_hash) = o.value.get_id() {
                unwrap_or_ws_error!(
                    sender,
                    store_collection.update_one(
                        doc! {"id_hash": hex::encode(id_hash)},
                        doc! {"$set": {"state": "bought"}},
                        None,
                    )
                );
            }
        }
        unwrap_or_ws_error!(
            sender,
            wallet.write().await.add_off_chain_transaction(&transaction)
        );

        for input in transaction.get_inputs() {
            floating_outputs
                .write()
                .await
                .remove(&(input.transaction_hash, input.output_index.get_value()));
        }
    } else if transaction_count.get_value() == 2 {
        println!("Got 2 transactions");
        let pixel_transaction = unwrap_or_ws_error!(
            sender,
            Transaction::from_serialized(bin_transaction, &mut i)
        );

        let value_transaction = unwrap_or_ws_error!(
            sender,
            Transaction::from_serialized(bin_transaction, &mut i)
        );

        unwrap_or_ws_error!(
            sender,
            wallet
                .read()
                .await
                .verify_off_chain_transaction(&pixel_transaction)
        );
        unwrap_or_ws_error!(
            sender,
            wallet
                .read()
                .await
                .verify_off_chain_transaction(&value_transaction)
        );

        let base_transaction_message =
            unwrap_or_ws_error!(sender, pixel_transaction.get_base_transaction_message())
                as [u8; BASE_TRANSACTION_MESSAGE_LEN];

        let (x, y, pixel) = unwrap_or_ws_error!(
            sender,
            canvas::Canvas::parse_pixel(base_transaction_message)
        );

        let current_pixel_hash = unwrap_or_ws_error!(sender, canvas.read().await.get_pixel(x, y))
            .hash(x as u16, y as u16)
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
            unwrap_or_ws_error!(sender, canvas.write().await.set_pixel(x, y, pixel));

            let mut update_pixel_binary_message = [0u8; 6];
            update_pixel_binary_message[0] = CMDOpcodes::UpdatePixel as u8;
            update_pixel_binary_message[1..]
                .copy_from_slice(&base_transaction_message[PIXEL_HASH_SIZE..]);
            for tx in clients.read().await.values() {
                if let Err(e) = tx.send(Message::binary(update_pixel_binary_message)) {
                    println!("Could not send updated pixel: {}", e);
                }
            }
        }

        unwrap_or_ws_error!(
            sender,
            wallet
                .write()
                .await
                .add_off_chain_transaction(&pixel_transaction)
        );
        unwrap_or_ws_error!(
            sender,
            wallet
                .write()
                .await
                .add_off_chain_transaction(&value_transaction)
        );
        for input in value_transaction.get_inputs() {
            floating_outputs
                .write()
                .await
                .remove(&(input.transaction_hash, input.output_index.get_value()));
        }
    } else {
        ws_error!(
            sender,
            format!(
                "Transaction count expected to be 1 or 2 got {}",
                transaction_count
            )
        );
    }

    unwrap_or_ws_error!(sender, save_wallet(&wallet, last_save_time).await);
}
