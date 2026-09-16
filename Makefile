export RUST_BACKTRACE ?= 1
export CARGO_BUILD_ARGS ?= --verbose --release
# Deployed builds are static musl binaries (see .cargo/config.toml); this
# defaults to the host's own arch so `make build` works the same on an
# aarch64 or x86_64 dev machine without extra flags.
export TARGET ?= $(shell uname -m)-unknown-linux-musl

dependencies:
	rustup target add ${TARGET}
	rustup component add clippy
	rustup component add rustfmt

clean:
	cargo clean

test: dependencies
	cargo clippy --all-targets --all-features -- -D warnings
	cargo fmt --all -- --check
	cargo test

fmt: dependencies
	cargo fmt

build: dependencies
	cargo build --target ${TARGET} ${CARGO_BUILD_ARGS}
