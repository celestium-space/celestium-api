// external
use warp::{
    Filter,
    Reply,
    Rejection,
    ws::WebSocket,
    filters::ws::Message
};
use tokio::sync::Mutex;
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};

// here
use celestium::{
    block::Block,
    block_hash::BlockHash,
    serialize::{DynamicSized, Serialize},
    transaction::Transaction,
    wallet::{Wallet, DEFAULT_N_THREADS, DEFAULT_PAR_WORK},
};

#[tokio::main]
async fn main() {
    // configure ws route
    let ws_route = warp::path::end()
        .and(warp::ws())
        .and_then(ws_handler)
        .with(warp::cors().allow_any_origin());

    println!("Starting server");
    warp::serve(ws_route).run(([0, 0, 0, 0], 8000)).await;
}

async fn ws_handler(ws: warp::ws::Ws) -> Result<impl Reply, Rejection> {
    // weird boilerplate because I don't know why
    // this async function seems to just pass stuff on to another async function
    // but I don't know how to inline it 🤷
    Ok(ws.on_upgrade(move |socket| client_connection(socket)))
}

async fn client_connection(ws: warp::ws::WebSocket) {
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
        handle_ws_message(message, &mut sender).await;
    }
}

async fn handle_ws_message(message: Message, sender: &mut SplitSink<WebSocket, Message>) {
    // this is the function that actually receives a message
    // rn it just echoes the message back
    sender.send(message).await.unwrap();
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
