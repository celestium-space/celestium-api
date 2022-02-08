# std lib
import asyncio
from enum import Enum
import itertools
import math
import struct
from hashlib import sha3_256
from multiprocessing import Pool
from functools import partial
from typing import Optional

# 3d party
import websockets

# Get pubkey from js console: localStorage.getItem("pk_bin")
pubkey = bytes.fromhex("03c99405208391fdb33ce374ff7fbda265f8188dea3d4a4dcdb7bb01f7d36c024b")

class Ops(Enum):
    get_canvas = 0x01
    got_canvas = 0x02
    update_pixel = 0x03
    unmined_transaction = 0x04
    mined_transaction = 0x05
    get_pixel = 0x06
    got_pixel = 0x07


def magic_bytes(*args, **kwargs):
    """ Make magic bytes for mining.
        Arguments are passed to `itertools.count`
    """
    for i in itertools.count(*args, **kwargs):
        # find out how many bytes to make for `i`
        num_bytes = math.ceil(math.ceil(math.log2(i+1)) / 8) or 1

        # make bytes from `i`
        i_bytes = i.to_bytes(num_bytes, 'big')
        assert len(i_bytes) == num_bytes and len(i_bytes) > 0

        # add continuation bits for every byte except last
        bs = bytes([b | 0x80 for b in i_bytes[:-1]]) + bytes([i_bytes[-1] & 0x7f])
        assert len(bs) == num_bytes

        # yield magic bytes value
        yield bs


def miner(transaction_hash: bytes, thread_count: int, thread_num: int) -> Optional[bytes]:
    for mb in magic_bytes(start=thread_num, step=thread_count):
        if sha3_256(transaction_hash + mb).digest().startswith(b"\0\0\0"):
            print("Mining complete!")
            return mb
    return None


async def main():
    async with websockets.connect("wss://api.celestium.space/", ping_interval=None) as ws:

        # get canvas
        await ws.send(bytes([Ops.get_canvas.value]))
        r = await ws.recv()
        assert r[0] == Ops.got_canvas.value
        assert len(r) == 1_000_001
        # print(f"Got something: {len(r)} bytes")
        print(f"Canvas: {r[:50]}")

        # mine a pixel
        px, py, c = 0, 0, 0

        # get previous pixel hash
        print("Sending request for pixel hash")
        await ws.send(
            bytes([Ops.get_pixel.value])
            + struct.pack("HH", px, py)
            + pubkey
        )
        print("Waiting for pixel hash response...")
        while not (r := await ws.recv()).startswith(bytes([Ops.got_pixel.value])):
            print(f"Ignoring opcode: {r[0]}")
            pass
        if isinstance(r, str):
            print("Error: " + r)
            quit(1)

        # parse previous pixel hash stuff
        # assert len(r) == 328, f"Got len: {len(r)} expected 328" what is actual len?
        print(f"Pixel len is {len(r)}")
        assert r[0] == Ops.got_pixel.value, f"Unexpected opcode: {r[0:50]}"

        prev_pixel_hash = r[1:29]
        print(f"{len(prev_pixel_hash)=}")
        assert len(prev_pixel_hash) == 28

        block_head_hash = r[29:61]
        print(f"{len(block_head_hash)=}")
        assert len(block_head_hash) == 32

        katjing_transaction = r[61:]
        print(len(katjing_transaction))
        print(f"{len(katjing_transaction)=}")

        # build a pixel NFT transaction
        transaction_len = sum([
            version_len := 1,
            input_count_len := 1,
            input_blockhash_len := 32,
            input_message_len := 32,
            input_index_len := 1,
            output_count_len := 1,
            output_value_version_len := 1,
            output_value_id_len := 32,
            output_pk_len := 33,
        ])
        transaction: bytes = b"".join([
            version         := bytes([0]),
            input_count     := bytes([0]),
            block_head_hash, # already defined
            prev_pixel_hash, # already defined
            x_16b           := px.to_bytes(2, 'big'),
            y_16b           := py.to_bytes(2, 'big'),
            c_8b            := bytes([c]),
            output_count    := bytes([1]),
            output_value_id := bytes([1]),
            nft_hash        := sha3_256(prev_pixel_hash
                                        + x_16b + y_16b + c_8b
                                        + output_count).digest(),
            pubkey           # already defined
        ])
        print(f"{len(transaction)=}")
        assert transaction_len == len(transaction)

        # mine transaction in parallel
        thread_count = 8
        p = Pool(thread_count)
        print(f"Mining with {thread_count=}...")
        mb = next(p.imap_unordered(
            partial(miner, sha3_256(transaction).digest(), thread_count),
            range(thread_count)
        ))
        p.terminate()
        print(f"Found {mb=}")

        # send mined transaction back to api
        mined_transaction = transaction + mb

        print(f"Sending mined transaction")
        await ws.send(bs := bytes([Ops.mined_transaction.value, 0x01]) + mined_transaction)

        print(f"{mined_transaction=}")

        print(f"total payload: {bs}")

        print(f"Mined transaction sent! Waiting for error or other response...")
        while r := await ws.recv():
            if isinstance(r, str):
                print(f"Error: {r=}")
            elif r[0] == Ops.update_pixel.value:
                print(f"Skipping opcode: {r[0]=}")
            else:
                print(f"Got opcode: {r[0]=}")
                break


asyncio.run(main())
