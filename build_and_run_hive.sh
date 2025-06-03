#!/usr/bin/env bash
# build_and_run_hive.sh
#
# 1. Build the Reth-workspace binaries (reth, ress, adapter).
# 2. Copy them into bin/hive/clients/reth/.
# 3. Inside the Hive repo, wipe any old workspace and run the simulation.

set -euo pipefail

# Move to the repository root (where this script lives).
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

###############################################################################
# Step 3 — Build reth, ress, adapter
###############################################################################
BINARIES=(reth ress adapter)

echo "→ Building ${BINARIES[*]} ..."
for bin in "${BINARIES[@]}"; do
  cargo build --release --bin "$bin"
done

###############################################################################
# Copy the freshly built binaries into hive/clients/reth/
###############################################################################
DEST="$ROOT_DIR/bin/hive/clients/reth"
mkdir -p "$DEST"

echo "→ Copying binaries to $DEST ..."
for bin in "${BINARIES[@]}"; do
  cp "target/release/$bin" "$DEST/"
done

###############################################################################
# Step 4 — Run the Hive simulation from inside the hive repo
###############################################################################
pushd "$ROOT_DIR/bin/hive" >/dev/null

echo "→ Running Hive simulation ..."
rm -rf workspace/
./hive --sim ethereum/engine --sim.limit api --client reth

popd >/dev/null