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
    sync::{Client, Database},
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
use crate::canvas::PIXEL_HASH_SIZE;
use celestium::{
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

////////////////////////////////////////////////// !!! !!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
type SharedCanvas = Arc<Mutex<canvas::Canvas>>; // !!! IMPORTANT! Always lock SharedCanvas before SharedWallet  !!!
type SharedWallet = Arc<Mutex<Box<Wallet>>>; ///// !!! IMPORTANT! to await potential dead locks (if using both) !!!
type SharedFloatingOutputs = Arc<Mutex<Box<HashSet<(TransactionHash, usize)>>>>;
type WSClients = Arc<RwLock<HashMap<usize, mpsc::UnboundedSender<Message>>>>;

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
}

static NEXT_USER_ID: AtomicUsize = AtomicUsize::new(1);
static DUST_PER_CEL: u128 = 10000000000000000000000000000000;
static DEFAULT_MONGODB_STORE_COLLECTION_NAME: &str = "asteroids";

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
            match generate_wallet() {
                Ok(w) => w,
                Err(e) => {
                    panic!("Failed generating wallet: {}", e)
                }
            }
        }
    };
    // for ((block_hash, transaction_hash, index), transaction_output) in wallet.unspent_outputs.iter()
    // {
    //     wallet.off_chain_transactions[]
    //     // if transaction_output.value.is
    // }
    let shared_wallet = Arc::new(Mutex::new(Box::new(wallet)));

    // initialize empty canvas
    print!("Initializing canvas...");
    let shared_canvas = canvas::Canvas::new_test();
    let real_wallet = shared_wallet.lock().await;
    for (hash, transaction) in real_wallet.off_chain_transactions.clone() {
        if transaction.is_id_base_transaction() {
            let (x, y, transaction_pixel) =
                canvas::Canvas::parse_pixel(transaction.get_base_transaction_message().unwrap())
                    .unwrap();
            let canvas_pixel = shared_canvas.get_pixel(x, y);
            //if canvas_pixel.hash
        }
    }
    drop(real_wallet);
    let shared_canvas = Arc::new(Mutex::new(shared_canvas));
    println!(" Done!");

    // Keep track of all connected users, key is usize, value
    // is a websocket sender.
    let ws_clients = WSClients::default();
    // Turn our "state" into a new Filter...
    let ws_clients = warp::any().map(move || ws_clients.clone());

    let floating_outputs = Arc::new(Mutex::new(Box::new(HashSet::new())));

    // configure ws route
    let ws_route = warp::path::end()
        .and(warp::ws())
        .and(with_wallet(shared_wallet))
        .and(with_canvas(shared_canvas))
        .and(with_floating_outputs(floating_outputs))
        .and(ws_clients)
        .and(database)
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
    Ok(())
}

fn generate_wallet() -> Result<Wallet, String> {
    // make new wallet, write it to disk
    // TODO: this should probably panic if there's a partial wallet in the way
    println!("Generating new wallet.");
    let wallet = Wallet::generate_init_blockchain()?;
    save_wallet(&wallet)?;
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

async fn ws_handler(
    ws: warp::ws::Ws,
    wallet: SharedWallet,
    canvas: SharedCanvas,
    floating_outputs: SharedFloatingOutputs,
    clients: WSClients,
    database: Database,
) -> Result<impl Reply, Rejection> {
    // weird boilerplate because I don't know why
    // this async function seems to just pass stuff on to another async function
    // but I don't know how to inline it 🤷
    println!("ws_handler");
    Ok(ws.on_upgrade(move |socket| {
        client_connection(socket, wallet, canvas, floating_outputs, clients, database)
    }))
}

macro_rules! ws_error {
    // print the error, send it out over websockets
    // and exit function early
    ($sender: expr, $errmsg: expr) => {
        println!("{}", $errmsg);
        $sender.send(Message::text($errmsg.to_string())).unwrap();
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
) {
    // keeps a client connection open, pass along incoming messages
    println!("Establishing client connection... {:?}", ws);
    let (mut sender, mut receiver) = ws.split();
    println!("Test0");
    let my_id = NEXT_USER_ID.fetch_add(1, Relaxed);
    println!("Test1");
    let (tx, rx) = mpsc::unbounded_channel();
    println!("Test2");
    let mut rx = UnboundedReceiverStream::new(rx);
    println!("Test3");
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
        )
        .await;
    }

    clients.write().await.remove(&my_id);
}

async fn handle_ws_message(
    message: Message,
    my_id: usize,
    wallet: &SharedWallet,
    canvas: &SharedCanvas,
    floating_outputs: &SharedFloatingOutputs,
    clients: &WSClients,
    database: &Database,
) {
    // this is the function that actually receives a message
    // validate it, add it to the blockchain, then exit.

    let real_clients = clients.read().await;
    let sender = real_clients.get(&my_id).unwrap();

    if !message.is_binary() {
        ws_error!(sender, "Expected binary WS message.".to_string());
    }

    println!(
        "Got binary message: {:?}",
        CMDOpcodes::from_u8(message.as_bytes()[0])
    );

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
            parse_transaction(
                &binary_message[1..],
                sender,
                wallet,
                canvas,
                floating_outputs,
                clients,
                database,
            )
            .await
        }
        Some(CMDOpcodes::GetPixelData) => {
            parse_get_pixel_data(
                &binary_message[1..],
                sender,
                wallet,
                canvas,
                floating_outputs,
            )
            .await
        }
        Some(CMDOpcodes::BuyStoreItem) => {
            parse_buy_store_item(&binary_message[1..], sender, wallet, database).await
        }
        Some(CMDOpcodes::GetStoreItem) => {
            parse_get_store_item(&binary_message[1..], sender, wallet, database).await
        }
        Some(CMDOpcodes::GetUserData) => {
            parse_get_user_data(&binary_message[1..], sender, wallet, database).await
        }
        _ => {
            ws_error!(
                sender,
                format!("Unexpeted CMD Opcode {}", binary_message[0])
            );
        }
    }
}

async fn parse_buy_store_item(
    bin_parameters: &[u8],
    sender: &mpsc::UnboundedSender<Message>,
    wallet: &SharedWallet,
    database: &Database,
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

    let store_value_in_dust = unwrap_or_ws_error!(sender, item.store_value_in_dust.parse::<u128>());

    let full_name_bytes = item.full_name.as_bytes();
    let id_hash = *Sha3_256::digest(
        &[
            full_name_bytes,
            unwrap_or_ws_error!(
                sender,
                unwrap_or_ws_error!(
                    sender,
                    reqwest::get(format!(
                        "https://celestium.space/videos/{}.mp4",
                        item.full_name
                    ))
                    .await
                )
                .bytes()
                .await
            )
            .as_ref(),
        ]
        .concat(),
    )
    .as_ref();

    // First, get ID base transaction output,
    // since this may not have been created yet
    // we will release wallet lock during mining
    let id_base_output = {
        let real_wallet = wallet.lock().await;
        match real_wallet.lookup_nft(id_hash) {
            Some(n) => {
                drop(real_wallet);
                n
            }
            None => {
                println!(
                    "{} trying to buy \"{}\", which does not yet exist among the known already mined {} IDs, creating it",
                    their_pk, item.full_name, real_wallet.count_nft_lookup(),
                );
                let mut padded_message = [0u8; transaction::BASE_TRANSACTION_MESSAGE_LEN];
                padded_message[0..full_name_bytes.len()].copy_from_slice(full_name_bytes);
                let t = unwrap_or_ws_error!(
                    sender,
                    Transaction::new_id_base_transaction(
                        real_wallet.get_head_hash(),
                        padded_message,
                        TransactionOutput::new(
                            unwrap_or_ws_error!(sender, TransactionValue::new_id_transfer(id_hash)),
                            unwrap_or_ws_error!(sender, real_wallet.get_pk())
                        ),
                    )
                );
                drop(real_wallet);
                // We have to drop the wallet lock while mining,
                // so other clients can still use the server

                println!("Starting mining NFT for \"{}\"...", item.full_name);
                let start = Instant::now();
                let t = unwrap_or_ws_error!(
                    sender,
                    Wallet::mine_transaction(
                        DEFAULT_N_THREADS,
                        DEFAULT_PAR_WORK,
                        t,
                        unwrap_or_ws_error!(
                            sender,
                            &ThreadPoolBuilder::new()
                                .num_threads(DEFAULT_N_THREADS as usize)
                                .build()
                        ),
                    )
                );
                println!(
                    "Mining of NFT {} done, took {:?}",
                    item.full_name,
                    start.elapsed()
                );

                // Mining done and we can retake the wallet lock
                let mut real_wallet = wallet.lock().await;
                unwrap_or_ws_error!(sender, real_wallet.add_off_chain_transaction(t));

                unwrap_or_ws_error!(sender, save_wallet(&real_wallet));
                unwrap_or_ws_error!(
                    sender,
                    store_collection.update_one(
                        doc! {"_id": item._id},
                        doc! {"$set": {"id_hash": hex::encode(id_hash), "state": "bought"}},
                        None,
                    )
                );

                let n = real_wallet.lookup_nft(id_hash).unwrap();
                drop(real_wallet);
                n
            }
        }
    };

    // Create the transaction to transfer ID to client and value to us
    let transaction = {
        let real_wallet = wallet.lock().await;
        let (dust, mut inputs) = unwrap_or_ws_error!(
            sender,
            real_wallet.collect_for_coin_transfer(
                &unwrap_or_ws_error!(
                    sender,
                    TransactionValue::new_coin_transfer(store_value_in_dust, 0)
                ),
                their_pk,
                HashSet::new()
            )
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
            unwrap_or_ws_error!(sender, real_wallet.get_pk()),
        )];
        let change = dust - store_value_in_dust;
        if change > 0 {
            outputs.push(TransactionOutput::new(
                unwrap_or_ws_error!(sender, TransactionValue::new_coin_transfer(change, 0)),
                their_pk,
            ));
        }

        inputs.push(TransactionInput::new(
            id_base_output.0,
            id_base_output.1,
            id_base_output.2,
        ));

        outputs.push(TransactionOutput::new(
            unwrap_or_ws_error!(sender, TransactionValue::new_id_transfer(id_hash)),
            their_pk,
        ));

        let mut transaction = unwrap_or_ws_error!(sender, Transaction::new(inputs, outputs));

        unwrap_or_ws_error!(
            sender,
            transaction.sign(
                real_wallet.get_sk().unwrap(),
                transaction.count_inputs() - 1
            )
        );

        drop(real_wallet);

        transaction
    };

    let mut response_data = vec![0u8; transaction.serialized_len() + 1];
    response_data[0] = CMDOpcodes::UnminedTransaction as u8;

    unwrap_or_ws_error!(
        sender,
        transaction.serialize_into(&mut response_data, &mut 1)
    );

    if let Err(e) = sender.send(Message::binary(response_data)) {
        println!("Could not send store item: {}", e);
    }
}

async fn parse_get_store_item(
    bin_parameters: &[u8],
    sender: &mpsc::UnboundedSender<Message>,
    wallet: &SharedWallet,
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

    if let Err(e) = sender.send(Message::binary(buffer)) {
        println!("Could not send store item: {}", e);
    }
    // // check length of params
    // if bin_parameters.len() != 16 {
    //     ws_error!(
    //         sender,
    //         format!(
    //             "Expected message len of 16 for CMD opcode {:x} (GetStoreItems)",
    //             CMDOpcodes::GetStoreItems as u8,
    //         )
    //     );
    // }

    // // parse params
    // let from: u32 = ((bin_parameters[0] as u32) << 24) + ((bin_parameters[1] as u32) << 16) + ((bin_parameters[2] as u32) << 8) + ((bin_parameters[3] as u32));
    // let to: u32 = ((bin_parameters[4] as u32) << 24) + ((bin_parameters[5] as u32) << 16) + ((bin_parameters[6] as u32) << 8) + ((bin_parameters[7] as u32));

    // // dance with mongo
    // let store_collection = database.collection::<StoreItem>("store");
    // let filter = doc! {"range": [from, to]};
    // let mongo_result = store_collection.find(filter, None).await;

    // // parse mongo result
    // let mut cursor = unwrap_or_ws_error!(
    //     sender,
    //     mongo_result.map_err(|_| "Failed querying mongodb for asteroids.")
    // );

    // // parse to json
    // let mut json_results: Vec<String> = vec![];
    // while let Some(Ok(asteroid)) = cursor.next().await {
    //   match serde_json::to_string(&asteroid) {
    //       Ok(a) => json_results.push(a),
    //       Err(_) => { ws_error!(sender, "Failed parsing an asteroid from Mongo."); }
    //   }
    // }

    // // put a bow on it
    // let json_array: String = "[".to_string() + &json_results.join(",") + "]";
    // let response_bytes: &[u8] = &[&[CMDOpcodes::StoreItems as u8], json_array.as_bytes()].concat();
    // if let Err(e) = sender.send(Message::binary(response_bytes)) {
    //     println!("Could not send StoreItems: {}", e);
    // };
}

async fn parse_get_user_data(
    bin_parameters: &[u8],
    sender: &mpsc::UnboundedSender<Message>,
    wallet: &SharedWallet,
    database: &Database,
) {
    let mut pk = [0u8; PUBLIC_KEY_COMPRESSED_SIZE];
    pk.copy_from_slice(&bin_parameters[..PUBLIC_KEY_COMPRESSED_SIZE]);
    let pk = *unwrap_or_ws_error!(sender, PublicKey::from_serialized(&pk, &mut 0));

    println!("Got user data request for: [0x{}]", pk);

    let user_data = {
        let real_wallet = wallet.lock().await;

        let (balance, owned_ids) = unwrap_or_ws_error!(sender, real_wallet.get_balance(pk));

        let mut user_data = UserData {
            balance: balance.to_string(),
            owned_store_items: Vec::new(),
        };

        let store_collection_name: String = env::var("MONGODB_STORE_COLLECTION_NAME")
            .unwrap_or(DEFAULT_MONGODB_STORE_COLLECTION_NAME.to_string());
        let store_collection = database.collection::<StoreItem>(&store_collection_name);

        for owned_id in owned_ids {
            if let Ok(Some(item)) = store_collection.find_one(
                doc! {"id_hash": hex::encode(unwrap_or_ws_error!(sender, owned_id.get_id()))},
                None,
            ) {
                user_data.owned_store_items.push(item);
            }
        }

        drop(real_wallet);
        user_data
    };

    let json = unwrap_or_ws_error!(sender, serde_json::to_string(&user_data));
    let bin_json = json.as_bytes();
    let mut buffer = vec![0; (bin_json.len() + 1) as usize];
    buffer[0] = CMDOpcodes::UserData as u8;
    buffer[1..].copy_from_slice(bin_json);

    if let Err(e) = sender.send(Message::binary(buffer)) {
        println!("Could not send user data: {}", e);
    };
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

    let pixel_hash = {
        let real_canvas = canvas.lock().await;
        let pixel = unwrap_or_ws_error!(sender, real_canvas.get_pixel(x, y));
        drop(real_canvas);
        pixel.hash(x as u16, y as u16)
    };
    {
        let real_wallet = wallet.lock().await;

        let our_pk = unwrap_or_ws_error!(sender, real_wallet.get_pk());

        // Value to transfer to pixel miner

        let value =
            unwrap_or_ws_error!(sender, TransactionValue::new_coin_transfer(DUST_PER_CEL, 0));
        let (dust, inputs) = {
            let mut real_floating_outputs = floating_outputs.lock().await;
            let (dust, inputs) = unwrap_or_ws_error!(
                sender,
                real_wallet.collect_for_coin_transfer(
                    &value,
                    our_pk,
                    *real_floating_outputs.clone()
                )
            );
            for input in &inputs {
                real_floating_outputs.insert((
                    input.transaction_hash.clone(),
                    input.output_index.get_value(),
                ));
            }
            drop(real_floating_outputs);
            (dust, inputs)
        };

        // Add output transferring "value" to pixel miner
        let mut outputs = vec![TransactionOutput::new(value, their_pk)];

        let needed_dust = DUST_PER_CEL;
        match dust.cmp(&(needed_dust)) {
            Ordering::Greater => {
                // Output transferring unused dust (if there is unused dust)
                println!(
                    "WARNING: Got more dust than needed ({} > {}), this shouldn't happen too much (ideally only once)",
                    dust,
                    needed_dust
                );
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

        let sk = unwrap_or_ws_error!(sender, real_wallet.get_sk());
        for i in 0..transaction.count_inputs() {
            unwrap_or_ws_error!(sender, transaction.sign(sk, i));
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
            &real_wallet
                .get_head_hash()
                .serialize_into(&mut pixel_hash_response, &mut i)
        );

        // Add transaction to response
        unwrap_or_ws_error!(
            sender,
            transaction.serialize_into(&mut pixel_hash_response, &mut i)
        );

        // Send response
        if let Err(e) = sender.send(Message::binary(pixel_hash_response)) {
            println!("Could not send pixel hash: {}", e);
        };

        drop(real_wallet);
    }
}

async fn parse_transaction(
    bin_transaction: &[u8],
    sender: &mpsc::UnboundedSender<Message>,
    wallet: &SharedWallet,
    canvas: &SharedCanvas,
    floating_outputs: &SharedFloatingOutputs,
    clients: &WSClients,
    database: &Database,
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
        let mut real_wallet = wallet.lock().await;
        let our_pk = unwrap_or_ws_error!(sender, real_wallet.get_pk());
        let (pre_balance, owned_ids) = unwrap_or_ws_error!(sender, real_wallet.get_balance(our_pk));
        if let Err(e) = real_wallet.add_off_chain_transaction(*transaction.clone()) {
            ws_error!(sender, e);
        }
        let (post_balance, owned_ids) =
            unwrap_or_ws_error!(sender, real_wallet.get_balance(our_pk));

        if let Err(e) = save_wallet(&real_wallet) {
            ws_error!(sender, e);
        }
        drop(real_wallet);

        let mut real_floating_outputs = floating_outputs.lock().await;
        for input in transaction.get_inputs() {
            real_floating_outputs.remove(&(input.transaction_hash, input.output_index.get_value()));
        }
        drop(real_floating_outputs);
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

        let mut real_wallet = wallet.lock().await;
        let pk = unwrap_or_ws_error!(sender, real_wallet.get_pk());
        if let Err(e) = real_wallet.add_off_chain_transaction(*pixel_transaction.clone()) {
            ws_error!(sender, e);
        }
        if let Err(e) = real_wallet.add_off_chain_transaction(*value_transaction.clone()) {
            ws_error!(sender, e);
        }

        if let Err(e) = save_wallet(&real_wallet) {
            ws_error!(sender, e);
        }
        drop(real_wallet);

        let mut real_floating_outputs = floating_outputs.lock().await;
        for input in value_transaction.get_inputs() {
            real_floating_outputs.remove(&(input.transaction_hash, input.output_index.get_value()));
        }
        drop(real_floating_outputs);

        let base_transaction_message =
            unwrap_or_ws_error!(sender, pixel_transaction.get_base_transaction_message())
                as [u8; BASE_TRANSACTION_MESSAGE_LEN];

        let mut real_canvas = canvas.lock().await;
        let (x, y, pixel) = unwrap_or_ws_error!(
            sender,
            canvas::Canvas::parse_pixel(base_transaction_message)
        );
        let current_pixel_hash = unwrap_or_ws_error!(sender, real_canvas.get_pixel(x, y))
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
            unwrap_or_ws_error!(sender, real_canvas.set_pixel(x, y, pixel));
            let canvas_collection = database.collection::<StoreItem>("canvas");

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
        drop(real_canvas);
    } else {
        ws_error!(
            sender,
            format!(
                "Transaction count expected to be 1 or 2 got {}",
                transaction_count
            )
        );
    }
}
