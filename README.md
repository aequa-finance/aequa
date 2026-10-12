# aequa program

The Solana program behind USDq, sUSDq and stAEQUA. Anchor 1.2.1, one program,
id `9xhUvFQy1yAnyuZTygfK1nD6naicXQoWMR6doxFxuTbL` (`keys/aequa-program.json`). Clients build on
it through the TypeScript kit, `@aequa-finance/sdk` ([aequa-sdk](https://github.com/aequa-finance/aequa-sdk)).

## Instructions

| Group | Instructions | Signer |
|---|---|---|
| Setup | `initialize`, `create_vaults`, `create_susdq_vaults`, `add_collateral`, `set_collateral`, `set_bridge_route`, `set_token_metadata`, `set_aequa` | admin (`initialize`: the upgrade authority) |
| Governance | `set_params`, `set_roles`, `accept_admin`, `unpause`, `unpause_collateral` | admin; `accept_admin` the pending admin |
| Safety | `pause` | guardian or admin |
| USDC paths | `mint_with_usdc`, `redeem_for_usdc` | user |
| xStock paths | `mint_with_collateral`, `redeem_for_collateral` (signed Pyth Pro update) | user |
| sUSDq | `stake_usdq`, `start_usdq_cooldown`, `claim_usdq` | user |
| Hedge | `report_hedge` | reporter |
| Desk | `swap` (Jupiter, signed price), `bridge_out` (CCTP V2), `observe_pool` | keeper |
| Cranks | `distribute`, `land_usdc`, `buyback`, `cover_deficit`, `sell_slashed` | anyone |
| AEQUA | `stake_aequa`, `start_aequa_unstake`, `withdraw_aequa` | user |
| Realms | `create_voter_weight_record`, `update_voter_weight_record`, `create_max_voter_weight_record`, `update_max_voter_weight_record` | anyone |

Every path is a bit in `Config.paused` (`state.rs` `path`); `initialize` sets them all, and the
admin opens them once the configuration is checked.

## How it reads the outside world

- **Prices.** One Pyth Pro feed per ETF (SPY 1398, QQQ 1363). Every priced instruction takes a
  signed update as its first argument and the index of an ed25519 instruction that verifies it;
  the program verifies it again through the Pyth Lazer program (`verify_message`, 1 lamport).
  An update counts on a fixed-rate channel of 200 ms or faster, at most `pyth_freshness_secs`
  old, with the feed's own last update inside the same window, in a session other than closed.
  The session in the update picks the band.
- **Multipliers.** Read from the xStock mint's ScaledUiAmount extension in the same
  instruction; a scheduled change closes the collateral's paths from 24 h before to 1 h after.
- **The hedge.** `report_hedge` carries the account value, the short notionals, a market flag
  per collateral and the bridge ledger totals, stamped with HyperCore's time (`hl_time`). A
  report lands only with an `hl_time` at most a minute ahead of Solana's clock and no older than
  `report_freshness_secs`, and counts while its `hl_time` stays within that window. The xStock
  paths need a counting report with the collateral's market flag set; `distribute` needs its
  Pyth update within 20 s of the report's `hl_time`, so both legs are marked at one moment.
- **Bridge.** `bridge_out` burns buffer USDC through CCTP V2 `deposit_for_burn_with_hook` to
  Circle's CctpForwarder on HyperEVM, with the hedge account in the hook; Circle mints it and
  deposits it into CoreDepositWallet for the account, keeping at most the burn's `max_fee`.
  `bridged_out_total` books the amount less that fee. The CCTP programs, HyperEVM's domain and
  the forwarder are constants (`constants.rs` `cctp`); `set_bridge_route` names only the
  HyperCore account and the fee cap. Withdrawals come back to the landing account, the USDC
  token account of the `["landing"]` PDA, and `land_usdc` moves them into the buffer.
- **Swaps.** `swap`, `buyback` and `sell_slashed` CPI into Jupiter v6 (`route_v2` or
  `shared_accounts_route_v2`, no platform or positive-slippage fee) through token accounts of
  the `["swap_authority"]` PDA, which signs and is never writable; the program checks what
  arrived against the signed price.
- **AEQUA's price.** The keeper's `observe_pool` records the designated pool's reserves into a
  64-slot ring (`Twap`); `buyback`, `cover_deficit` and `sell_slashed` bound their fills by the
  30-minute TWAP, in dollars through the quote asset's Pyth feed (or 1:1 for a dollar quote). An
  average needs an observation from the last 5 minutes and at least ten inside its window.
- **Hourly volumes.** `buyback` spends at most `buyback_max_chunk` USDq an hour and
  `sell_slashed` sells at most that many dollars of AEQUA (at the TWAP), each through a bucket
  that refills continuously, so small calls can't use up an hour.

## Design notes

Choices the build makes:

1. One Pyth feed per ETF, with the session in each update, instead of four feeds.
2. Circle's CctpForwarder and forwarding hook carry `bridge_out` to the hedge account; there
   is no forwarder of our own on HyperEVM.
3. The landing account is the associated USDC account of the `["landing"]` PDA.
4. Swaps go through the `["swap_authority"]` transit accounts.
5. `buyback` retires the protocol vault's USDq at par for buffer USDC, then buys AEQUA with
   that USDC; half is burned, half streams to stAEQUA. It and `sell_slashed` draw on hourly
   buckets rather than a once-an-hour slot.
6. `sell_slashed` sends its USDC to the buffer and mints the matching USDq into the Reserve
   Fund.
7. The TWAP is the program's own, over constant-product reserves of the designated pool,
   observed by the keeper.
8. `initialize` pauses every path until the admin opens them.
9. With no sUSDq outstanding, its share of a distribution goes to the Reserve Fund.
10. The max voter weight is the whole stAEQUA pool.
11. The AEQUA withdraw window is fixed at 2 days.
12. Margin withdrawals from HyperCore are signed by the hedge account's own key (the council);
    the desk sends them itself only when it holds that key (`HYPER_MASTER_KEY`).
13. `cover_deficit` burns whatever the Reserve Fund holds first, instead of waiting for it to be
    empty, so nothing sent to the fund can hold the cover off.
14. The landing and transit accounts are associated accounts the setup creates (anyone could);
    the program checks them. sUSDq's two accounts come from `create_susdq_vaults`.
15. Fee-tier thresholds are whole AEQUA, scaled by the token's decimals.
16. A collateral's value in `distribute` reads its mint without the admission checks, so a
    vault still counts if its issuer later turns on a fee or a hook; the paths that move it
    keep the checks.

## Build, test, deploy

```bash
cargo build-sbf --manifest-path programs/aequa/Cargo.toml     # target/deploy/aequa.so (SBPF v0)
scripts/idl.sh <path to aequa-sdk>                             # IDL + types into the SDK
```

Tests run on Linux or WSL (LiteSVM with precompiles needs OpenSSL), against mainnet binaries
and accounts in `tests/fixtures` (`scripts/fixtures.sh` refreshes them; the Jupiter routes
were captured from Jupiter's API):

```bash
scripts/test.sh                  # from Windows: wsl --exec bash scripts/test.sh
```

| Suite | Covers |
|---|---|
| `usdc.rs` (13) | USDC paths, hourly buckets, supply ceiling, fee routing, sUSDq stake, vesting, cooldown (and claiming an ended one while starting the next, in one transaction), roles, pauses, parameter ranges, token metadata, report times, associated accounts created ahead of setup |
| `xstock.rs` (7) | Pyth Pro verification through the mainnet Lazer program (untrusted signer, tampered bytes), sessions and bands, corporate-action window, issuer pause, transfer-hook admission |
| `distribute.rs` (7) | 80 / 10 / 10, yield cap, overflow to the Reserve Fund, deficit burn and streak, `cover_deficit` and its 30-day cap, USDq sent to the fund, both legs at one moment |
| `aequa.rs` (5) | the stAEQUA lifecycle, fee tiers, voter-weight records, the keeper's observations, minimum shares |
| `bridge.rs` (3) | `bridge_out` on the mainnet CCTP V2 programs and state, the fixed route |
| `jupiter.rs` (4) | `swap` on captured Jupiter routes through Raydium CLMM, buy and sell, a paused collateral |
| `buyback.rs` (6) | `buyback` 50 / 50 and its hourly volume, `sell_slashed` on two-hop routes and its dollar bucket |

Unit tests in the program crate (23) cover the arithmetic, the Pyth payload reader, the TWAP's
freshness and coverage, the slash records and the pool head: `cargo test --manifest-path
programs/aequa/Cargo.toml --lib`.

Deploy (`scripts/deploy.sh`; the upgrade authority later moves to the vault that holds the
admin role, checked against Config):

```bash
scripts/deploy.sh <rpc-url> <deployer.json>     # builds, prints the rent, asks, deploys
```

`keys/aequa-program.json` is the keypair of the program's address: until the deploy lands,
whoever holds it can put a program at that address. Keep it private.

The program account holds about 7.11 SOL of rent at this build's size on mainnet; a deploy
writes through a buffer account of the same size first, refunded when the deploy completes.
