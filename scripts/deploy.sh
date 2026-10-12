#!/usr/bin/env bash
# Builds the program and deploys it under its fixed id (keys/aequa-program.json). The deployer
# keypair pays the program account's rent (printed before anything is sent) and becomes the
# upgrade authority; the protocol's setup then runs with the same keypair, and later hands
# the upgrade authority to the vault that accepted the admin role, checked against the
# program's own Config.
#
#   scripts/deploy.sh <rpc-url> <deployer.json>     build, price, confirm, deploy
#
# Needs the Solana CLI (Agave 3.x) with cargo-build-sbf. Upgrades run the same command, signed by
# the upgrade authority.
set -euo pipefail
cd "$(dirname "$0")/.."
url="${1:?rpc url}"
payer="${2:?deployer keypair path}"
program_id="9xhUvFQy1yAnyuZTygfK1nD6naicXQoWMR6doxFxuTbL"

[ "$(solana address -k keys/aequa-program.json)" = "$program_id" ] || { echo "keys/aequa-program.json isn't $program_id" >&2; exit 1; }
cargo build-sbf --manifest-path programs/aequa/Cargo.toml
bytes=$(wc -c < target/deploy/aequa.so)
echo "program: $program_id, $bytes bytes"
echo "deployer: $(solana address -k "$payer"), $(solana balance -k "$payer" --url "$url")"
echo "program data rent: $(solana rent $((bytes + 45)) --url "$url" | head -1)"
read -r -p "deploy to $url? [y/N] " answer
[ "$answer" = "y" ] || exit 1
solana program deploy target/deploy/aequa.so --program-id keys/aequa-program.json \
  --keypair "$payer" --url "$url" --use-rpc --with-compute-unit-price 100000 --max-sign-attempts 60
solana program show "$program_id" --url "$url"
