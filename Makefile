.PHONY: debug-run

debug-run:
	API_PORT=1337 CELESTIUM_DATA_DIR=data cargo run --features mining-ez-mode
