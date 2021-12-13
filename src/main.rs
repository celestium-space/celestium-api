use std::{
    collections::HashMap,
    convert::Infallible,
    env,
    fs::File,
    fs::{self, read},
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
use mongodb::{bson::doc, options::ClientOptions, Client, Database};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use rand::Rng;
use secp256k1::PublicKey;
use sha3::{Digest, Sha3_256};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::{filters::ws::Message, Filter, Rejection, Reply};

// here
use crate::canvas::PIXEL_HASH_SIZE;
use celestium::{
    ec_key_serialization::PUBLIC_KEY_COMPRESSED_SIZE,
    merkle_forest::HASH_SIZE,
    serialize::Serialize,
    transaction::{Transaction, BASE_TRANSACTION_MESSAGE_LEN},
    transaction_output::TransactionOutput,
    transaction_value::TransactionValue,
    wallet::{BinaryWallet, Wallet},
};

mod canvas;

////////////////////////////////////////////////// !!! !!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
type SharedCanvas = Arc<Mutex<canvas::Canvas>>; // !!! IMPORTANT! Always lock SharedCanvas before SharedWallet  !!!
type SharedWallet = Arc<Mutex<Box<Wallet>>>; ///// !!! IMPORTANT! to await potential dead locks (if using both) !!!
type WSClients = Arc<RwLock<HashMap<usize, mpsc::UnboundedSender<Message>>>>;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StoreItem {
    _id: String,
    object_type: String,
    semi_major_axis_in_au: f64,
    eccentricity: f64,
    value_in_usd: f64,
    est_profit_in_usd: f64,
    delta_v_in_km_s: f64,
    minimum_orbit_intersection_distance_in_au: f64,
    group: String,
    store_price_in_dust: usize,
}

impl StoreItem {
    fn new(
        name: String,
        object_type: String,
        semi_major_axis_in_au: f64,
        eccentricity: f64,
        value_in_usd: f64,
        est_profit_in_usd: f64,
        delta_v_in_km_s: f64,
        minimum_orbit_intersection_distance_in_au: f64,
        group: String,
        store_price_in_dust: usize,
    ) -> StoreItem {
        StoreItem {
            _id: name,
            object_type,
            semi_major_axis_in_au,
            eccentricity,
            value_in_usd,
            est_profit_in_usd,
            delta_v_in_km_s,
            minimum_orbit_intersection_distance_in_au,
            group,
            store_price_in_dust,
        }
    }

    fn hash(&self) -> [u8; HASH_SIZE] {
        let mut hash = [0u8; HASH_SIZE];
        let mut self_serialized = Vec::new();
        self_serialized.append(&mut self._id.as_bytes().to_vec());
        self_serialized.append(&mut self.semi_major_axis_in_au.to_ne_bytes().to_vec());
        self_serialized.append(&mut self.eccentricity.to_ne_bytes().to_vec());
        self_serialized.append(&mut self.value_in_usd.to_ne_bytes().to_vec());
        self_serialized.append(&mut self.est_profit_in_usd.to_ne_bytes().to_vec());
        self_serialized.append(&mut self.delta_v_in_km_s.to_ne_bytes().to_vec());
        self_serialized.append(
            &mut self
                .minimum_orbit_intersection_distance_in_au
                .to_ne_bytes()
                .to_vec(),
        );
        self_serialized.append(&mut self.group.as_bytes().to_vec());
        hash.copy_from_slice(Sha3_256::digest(&self_serialized).as_slice());
        hash
    }
}

static NEXT_USER_ID: AtomicUsize = AtomicUsize::new(1);
static DUST_PER_CEL: u128 = 1000000;

#[repr(u8)]
#[derive(FromPrimitive)]
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
        .unwrap_or_else(|_| "mongodb://admin:admin@localhost/".to_string());

    let mongodb_database_name =
        env::var("MONGO_DATABASE_NAME").unwrap_or_else(|_| "celestium".to_string());

    let mut client_options = match ClientOptions::parse(&mongodb_connection_string).await {
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
    if let Err(e) = database.run_command(doc! {"ping": 1}, None).await {
        println!(
            "Could not ping database \"{}\": {}",
            mongodb_database_name, e
        );
        return;
    };
    let database = warp::any().map(move || database.clone());

    println!("MongoDB Connected successfully.");

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
            nft_lookups_bin: load("nft_lookups")?,
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
    save("nft_lookups", wallet_bin.nft_lookups_bin)??;
    save("root_lookup", wallet_bin.root_lookup_bin)??;
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
    let wallet = Wallet::generate_init_blockchain(true)?;
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
    floating_outputs: HashSet,
) -> impl Filter<Extract = (HashSet,), Error = Infallible> + Clone {
    warp::any().map(move || floating_outputs)
}

async fn ws_handler(
    ws: warp::ws::Ws,
    wallet: SharedWallet,
    canvas: SharedCanvas,
    floating_outputs: HashSet,
    clients: WSClients,
    database: Database,
) -> Result<impl Reply, Rejection> {
    // weird boilerplate because I don't know why
    // this async function seems to just pass stuff on to another async function
    // but I don't know how to inline it 🤷
    Ok(ws.on_upgrade(move |socket| {
        client_connection(socket, wallet, canvas, floating_outputs, clients, database)
    }))
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
    floating_outputs: HashSet,
    clients: WSClients,
    database: Database,
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
                    println!("Websocket send error: {}", e);
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
    floating_outputs: &HashSet,
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
            parse_get_user_data(&binary_message[1..], sender, wallet).await
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
    let pk = *unwrap_or_ws_error!(sender, PublicKey::from_serialized(&pk, &mut 0));

    let item_name = match std::str::from_utf8(&bin_parameters[PUBLIC_KEY_COMPRESSED_SIZE..]) {
        Ok(i) => i,
        Err(e) => {
            ws_error!(sender, format!("Could not parse object name: {}", e));
        }
    };

    let store_collection = database.collection::<StoreItem>("store");

    let filter = doc! {"_id": item_name};
    let item = match store_collection.find_one(filter, None).await {
        Ok(Some(s)) => s,
        Err(e) => {
            ws_error!(
                sender,
                format!(
                    "Could not find Store Item with ID \"{}\": {}",
                    &item_name, e
                )
            );
        }
        _ => {
            ws_error!(
                sender,
                format!("Could not find Store Item with ID \"{}\"", &item_name)
            );
        }
    };

    {
        let real_wallet = wallet.lock().await;
        let (dust, inputs, _) = unwrap_or_ws_error!(
            sender,
            real_wallet.collect_for_coin_transfer(
                &unwrap_or_ws_error!(
                    sender,
                    TransactionValue::new_coin_transfer(item.store_price_in_dust as u128, 0)
                ),
                pk
            )
        );
        //real_wallet.
        let change = dust - item.store_price_in_dust as u128;
        let mut outputs = vec![TransactionOutput::new(
            unwrap_or_ws_error!(
                sender,
                TransactionValue::new_coin_transfer(item.store_price_in_dust as u128, 0)
            ),
            pk,
        )];
        if change > 0 {
            outputs.push(TransactionOutput::new(
                unwrap_or_ws_error!(sender, TransactionValue::new_coin_transfer(change, 0)),
                pk,
            ));
        }

        let transaction = unwrap_or_ws_error!(sender, Transaction::new(inputs, outputs));
        drop(real_wallet);
    }

    println!("{:?}", item);
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
    let image_url = image_url.as_bytes();
    let mut buffer = vec![0; (image_url.len() + 1) as usize];
    buffer[0] = CMDOpcodes::StoreItem as u8;
    buffer[1..].copy_from_slice(image_url);

    if let Err(e) = sender.send(Message::binary(buffer)) {
        println!("Could not send pixel hash: {}", e);
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
) {
    let mut pk = [0u8; PUBLIC_KEY_COMPRESSED_SIZE];
    pk.copy_from_slice(&bin_parameters[..PUBLIC_KEY_COMPRESSED_SIZE]);
    let pk = *unwrap_or_ws_error!(sender, PublicKey::from_serialized(&pk, &mut 0));

    let mut user_data_response = [0u8; 17];
    user_data_response[0] = CMDOpcodes::UserData as u8;

    {
        let real_wallet = wallet.lock().await;
        let balance = unwrap_or_ws_error!(sender, real_wallet.get_balance(pk));
        for i in 0..16 {
            user_data_response[i + 1] = (balance >> ((15 - i) * 8)) as u8;
        }
        drop(real_wallet);
    }

    if let Err(e) = sender.send(Message::binary(user_data_response)) {
        println!("Could not send user data: {}", e);
    };
}

async fn parse_get_pixel_data(
    bin_parameters: &[u8],
    sender: &mpsc::UnboundedSender<Message>,
    wallet: &SharedWallet,
    canvas: &SharedCanvas,
    floating_outputs: &HashSet,
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

    {
        let real_canvas = canvas.lock().await;
        let x: usize = ((bin_parameters[0] as usize) << 8) + (bin_parameters[1] as usize);
        let y: usize = ((bin_parameters[2] as usize) << 8) + (bin_parameters[3] as usize);
        let pixel = unwrap_or_ws_error!(sender, real_canvas.get_pixel(x, y));
        drop(real_canvas);
    }
    let mut client_pk = [0u8; PUBLIC_KEY_COMPRESSED_SIZE];
    client_pk.copy_from_slice(&bin_parameters[4..4 + PUBLIC_KEY_COMPRESSED_SIZE]);
    let client_pk = *unwrap_or_ws_error!(sender, PublicKey::from_serialized(&client_pk, &mut 0));

    let mut pixel_hash_response = [0u8; 1 + PIXEL_HASH_SIZE + HASH_SIZE];
    pixel_hash_response[0] = CMDOpcodes::PixelData as u8;
    {
        let real_wallet = wallet.lock().await;
        let value = unwrap_or_ws_error!(
            sender,
            TransactionValue::new_coin_transfer(DUST_PER_CEL, DUST_PER_CEL)
        );
        {
            let real_floating_outputs = floating_outputs.lock().await;
            let (dust, inputs, used_outputs) = unwrap_or_ws_error!(
                sender,
                real_wallet.collect_for_coin_transfer(&value, client_pk, real_floating_outputs)
            );
            real_floating_outputs
                .extend(used_outputs.map(|x| x[1]).collect::<Vec<[u8; HASH_SIZE]>>());
            drop(real_floating_outputs);
        }
        let mut outputs = [TransactionOutput::new(value, client_pk)];
        if dust > DUST_PER_CEL {
            outputs.push(TransactionOutput::new(
                TransactionValue::new_coin_transfer(dust - DUST_PER_CEL, 0),
                real_wallet.pk,
            ))
        }
        let block_hash = real_wallet.get_head_hash();
        let nft_output = TransactionOutput::new(TransactionValue::new_id_transfer());
        let transaction = Transaction::new(inputs, outputs);
        pixel_hash_response[PIXEL_HASH_SIZE + 1..].copy_from_slice(&block_hash);
        drop(real_wallet);
    }
    pixel_hash_response[1..PIXEL_HASH_SIZE + 1].copy_from_slice(&pixel.hash(x as u16, y as u16));

    if let Err(e) = sender.send(Message::binary(pixel_hash_response)) {
        println!("Could not send pixel hash: {}", e);
    };
}

async fn parse_transaction(
    bin_transaction: &[u8],
    sender: &mpsc::UnboundedSender<Message>,
    wallet: &SharedWallet,
    canvas: &SharedCanvas,
    clients: &WSClients,
    database: &Database,
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
}
