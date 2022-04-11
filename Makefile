.PHONY: debug-run

debug-run:
	API_PORT=1337 CELESTIUM_DATA_DIR=data cargo run --features mining-ez-mode
run:
	API_PORT=1337 CELESTIUM_DATA_DIR=data cargo run --release
run-freeze:
	API_PORT=1337 CELESTIUM_DATA_DIR=data cargo run --release --features freeze-blockchain
run-freeze-canvas:
	API_PORT=1337 CELESTIUM_DATA_DIR=data cargo run --release --features freeze-canvas
