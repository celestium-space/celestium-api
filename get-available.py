from pymongo import MongoClient

DUST_PER_CEL = 10_000_000_000_000_000_000_000_000_000_000

client = MongoClient("localhost", 27018, maxPoolSize=50)
collection = client.asterank.asteroids

avail = collection.find({"state": "available"})
avail = sorted(list(avail), key=lambda l: int(l["store_value_in_dust"]), reverse=True)

cheap = [a for a in avail if int(a["store_value_in_dust"]) / (DUST_PER_CEL) < 1]

for a in [a for a in avail if 1_000_000_000 < a["price"] < 1_000_000_000_000]:
    print(f"{int(a['store_value_in_dust'])/(DUST_PER_CEL)} | {a['full_name']}")


print(f"{len(cheap)} cheap NFTs left")
