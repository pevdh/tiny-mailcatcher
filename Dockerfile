# Build Stage
FROM rust:latest AS builder
WORKDIR /usr/src/
RUN rustup target add x86_64-unknown-linux-musl

RUN USER=root cargo new tiny-mailcatcher
WORKDIR /usr/src/tiny-mailcatcher
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo install --target x86_64-unknown-linux-musl --path .

# Bundle Stage
FROM scratch
COPY --from=builder /usr/local/cargo/bin/tiny-mailcatcher .
ENV RUST_LOG=tiny_mailcatcher=info
EXPOSE 25 80
CMD ["./tiny-mailcatcher", "--ip=0.0.0.0", "--smtp-port=25", "--http-port=80"]