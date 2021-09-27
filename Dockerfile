FROM rust:1.53-buster

WORKDIR /app
COPY ./src /app/src
COPY ./Cargo.lock /app/Cargo.lock
COPY ./Cargo.toml /app/Cargo.toml
COPY ./celestium-lib /app/celestium-lib
RUN cargo build --release

CMD /app/target/release/celestium-api
