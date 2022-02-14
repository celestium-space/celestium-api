
# Celestium API

This project is a Websocket API exposing functionality to interact with a Celestium Node.
The websocket API makes it possible for web clients will connect to.

It serves three main purposes:

- Maintaining the Celestium Blockchain
  - Keep complete Celestium Blockchain state
  - Expose data needed for clients to create new [Transactions](https://gitlab.com/artificialmind/celestium/celestium-lib/-/wikis/Transaction)
  - Recieve client created (and mined) [Transactions](https://gitlab.com/artificialmind/celestium/celestium-lib/-/wikis/Transaction), verify them and store them as off-chain transactions
  - Take all recieved off chain transactions and construct a Block; ready for mining in space
- Maintaining canvas
  - Keep all latest pixels at each of the 1M positions
  - Expose pixel data as either single pixels or entire canvas
- Maintaining NFT store
  - Keep entire store state and data
  - Expose store items and data needed to create transaction purchase transactions

## API Protocol

Everything is done via websockets and all commands are sent as binary websocket packages.
Errors are sent in text websocket packages.

The first byte in every message (sent or received) indicates the command.
Commands and their opcodes are as follows:

| CMD Opcode                                  | Direction | Description                    | Data                                 |
| ------------------------------------------- | --------- | ------------------------------ | ------------------------------------ |
| [0x00](0x00-get-pixel-color)                | C -> API  | get pixel color                | `<x:u16><y:u16>`                     |
| [0x01](0x01-pixel-color)                    | API -> C  | pixel color                    | `[color:u8; 1]`                      |
| [0x02](0x02-get-entire-image)               | C -> API  | get entire image               |                                      |
| [0x03](0x03-entire-image)                   | API -> C  | entire image                   | `<image:[u8; 1_000_000]>`            |
| [0x04](0x04-updated-pixel-event)            | API -> C  | updated pixel event            | `<x:u16><y:u16><color:u8>`           |
| [0x05](0x05-unmined-transaction)            | API -> C  | unmined transaction            | `<transaction:[u8]>`                 |
| [0x06](0x06-mined-transaction)              | C -> API  | mined transaction              | `<transaction:[u8]>`                 |
| [0x07](0x07-get-pixel-mining-data)          | C -> API  | get pixel mining data          | `<x:u16><y:u16><public_key>`         |
| [0x08](0x08-pixel-mining-data)              | API -> C  | pixel mining data              | `<prev_pixel_hash:[u8;28]><block_head_hash:[u8;32]><katjing_transaction:[u8]>` |
| [0x09](0x09-get-store-items)                | C -> API  | get store items                | `<from_index:u32><to_index:u32>`     |
| [0x0a](0x0a-store-items)                    | API -> C  | store items                    | `<store_items:JSON>`                 |
| [0x0b](0x0b-buy-tore-item)                  | C -> API  | buy store item                 | `<id:[u8]>`                          |
| [0x0c](0x0c-get-user-data)                  | C -> API  | get user data                  | `<public_key:[u8; 32]>`              |
| [0x0d](0x0d-user-data)                      | API -> C  | user data                      | `<total_celestium:u64>`              |
| [0x0e](0x0e-get-user-migration-transaction) | C -> API  | get user migration transaction | `<total_celestium:u64>`              |

All multi-byte numbers are in big-endian format.

## Useage
We have currently implemented two clients for the API. One is found in the [Celestium Frontend](https://gitlab.com/artificialmind/celestium/celestium-frontend) and the other is a bare bones Python client found in this repository.

### Python 
The file `celestium.py` in the root of this repository exposes functions and constants meant to help people automate interactions with the Canvas programically. We have limited the scope of these functions to canvas interactions as this is the most entertaining part of the project to automate.

### load_key_pair
This function is meant to help people load their Public Key and Secret Key from an exported [`sk.txt`](https://celestium.space/wallet).

### set_pixel
This function will request the api for the data needed to create the two transactions needed to set a pixel on the canvas, `pixel base transaction` and `katjing transaction`. It will then create and mine both transactions and send the result back to the API, changing the pixel and transferring 1 CEL to the PK given.

### color_map
This constant is a simple lookup table from color indicies to the actual RGB color on the canvas.

### Client useage examples
#### Pixel Place

1. Connect to websockets on path `/` (`wss://api.celestium.space` on the public instance)

2. Send `[0x01]` and listen. You will immediately receive one million and one bytes. This first byte in this response will be `0x02`. The remaining bytes bytes are a "columns-first-scan" of the initial image: i.e. the second byte is the color of the top-left pixel (0,0), the third byte is the pixel below that (0,1), and so on...

3. Send `[0x06]` to the server, and listen. You will soon receive a response which begins with `0x07`. You can parse this byte array according to the table above, to extract `prev_pixel_hash`, `block_head_hash`, and `katjing transaction`.
Once you have received these bytes, construct a new bytes object which is structured looks like this:

```
[
  0x00,
  0x00,
  <block_head_hash:[u8;32]>,
  <prev_pixel_hash:[u8;28]>,
  <x:[u8;2]>,
  <y:[u8;2]>,
  <c:u8>,
  0x01,
  0x01,
  sha3_256(<prev_pixel_hash:[u8;32]> + <x:[u8;2]> + <y:[u8;2]> + <c:u8>)
]
```

(`x`, `y`, and `c` are x/y coordinates and color index. all numbers are big-endian)

Once you have constructed the transaction (the byte-array above), you need to find some "magic bytes" which you can append to a SHA3_256 hash of it, in order to make the transaction's `SHA3_256` hash end with at least three zero-bytes. This is the mining task.
There is (hopefully) no way to predict this - so just pick some random bytes to append, and retry until you get a hash that ends with three null-bytes.

3a) Take a SHA3_256 hash of the transaction bytes you just constructed.

3b) Concat a handful of random bytes to that hash digest.

3c) SHA3_256 the concatenated bytes. Repeat from 3b) if the resulting digest does not end with 3 zero-bytes.

3d) Send a byte array structured like this: `[0x05, 0x01, <mined_transaction:[u8]>]`

Tada! That's it! If you don't receive a string back with an error message, it means you have successfully set a pixel!

Remember: You need to continually listen for incoming messages, which will tell you about updated pixels (including the ones you set) or error messages.



#### Store
1. Connect to websockets on path `/` (`wss://api.celestium.space` on the public instance)

2. Send `[0x0b, 0x00]` and listen. You will recieve the items for the store (first page) JSON formatted.

3. Populate your HTML store page with the data. Use the socket command `[0x06, page]` to recieve more pages as the user traverses your store.

4. When the user engages in a buy action send `[0x08, id]`.

5. Listen. You will recieve a serialized transaction transferring the NFT to you and required Celestium to the NFT ower (if the user does not have enough Celestium you will recieve an error).

6. Sign the appropriate transaction input(s) and mine the transaction.

7. Serialize that transaction, prepend 0x03 and send it back.

## Configuration

The environment variable `CELESTIUM_DATA_DIR` controls where the API server state is stored.
This folder will contain *critical* data, like private keys and the entire blockchain.
It absolutely needs to be backed up.

## Opcode explainations
### 0x00 get pixel color
### 0x01 pixel color
### 0x02 get entire image 
### 0x03 entire image
### 0x04 updated pixel event
### 0x05 unmined transaction
### 0x06 mined transaction
### 0x07 get pixel mining data
### 0x08 pixel mining data
### 0x09 get store items
### 0x0a store items
### 0x0b buy store item 
### 0x0c get user data
### 0x0d user data
### 0x0e get user migration transaction