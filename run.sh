#!/bin/sh
MONGODB_CONNECTION_STRING="mongodb://localhost:27018" API_PORT="1337" make run-freeze
#MONGODB_CONNECTION_STRING="mongodb://localhost:27018" API_PORT="1337" RELOAD_UNSPENT_OUTPUTS=true IGNORE_OFF_CHAIN_TRANSACTIONS=true make run
