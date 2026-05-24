.PHONY: dev build start clean

dev:
	. "\$$HOME/.cargo/env" && cargo run

build:
	. "\$$HOME/.cargo/env" && cargo build --release

start:
	. "\$$HOME/.cargo/env" && cargo run --release

clean:
	. "\$$HOME/.cargo/env" && cargo clean