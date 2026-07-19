build:
    cargo build --workspace
test:
    cargo test --workspace
lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
run *ARGS:
    cargo run -p kasane-cli -- {{ARGS}}
