#!/usr/bin/env bash
# Dumps the mainnet programs and accounts the LiteSVM suites load (tests/fixtures/), and prints
# each program's sha256 so a stale dump shows. Read-only; uses the public mainnet RPC unless
# RPC_URL is set.
set -euo pipefail
cd "$(dirname "$0")/../tests/fixtures"
RPC="${RPC_URL:-https://api.mainnet-beta.solana.com}"

program() { solana program dump -u "$RPC" "$1" "$2.so" >/dev/null && echo "$2.so $(sha256sum "$2.so" | cut -c1-16)"; }
account() { solana account -u "$RPC" "$1" --output json-compact -o "$2.json" >/dev/null && echo "$2.json"; }

program TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA spl_token
program TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb spl_token_2022
program ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL spl_ata
program pytd2yyk641x7ak7mkaasSJVXh6YYZnC7wTmtgAyxPt pyth_lazer
program metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s mpl_token_metadata
program CCTPV2Sm4AdWt5296sk4P66VBZ7bEhcARwFaaS9YPbeC cctp_message_transmitter_v2
program CCTPV2vPZJS2u2BBsUoscuikbYjnpFmbFsvVuJdgUMQe cctp_token_messenger_minter_v2

account EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v usdc_mint
account XsoCS1TfEyfFhfvj8EtZ528L3CaKBDBRqRapnBbDF2W spyx_mint
account Xs8S1uUs1zvS2p7iwtsG3b6fkhpvmwz4GYU3gWAmWHZ qqqx_mint
account 3rdJbqfnagQ4yx9HXJViD4zc4xpiSqmFsKpPuSCQVyQL pyth_lazer_storage
account Gx4MBPb1vqZLJajZmsKLg8fGw9ErhoKsR8LeKcCKFyak pyth_lazer_treasury
# CCTP V2 state (Solana side of the route to HyperEVM, domain 19)
account W1k5ijkaSTo5iA5zChNpfzcy796fLhkBxfmJuR8W8HU cctp_message_transmitter
account AawthJCGRmggpfv9MMWV6Jmo9cue4gL9wUZgRBShg58W cctp_token_messenger
account 3WG7G3vPxz4a6bdBb9Xic2YL4QM84ru1YcVYskcweVHY cctp_remote_token_messenger_19
account E1bQJ8eMMn3zmeSewW3HQ8zmJr7KR75JonbwAtWx2bux cctp_token_minter
account CRBBbuLCyrkQy4dCTHxqstSmDQv4ajBeUVb9qUdMVaP1 cctp_local_token_usdc
date -u +%Y-%m-%dT%H:%M:%SZ > dumped_at.txt
