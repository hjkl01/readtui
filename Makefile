.PHONY: help build run test fmt check clean

help:
	@echo "readtui development commands"
	@echo "  make build  Build optimized binary"
	@echo "  make run    Run the application"
	@echo "  make test   Run tests"
	@echo "  make fmt    Format Rust code"
	@echo "  make check  Run cargo check"
	@echo "  make clean  Remove build artifacts"

build:
	cargo build --release

run:
	cargo run

test:
	cargo test

fmt:
	cargo fmt --all

check:
	cargo check

clean:
	cargo clean
