import asyncio
import random
from pathlib import Path

import click
import click_pathlib
import cv2
import numpy
import websockets
from tqdm import tqdm

from celestium import Ops, color_map, load_key_pair, set_pixel


def find_closest_color(color):
    closest = (0, 0, 1000)
    for i, allowed_color in enumerate(color_map):
        allowed_color = numpy.array(allowed_color)
        dist = numpy.linalg.norm(allowed_color[:3] - color[:3])
        if dist < closest[2]:
            closest = (i, 0xFF if color[3] else 0x00, dist)
    return [closest[0], closest[1]]


@click.command()
@click.option(
    "-i",
    "--image",
    type=click_pathlib.Path(exists=True, dir_okay=False, resolve_path=True),
    default=None,
    help="The image to place on canvas (completely transparrent pixels will be ignored)",
)
@click.option(
    "-x",
    type=int,
    required=True,
    help="Location of Top left corner of image x-coordinate",
)
@click.option(
    "-y",
    type=int,
    required=True,
    help="Location of Top left corner of image y-coordinate",
)
@click.option(
    "-s",
    "--sk",
    type=click_pathlib.Path(exists=True, dir_okay=False, resolve_path=True),
    required=True,
    help='Text file containing a SECP256k1 Signing Key (Secret Key) has hex string (use an exported "sk.txt" from https://celestium.space/wallet to get mined coin into your browsers wallet)',
)
@click.option(
    "--shuffle/--no-shuffle",
    default=True,
    help="Shuffle the order of pixels to place (default: True)",
)
@click.option(
    "-n",
    "--instance",
    type=str,
    default="wss://api.celestium.space/",
    help='URL of instance to target (default: "wss://api.celestium.space/")',
)
def cli(image, x, y, sk, shuffle, instance):
    asyncio.run(main(image, x, y, sk, shuffle, instance))


async def main(image, x, y, sk, shuffle, instance):
    # Get PK from exported SK
    (pk, sk) = load_key_pair(sk)

    pixels = []

    img = cv2.cvtColor(
        cv2.imread(str(image), cv2.IMREAD_UNCHANGED), cv2.COLOR_BGRA2RGBA
    )
    img = numpy.apply_along_axis(find_closest_color, 2, img)

    async with websockets.connect(instance, ping_interval=None) as ws:
        r = [0xFF]
        await ws.send(bytes([Ops.get_canvas.value]))
        while True:
            r = await ws.recv()
            if r[0] == Ops.canvas.value:
                break
            else:
                print(
                    f"Got unexpected response opcode, got {r[0]} expected {Ops.canvas.value}, I'll keep listening"
                )
    canvas = numpy.array([int(x) for x in r][1:]).reshape((1000, 1000))

    missing_img = numpy.zeros((img.shape[0], img.shape[1], 4))
    goal_img = numpy.zeros((img.shape[0], img.shape[1], 4))

    for py, row in enumerate(img):
        for px, [color, alpha] in enumerate(row):
            dc = color_map[color] + [alpha]
            try:
                cc = color_map[canvas[py + y][px + x]] + [0xFF]

                goal_img[py][px] = dc
                if dc[3] > 0 and (dc[0] != cc[0] or dc[1] != cc[1] or dc[2] != cc[2]):
                    raise IndexError
                else:
                    missing_img[py][px] = dc
            except IndexError:
                missing_img[py][px] = numpy.array([0xFF, 0x00, 0x00, 0xFF])
                pixels.append((px + x, py + y, color))

    missing_name = image.stem + "-missing.png"
    cv2.imwrite(
        missing_name,
        cv2.cvtColor(numpy.float32(missing_img), cv2.COLOR_RGBA2BGRA),
    )
    print(
        f'Saved image illustrating missing pixels as "{missing_name}" (only completely red pixels will be changed)'
    )

    goal_name = image.stem + "-goal.png"
    cv2.imwrite(
        goal_name,
        cv2.cvtColor(numpy.float32(goal_img), cv2.COLOR_RGBA2BGRA),
    )
    print(
        f'Saved our goal image as "{missing_name}" (when corrected for available colors)'
    )

    with Pool(thread_count) as pool:
        if pixels:
            if shuffle:
                random.shuffle(pixels)
            for px, py, c in tqdm(pixels, desc=f'Setting pixels for "{image.stem}"'):
                async with websockets.connect(instance, ping_interval=None) as ws:
                    r = [0xFF]
                    await ws.send(
                        bytes([Ops.get_pixel_color.value])
                        + px.to_bytes(2, "big")
                        + py.to_bytes(2, "big")
                    )
                    while True:
                        r = await ws.recv()
                        if r[0] == Ops.pixel_color.value:
                            break
                        else:
                            print(
                                f"Got unexpected response opcode, got {r[0]} expected {Ops.pixel_color.value}, I'll keep listening"
                            )
                cc = r[1]
                if cc != c:
                    await set_pixel(pk, px, py, c, pool, instance_url=instance)
                else:
                    print()
                    print(
                        f"({px}, {py}) set to correct color ({cc}) since initial check, skipping"
                    )
        else:
            print(f'All pixels already correct for "{image.stem}"')
    print("Done!")


if __name__ == "__main__":
    cli()
