
# Celestium API


## What is this?

This project is a Websocket API which the web clients will connect to.

It serves three main purposes:

- Maintaining the Celestium Blockchain
  - Keep complete Celestium Blockchain state
  - Latest mined block needed for a `Pixel Place NFT Transaction`
  - Recieve a client created (and mined) `Pixel Place NFT Transaction` and store it on off-chain transactions
  - Take all recieved off chain transactions, construct a Merkle Tree and Block; ready for mining in space
  - Validate transactions from clients
- Maintaining image state
  - Keep all latest pixels at each of the 1M positions
  - Send entire image as "columns-first-scan"
  - Send latest pixel hash needed for a `Pixel Place NFT Transaction` given coordinates
  - Theme-poll state with theme submissions and vote counts
- Maintaining NFT store
  - Keep entire store state and data
  - Send store items as paginated data
  - Facilitate store purchaces


## Protocol

Everything is done via websockets. 

Communication is split into a several of message types.
The first byte in every message (sent or received) indicates the message type.
The message types and their opcodes are as follows:

| CMD Opcode | Direction | Description           | Data                                 |
| ---------- | --------- | --------------------  | ------------------------------------ |
| 0x01       | C -> API  | get entire image      |                                      |
| 0x02       | API -> C  | entire image response | `<image:[u8; 1_000_000]>`            |
| 0x03       | API -> C  | update pixel          | `<x:u16><y:u16><color:u8>`           |
| 0x04       | C -> API  | unmined transaction   | `<transaction:[u8]>`                 |
| 0x05       | C -> API  | mined transaction     | `<transaction:[u8]>`                 |
| 0x06       | C -> API  | get pixel data        | `<x:u16><y:u16><public_key>`         |
| 0x07       | API -> C  | pixel data response   | `<prev_pixel_hash:[u8;28]><block_head_hash:[u8;32]><katjing:[u8]>` |
| 0x08       | C -> API  | get store items       | `<from_index:u32><to_index:u32>`     |
| 0x09       | API -> C  | store items response  | `<store_items:JSON>`                 |
| 0x0a       | C -> API  | buy store item        | `<id:[u8]>`                          |
| 0x0b       | C -> API  | get user data         | `<public_key:[u8; 32]>`              |
| 0x0c       | API -> C  | user data             | `<total_celestium:u64>`              |


All multi-byte numbers are in big-endian format.

## How to use/Use cases (client)

### Pixel Place

1. Connect to websockets on path `/` (`wss://api.celestium.space` on the public instance)

2. Send `[0x01]` and listen. You will immediately receive one million and one bytes. This first byte in this response will be `0x02`. The remaining bytes bytes are a "columns-first-scan" of the initial image: i.e. the second byte is the color of the top-left pixel (0,0), the third byte is the pixel below that (0,1), and so on...

3. Send `[0x06]` to the server, and listen. You will soon receive a response which begins with `0x07`. You can parse this byte array according to the table above, to extract `prev_pixel_hash`, `block_head_hash`, and `katjing`.
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



### Store
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
