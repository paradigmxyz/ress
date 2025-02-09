# Set environment variable
export CARGO_MAKE_EXTEND_WORKSPACE_MAKEFILE := "true"

# Run rustfmt to check the code formatting without making changes
fmt:
    cargo +nightly fmt -- --check

# Run clippy to catch common mistakes and improve your Rust code
clippy:
    cargo +nightly clippy --all-targets --all-features -- -Dwarnings

# Execute all unit tests in the workspace
test:
    cargo llvm-cov nextest

# Generate documentation for the project
docs:
    cargo doc --no-deps

# Runs cargo deny to check for any vulnerabilities in the project
deny:
    cargo deny --all-features check all

# Clean up the project by removing the target directory
clean:
    cargo clean

# Build hivetest binary for `x86_64-unknown-linux-musl`, used for mac -> linux cross compile
build-cross-hive:
    CFLAGS=-march=x86_64_v4 cargo +stable zigbuild --bins --target x86_64-unknown-linux-musl --profile hivetests

# Build hivetest binary, run on linux
build-hive:
    cargo build --bins --profile hivetests
    cp target/hivetests/{adapter,ress,reth} ../hive/clients/reth/ 

# Run the entire CI pipeline including fmt, clippy, docs, and test checks
ci: fmt clippy test docs deny
    @echo "CI flow completed"
