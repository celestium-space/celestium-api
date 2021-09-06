
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

Everything is done via websockets. Communication is split into a several of packet types, they are:

| CMD Opcode | Direction | Description          | Endpoint | Data                                 |
| ---------- | --------- | -------------------- | -------- | ------------------------------------ |
| 0x00       | API -> C  | entire image         | `/`      | `<image:[u8; 1_000_000]>`            |
| 0x01       | API -> C  | update pixel         | `/`      | `<x:u16><y:u16><color:u8>`           |
| 0x02       | API -> C  | error                | `/`      | `<0x45:u8><msg:str>`                 |
| 0x03       | C -> API  | transaction          | `/`      | `<transaction:[u8]>`                 |
| 0x04       | C -> API  | get pixel hash       | `/`      | `<x:u16><y:u16>`                     |
| 0x05       | API -> C  | pixel hash response  | `/`      | `<pixel_hash:[u8]><block_hash:[u8]>` |
| 0x06       | C -> API  | get store items      | `/`      | `<page_nr:u64>`                      |
| 0x07       | API -> C  | store items response | `/`      | `<store_items:JSON>`                 |
| 0x08       | C -> API  | buy store item       | `/`      | `<id:[u8]>`                          |
| 0x09       | API -> C  | transaction          | `/`      | `<transaction:[u8]>`                 |
| 0x0a       | C -> API  | get user data        | `/`      | `<public_key:[u8; 32]>`              |
| 0x0b       | C -> API  | get entire image     | `/`      | `<none>`                             |
| 0x0c       | API -> C  | user data            | `/`      | `<total_celestium:u64>`              |

## How to use/Use cases (client)
### Pixel Place
1. Connect to websockets on path `/`

2. Send `[0x0b]` and listen. You will immediately receive one million bytes. These bytes are a "columns-first-scan" of the initial image. The first byte is the top-left pixel (0,0), the second byte is the pixel below that (0,1).

3. Create and mine a pixel place NFT base transaction.

4. Serialize that transaction, prepend 0x03 and send it back.

Remember: You need to continually listen for incoming messages, which will tell you about updated pixels (including the ones you set) or error messages.

### Store
1. Connect to websockets on path `/`

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
