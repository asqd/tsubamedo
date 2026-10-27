.PHONY: test lint cli install

# Core tests: request files, variables, sending against a local server.
test:
	cargo test --workspace

lint:
	cargo clippy --workspace --all-targets -- -D warnings

# Release build of the CLI: target/release/tsubamedo.
cli:
	cargo build --release -p tsubamedo-cli

# Puts tsubamedo into ~/.cargo/bin.
install:
	cargo install --path core/tsubamedo-cli
