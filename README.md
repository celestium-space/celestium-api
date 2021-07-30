
# Celestium API


## What is this?

This project is a Websocket API which the web clients will connect to.

It serves these purposes:

- Validating transactions from clients
- Forwarding transactions to ISS miner (how the hell do we do this?)
- Traversing the transactisno, so the clients don't have to. The server maintains...
  - Image state with latest pixels at each of the 1M positions
  - Theme-poll state with theme submissions and vote counts


## Protocol

Everything is done via websockets.
Communication is split into a handful of packet types.
Here's what they are:

**From API server to client:**

entire image: `<image:[u8; 1_000_000]>`

update pixel: `<x:u16><y:u16><color:u8>`

error: `<0x45:u8><msg:str>`

**From client to API server:**

blockchain transaction: `<transaction:[u8]>`


## How to Use

STEP 1. Connect to websockets on path `/`

STEP 2. Listen. You will immediately receive one million bytes.
These bytes are a "columns-first-scan" of the initial image. The first byte is the top-left pixel (0,0), the second byte is the pixel below that (0,1).

STEP 3. Make a transaction. This logic is defined in celestium-lib. Ask Hutli.

STEP 4. Serialize that transaction and send it as a binary message to the server.

STEP 5. ???

STEP 6. PROFIT!

You also need to continually listen for incoming messages, which can tell you about new pixels (or error messages).


## Configuration

The environment variable `CELESTIUM_DATA_DIR` controls where the API server state is stored.
This folder will contain *critical* data, like private keys and the entire blockchain.
It absolutely needs to be backed up.
