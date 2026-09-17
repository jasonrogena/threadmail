export RUST_BACKTRACE ?= 1
export CARGO_BUILD_ARGS ?= --verbose --release

dependencies:
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

# A regular, dynamically-linked build for local dev/testing only. The
# deployable artifact is a static musl binary and is only ever built by CI
# (release.yml/dev-image.yml, via cross): that needs a real musl C
# toolchain to compile rusqlite's bundled SQLite, which cross's own build
# images provide but this host doesn't (cross's images are amd64-only and
# don't run on this aarch64 machine either).
build: dependencies
	cargo build ${CARGO_BUILD_ARGS}
