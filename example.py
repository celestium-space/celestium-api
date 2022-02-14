import asyncio

from tqdm import tqdm

from celestium import Ops, color_map, load_key_pair, set_pixel
from logo import logo


async def main():
    # Get PK from exported SK
    pixels = []
    x_offset = 833
    y_offset = 833
    (pk, sk) = load_key_pair("sk.txt")

    for py, row in enumerate(logo):
        for px, color in enumerate(row):
            pixels.append((px + x_offset, py + y_offset, 3 if color else 4))

    for px, py, c in tqdm(pixels):
        await set_pixel(pk, px, py, c)
    print("Done!")


asyncio.run(main())
