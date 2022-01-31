import json

with open("schema.json", "r") as f:
    schema = json.loads(f.read())

for field in schema["fields"]:
    types = sorted(
        [t for t in field["types"] if t["name"].lower() != "undefined"],
        key=lambda x: x["probability"],
    )
    print(
        f'{field["name"]}: {"Option<" if len(types) > 1 else ""}{types[0]["name"]}{">" if len(types) > 1 else ""},'
    )
