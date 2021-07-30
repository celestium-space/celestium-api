
# Celestium API

## What is this?

This project is a Websocket API which the web clients will connect to.

It serves these purposes:

- Validating transactions from clients
- Forwarding transactions to ISS miner
- Traversing the blockchain, so the clients don't have to. The server maintains...
  - Image state with latest pixels at each of the 1M positions
  - Theme-poll state with theme submissions and vote counts


## Protocol

Everything is done via websockets.
Communication is split into a handful of packet types.
Here's what they are:

**From API server to client:**

update pixel: `0x00<x:u16><y:u16><color:u8>`

entire image: `0x01<image:[u8; 1_000_000]>`

error: `0x02<msg:str>`

**From client to API server:**

blockchain transaction: `<transaction:[u8]>`


## Configuration

The environment variable `CELESTIUM_DATA_DIR` controls where the API server state is stored.
This folder will contain *critical* data, like private keys and the entire blockchain.
It absolutely needs to be backed up.
