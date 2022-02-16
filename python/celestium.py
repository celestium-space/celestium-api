import math
from enum import Enum
from hashlib import sha3_256
from typing import Optional

import ecdsa
import websockets

DEFAULT_PAR_WORK = 0x200000

color_map = [
    [0x00, 0x00, 0x00],
    [0xE5, 0x00, 0x00],
    [0x02, 0xBE, 0x01],
    [0x00, 0x00, 0xEA],
    [0xF8, 0xF2, 0x08],
    [0xFD, 0x5E, 0xF8],
    [0x00, 0xD3, 0xDD],
    [0xFF, 0xFF, 0xFF],
    [0x74, 0x15, 0xCD],
    [0xF3, 0xC9, 0x9D],
    [0x99, 0x99, 0x99],
    [0xE5, 0x95, 0x00],
    [0x00, 0x83, 0xC7],
    [0x34, 0x71, 0x15],
    [0x43, 0x27, 0x0A],
    [0x86, 0x5A, 0x48],
]


class Ops(Enum):
    get_pixel_color = 0x00
    pixel_color = 0x01
    get_canvas = 0x02
    canvas = 0x03
    updated_pixel_event = 0x04
    unmined_transaction = 0x05
    mined_transaction = 0x06
    get_pixel_mining_data = 0x07
    pixel_mining_data = 0x08
    get_store_item = 0x09
    store_item = 0x0A
    buy_store_item = 0x0B
    get_user_data = 0x0C
    user_data = 0x0D
    get_user_migration_transaction = 0x0E


def load_key_pair(sk_path):
    with open(sk_path) as f:
        sk = ecdsa.SigningKey.from_string(
            bytes.fromhex(f.read()), curve=ecdsa.SECP256k1
        )
        return (sk.get_verifying_key().to_string("compressed"), sk)


def magic_byte(i):
    """Make magic bytes for mining.
    Arguments are passed to `itertools.count`
    """
    # find out how many bytes to make for `i`
    num_bytes = math.ceil(math.ceil(math.log2(i + 1)) / 8) or 1

    # make bytes from `i`
    i_bytes = i.to_bytes(num_bytes, "big")
    assert len(i_bytes) == num_bytes and len(i_bytes) > 0

    # add continuation bits for every byte except last
    bs = bytes([b | 0x80 for b in i_bytes[:-1]]) + bytes([i_bytes[-1] & 0x7F])
    assert len(bs) == num_bytes

    # return magic bytes value
    return bs


def miner(obj) -> Optional[bytes]:
    (transaction_hash, start, end) = obj
    for mb in [magic_byte(i) for i in range(start, end)]:
        if sha3_256(transaction_hash + mb).digest().startswith(b"\0\0\0"):
            return mb
    return None


def mine(digest, pool, thread_count):
    start = 0
    mb = None

    while not mb:
        end = start + thread_count * DEFAULT_PAR_WORK
        mb = next(
            (
                m
                for m in pool.imap_unordered(
                    miner,
                    [
                        (digest, s, s + DEFAULT_PAR_WORK)
                        for s in range(
                            start,
                            end,
                            DEFAULT_PAR_WORK,
                        )
                    ],
                )
                if m
            ),
            None,
        )
        start = end
    return mb


async def set_pixel(
    pk,
    px,
    py,
    c,
    pool,
    thread_count,
    instance_url="wss://api.celestium.space/",
):
    async with websockets.connect(instance_url, ping_interval=None) as ws:
        # get previous pixel hash
        req = (
            bytes([Ops.get_pixel_mining_data.value])
            + px.to_bytes(2, "big")
            + py.to_bytes(2, "big")
            + pk
        )

        # Sending request for pixel mining data
        await ws.send(req)

        # Waiting for pixel mining data response...
        while True:
            r = await ws.recv()
            if isinstance(r, str):
                print("Server ws error: " + r)
                return
            if r.startswith(bytes([Ops.pixel_mining_data.value])):
                break

        # parse previous pixel hash stuff
        prev_pixel_hash = r[1:29]
        assert len(prev_pixel_hash) == 28

        block_head_hash = r[29:61]
        assert len(block_head_hash) == 32

        katjing_transaction = r[61:-1]

        # build a pixel NFT transaction
        transaction_len = sum(
            [
                version_len := 1,
                input_count_len := 1,
                input_blockhash_len := 32,
                input_message_len := 32,
                input_index_len := 1,
                output_count_len := 1,
                output_value_version_len := 1,
                output_value_id_len := 32,
                output_pk_len := 33,
            ]
        )
        pixel_transaction: bytes = b"".join(
            [
                version := bytes([0]),
                input_count := bytes([0]),
                block_head_hash,  # already defined
                prev_pixel_hash,  # already defined
                x_16b := px.to_bytes(2, "big"),
                y_16b := py.to_bytes(2, "big"),
                c_8b := bytes([c]),
                output_count := bytes([1]),
                output_value_id := bytes([1]),
                nft_hash := sha3_256(prev_pixel_hash + x_16b + y_16b + c_8b).digest(),
                pk,  # already defined
            ]
        )

        assert transaction_len == len(pixel_transaction)

        # Mine pixel transaction in parallel
        pixel_transaction = pixel_transaction + mine(
            sha3_256(pixel_transaction).digest(), pool, thread_count
        )

        # Mine katjing transaction in parallel
        katjing_transaction = katjing_transaction + mine(
            sha3_256(katjing_transaction).digest(), pool, thread_count
        )

        # send mined transactions back to api
        await ws.send(
            bs := bytes([Ops.mined_transaction.value])
            + pixel_transaction
            + katjing_transaction
        )
        r = await ws.recv()
        if isinstance(r, str):
            print("Server ws error: " + r)
