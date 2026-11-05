.PHONY: help build run test fmt check clean

ifeq ($(OS),Windows_NT)
BINARY := readtui.exe
COPY_BINARY := copy /Y target\\release\\$(BINARY) $(BINARY)
REMOVE_BINARY := if exist $(BINARY) del /Q $(BINARY)
else
BINARY := readtui
COPY_BINARY := cp target/release/$(BINARY) ./$(BINARY)
REMOVE_BINARY := rm -f $(BINARY)
endif

help:
	@echo "readtui development commands"
	@echo "  make build  Build optimized binary and copy it to the current directory"
	@echo "  make run    Run the application"
	@echo "  make test   Run tests"
	@echo "  make fmt    Format Rust code"
	@echo "  make check  Run cargo check"
	@echo "  make clean  Remove build artifacts and the copied binary"

build:
	cargo build --release
	$(COPY_BINARY)

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
	$(REMOVE_BINARY)
