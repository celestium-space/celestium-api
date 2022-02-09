import asyncio
import itertools
import math
import struct
import time
from enum import Enum
from functools import partial
from hashlib import sha3_256
from multiprocessing import Pool, cpu_count
from typing import Optional
from tqdm import tqdm
from logo import logo

import ecdsa
import websockets

color_map = [
    0x000000,
    0xE50000,
    0x02BE01,
    0x0000EA,
    0xF8F208,
    0xFD5EF8,
    0x00D3DD,
    0xFFFFFF,
    0x7415CD,
    0xF3C99D,
    0x999999,
    0xE59500,
    0x0083C7,
    0x347115,
    0x43270A,
    0x865A48,
]


class Ops(Enum):
    get_canvas = 0x01
    got_canvas = 0x02
    update_pixel = 0x03
    unmined_transaction = 0x04
    mined_transaction = 0x05
    get_pixel = 0x06
    got_pixel = 0x07


def magic_bytes(*args, **kwargs):
    """Make magic bytes for mining.
    Arguments are passed to `itertools.count`
    """
    for i in itertools.count(*args, **kwargs):
        # find out how many bytes to make for `i`
        num_bytes = math.ceil(math.ceil(math.log2(i + 1)) / 8) or 1

        # make bytes from `i`
        i_bytes = i.to_bytes(num_bytes, "big")
        assert len(i_bytes) == num_bytes and len(i_bytes) > 0

        # add continuation bits for every byte except last
        bs = bytes([b | 0x80 for b in i_bytes[:-1]]) + bytes([i_bytes[-1] & 0x7F])
        assert len(bs) == num_bytes

        # yield magic bytes value
        yield bs


def miner(
    transaction_hash: bytes, thread_count: int, thread_num: int
) -> Optional[bytes]:
    for mb in magic_bytes(start=thread_num, step=thread_count):
        if sha3_256(transaction_hash + mb).digest().startswith(b"\0\0\0"):
            return mb
    return None


async def main():
    async with websockets.connect(
        "wss://api.celestium.space/", ping_interval=None
    ) as ws:
        # Get PK from exported SK
        with open("sk.txt") as f:
            pk = (
                ecdsa.SigningKey.from_string(
                    bytes.fromhex(f.read()), curve=ecdsa.SECP256k1
                )
                .get_verifying_key()
                .to_string("compressed")
            )

        pixels = []
        x_offset = 50
        y_offset = 50
        text_color = 4
        background_color = 3

        for py, row in enumerate(logo):
            for px, color in enumerate(row):
                pixels.append(
                    (
                        px + x_offset,
                        py + y_offset,
                        text_color if color else background_color,
                    )
                )

        start_time = time.time()
        for px, py, c in tqdm(pixels):
            # get previous pixel hash
            req = (
                bytes([Ops.get_pixel.value])
                + px.to_bytes(2, "big")
                + py.to_bytes(2, "big")
                + pk
            )
            # Sending request for pixel hash: {req=}")
            await ws.send(req)

            # Waiting for pixel hash response...
            while True:
                r = await ws.recv()
                if isinstance(r, str):
                    print("Error: " + r)
                    quit(1)
                if r.startswith(bytes([Ops.got_pixel.value])):
                    break

            # parse previous pixel hash stuff
            assert r[0] == Ops.got_pixel.value, f"Unexpected opcode: {r[0:50]}"

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
                    nft_hash := sha3_256(
                        prev_pixel_hash + x_16b + y_16b + c_8b
                    ).digest(),
                    pk,  # already defined
                ]
            )

            assert transaction_len == len(pixel_transaction)

            # Mine pixel transaction in parallel
            thread_count = cpu_count()
            p = Pool(thread_count)
            mb = next(
                p.imap_unordered(
                    partial(miner, sha3_256(pixel_transaction).digest(), thread_count),
                    range(thread_count),
                )
            )
            p.terminate()
            pixel_transaction = pixel_transaction + mb

            # send mined transaction back to api
            await ws.send(
                bs := bytes([Ops.mined_transaction.value]) + pixel_transaction
            )

            # Mine katjing transaction in parallel
            thread_count = cpu_count()
            p = Pool(thread_count)
            mb = next(
                p.imap_unordered(
                    partial(
                        miner, sha3_256(katjing_transaction).digest(), thread_count
                    ),
                    range(thread_count),
                )
            )
            p.terminate()
            katjing_transaction = katjing_transaction + mb

            # send mined transaction back to api
            await ws.send(
                bs := bytes([Ops.mined_transaction.value]) + katjing_transaction
            )

        took = time.time() - start_time
        print(
            f"Avg mining time: {took/(len(pixels)*2)}s ({took/cpu_count()/(len(pixels)*2)}s per core)"
        )


asyncio.run(main())

