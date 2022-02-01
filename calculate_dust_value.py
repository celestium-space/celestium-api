import multiprocessing
import random
import subprocess
import sys

import click
import click_pathlib
import pandas as pd
from bson.decimal128 import Decimal128
from loguru import logger
from pymongo import MongoClient, errors
from tqdm import tqdm


def split(x, n):
    if x < n:
        return -1
    elif x % n == 0:
        v = x // n
        return [v for i in range(n)]
    else:
        zp = n - (x % n)
        pp = x // n
        return [pp + 1 if i >= zp else pp for i in range(n)]


@click.group()
def cli():
    pass


@cli.command()
def calculate_dust_value():
    client = MongoClient("localhost", 27018, maxPoolSize=50)
    collection = client.asterank.asteroids
    cursor = collection.find({})
    total_price_value = 0.0
    min_value = float("inf")
    max_value = 0.0
    d0 = []
    data = []
    for document in tqdm(cursor, total=collection.count_documents({})):
        if "price" in document and (
            isinstance(document["price"], float) or isinstance(document["price"], int)
        ):
            d0.append(document)
            if document["price"] > 1.0:
                total_price_value += document["price"]
                min_value = min(min_value, document["price"])
                max_value = max(max_value, document["price"])
                number = document["price"]
                data.append(number)
        else:
            logger.error(
                f'Document "{document["id"]}" ({document["full_name"]}) has price "{document["price"]}" which is not float nor int'
            )
            return

    plot = pd.DataFrame(data).plot(kind="density")
    plot.set_yscale("log")
    fig = plot.get_figure()
    fig.savefig("figure.png")

    logger.info(f"Total price value: {total_price_value}")
    logger.info(f"Values: {min_value} - {max_value}")

    d1 = []
    total_store_value = 0.0
    for document in tqdm(d0):
        document["store_value"] = (
            document["price"]
            if document["price"] > 1_000_000_000
            else random.uniform(1_000_000_000, 1_000_000_000_000)
        )
        total_store_value += document["store_value"]
        d1.append(document)

    TOTAL_DUST = pow(2, 128) - 1

    value_factor = TOTAL_DUST / total_store_value

    logger.info(f"Total store value: {total_store_value}")
    logger.info(f"Value factor: {value_factor}")

    d2 = []
    total_store_value_in_dust = 0
    for document in tqdm(d1):
        document["store_value_in_dust"] = int(document["store_value"] * value_factor)
        total_store_value_in_dust += document["store_value_in_dust"]
        d2.append(document)

    ran_dust_distro_docs = [d for d in d2 if d["price"] < 1.0]
    chunks = split(TOTAL_DUST - total_store_value_in_dust, len(ran_dust_distro_docs))
    logger.info(f"Total store value: {total_store_value_in_dust}")
    logger.info(f"Dust diff: {TOTAL_DUST - total_store_value_in_dust}")
    logger.info(f"D : {len(ran_dust_distro_docs)} ({set(chunks)})")

    d3 = []
    i = 0
    total_store_value_in_dust = 0
    for document in tqdm(d2):
        if document["price"] < 1.0:
            document["store_value_in_dust"] += chunks[i]
            i += 1
        total_store_value_in_dust += document["store_value_in_dust"]
        d3.append(document)

    logger.info(f"Total store value: {hex(total_store_value_in_dust)}")
    logger.info(f"Balanced dust diff: {TOTAL_DUST - total_store_value_in_dust}")

    for document in tqdm(d3):
        collection.update_one(
            {"_id": document["_id"]},
            {"$set": {"store_value_in_dust": f'{document["store_value_in_dust"]}'}},
        )

        d1.append(document)


def convert_files(obj):
    image, asteroid, output, output_256, convert = obj
    if convert:
        process = subprocess.Popen(
            [
                "ffmpeg",
                "-y",
                "-i",
                image,
                "-c:v",
                "libx264",
                f'{output / asteroid["full_name"]}.mp4',
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        process.communicate()
        process = subprocess.Popen(
            [
                "ffmpeg",
                "-y",
                "-i",
                image,
                "-c:v",
                "libx264",
                "-vf",
                "scale=256:256",
                f'{output_256 / asteroid["full_name"]}.mp4',
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        process.communicate()
        process = subprocess.Popen(
            [
                "ffmpeg",
                "-y",
                "-i",
                image,
                "-vf",
                "scale=256:256",
                f'{output_256 / asteroid["full_name"]}.gif',
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        process.communicate()
    else:
        stem = image.stem
        image.rename(image.parent / f"{asteroid['full_name']}{image.suffix}")
        (output_256 / f"{stem}.mp4").rename(output_256 / f"{asteroid['full_name']}.mp4")
        (output_256 / f"{stem}.gif").rename(output_256 / f"{asteroid['full_name']}.gif")


@cli.command()
@click.option(
    "--images",
    "-i",
    required=True,
    type=click_pathlib.Path(exists=True, file_okay=False, resolve_path=True),
    help="Directory containing images to use",
)
@click.option(
    "--output",
    "-o",
    required=True,
    type=click_pathlib.Path(exists=True, file_okay=False, resolve_path=True),
    help="Directory to save converted images",
)
@click.option(
    "--output-256",
    "-s",
    required=True,
    type=click_pathlib.Path(exists=True, file_okay=False, resolve_path=True),
    help="Directory to save 256x256 resized images",
)
@click.option(
    "--convert",
    "-c",
    default=True,
    type=bool,
    help="Conversion mode (default) or rename mode (ignores output folder)",
)
def rename_images(images, output, output_256, convert):

    images = list(images.iterdir())
    try:
        client = MongoClient("localhost", 27018, maxPoolSize=50)
        collection = client.asterank.asteroids
        cursor = collection.find({})
        certain_asteroids = []
        other_asteroids = []
        for document in tqdm(cursor, total=collection.count_documents({})):
            if "price" in document and (
                isinstance(document["price"], float)
                or isinstance(document["price"], int)
            ):
                if document["price"] > 1.0:
                    certain_asteroids.append(document)
                else:
                    other_asteroids.append(document)
            else:
                logger.error(
                    f'Document "{document["id"]}" ({document["full_name"]}) has price "{document["price"]}" which is not float nor int'
                )
                return
        random.shuffle(images)
    except errors.ServerSelectionTimeoutError as e:
        logger.warning(
            f"WARNING: Could not find database, using default naming scheme: {e}"
        )
        certain_asteroids = [
            {"full_name": str(x).zfill(10), "price": 1_000_000 - x}
            for x in range(1_000_000)
        ]
        other_asteroids = []
        images = list(sorted(images, key=lambda x: int(x.stem)))

    collection.update_many(
        {"state": {"$ne": "bought"}}, {"$set": {"state": "unavailable"}}
    )

    objs = [
        (i, a, output, output_256, convert)
        for i, a in zip(
            images,
            sorted(certain_asteroids, key=lambda x: x["price"], reverse=True)
            + other_asteroids,
        )
    ]
    for (_, a, _, _, _) in objs:
        collection.update_one({"_id": a["_id"]}, {"$set": {"state": "available"}})

    with multiprocessing.Pool(multiprocessing.cpu_count()) as pool:
        [
            obj
            for obj in tqdm(
                pool.imap_unordered(convert_files, objs),
                total=len(objs),
            )
        ]


if __name__ == "__main__":
    cli()
