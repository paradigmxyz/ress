# Set environment variable
export CARGO_MAKE_EXTEND_WORKSPACE_MAKEFILE := "true"



# Runs cargo deny to check for any vulnerabilities in the project
deny:
    cargo deny --all-features check all

# Run rustfmt to check the code formatting without making changes
format:
    cargo fmt -- --check

# Clean up the project by removing the target directory
clean:
    cargo clean

# Run clippy to catch common mistakes and improve your Rust code
clippy:
    cargo clippy --all-targets --all-features -- -Dwarnings

# Generate documentation for the project
docs:
    cargo doc --no-deps

# Execute all unit tests in the workspace
test:
    cargo llvm-cov nextest

# Build hivetest binary for `x86_64-unknown-linux-musl`, used for mac -> linux cross compile
build-cross-hive:
    CFLAGS=-march=x86_64_v4 cargo +stable zigbuild --bins --target x86_64-unknown-linux-musl --profile hivetests

# Build hivetest binary, run on linux
build-hive:
    cargo build --bins --profile hivetests
    cp target/hivetests/{adapter,ress,reth} ../hive/clients/reth/ 

# Run the entire CI pipeline including format, clippy, docs, and test checks
ci: format clippy docs deny test
    @echo "CI flow completed"
