//! The keeper's `swap` through Jupiter on a mainnet snapshot: Jupiter, Raydium CLMM and the pool
//! accounts as they stood when a live route through the swap authority's transit accounts was
//! captured (`tests/fixtures/jupiter_*.json`).

use aequa::events::Swapped;
use aequa_tests::*;
use solana_keypair::Keypair;
use solana_signer::Signer;

/// A world with a captured route loaded.
fn world(name: &str) -> (Env, Route) {
    let mut env = Env::new();
    env.list(SPYX, 15_000_000 * USD);
    let r = env.load_route(name);
    env.set_time(r.block_time);
    (env, r)
}

fn spy_price() -> f64 {
    snapshot_price("spyx")
}

fn swap(env: &mut Env, signer: &Keypair, buy: bool, amount_in: u64, r: &Route, spy: f64) -> TxResult {
    let t = env.now();
    let envelope = pyth_envelope(&env.pyth, t, 3, &[Feed::at(SPY, spy, 0, t)]);
    let accounts = aequa::accounts::Swap {
        keeper: signer.pubkey(),
        config: env.p.config,
        collateral: Pdas::collateral(&SPYX),
        mint: SPYX,
        vault: Pdas::collateral_vault(&SPYX),
        usdc_mint: USDC,
        buffer: env.p.buffer,
        swap_authority: env.p.swap_authority,
        transit_usdc: env.p.transit_usdc,
        transit_xstock: Pdas::transit(&SPYX, &TOKEN_2022),
        token_program: TOKEN,
        xstock_token_program: TOKEN_2022,
        jupiter_program: aequa::jupiter::JUPITER,
        pyth: pyth_accounts(),
    };
    let budget = compute_budget(1_400_000);
    let ed = ed25519_ix(&envelope, 2);
    let mut program_ix = ix(accounts, aequa::instruction::Swap { message: envelope, ed25519_index: 1, buy, amount_in, route: r.data.clone() });
    program_ix.accounts.extend(r.accounts.iter().cloned());
    env.send(&[budget, ed, program_ix], &[signer])
}

#[test]
fn buys_spyx_through_jupiter_at_the_pyth_price() {
    let (mut env, r) = world("buy");
    let u = env.user(20_000 * USD);
    assert_ok(&env.mint_with_usdc(&u, 20_000 * USD, 0));
    let vault = Pdas::collateral_vault(&SPYX);
    let buffer = env.balance(&env.p.buffer);

    let keeper = clone(&env.keeper);
    let res = swap(&mut env, &keeper, true, r.in_amount, &r, spy_price());
    assert_ok(&res);
    let e = &events::<Swapped>(&res)[0];
    assert!(e.buy);
    assert_eq!(e.amount_in, r.in_amount);
    assert!(e.amount_out >= e.min_out, "{} < {}", e.amount_out, e.min_out);
    // Jupiter's quote held: the pool hasn't moved since the snapshot.
    assert!(e.amount_out as f64 >= r.out_amount as f64 * 0.995, "{} vs quote {}", e.amount_out, r.out_amount);
    assert_eq!(env.balance(&vault), e.amount_out);
    assert_eq!(env.balance(&env.p.buffer), buffer - r.in_amount);
    // The transit accounts hold nothing once the swap is done.
    assert_eq!(env.balance(&env.p.transit_usdc), 0);
    assert_eq!(env.balance(&Pdas::transit(&SPYX, &TOKEN_2022)), 0);
}

#[test]
fn sells_spyx_through_jupiter_at_the_pyth_price() {
    let (mut env, r) = world("sell");
    let vault = Pdas::collateral_vault(&SPYX);
    env.mint_to(&SPYX, &vault, r.in_amount);
    let buffer = env.balance(&env.p.buffer);
    let keeper = clone(&env.keeper);
    let res = swap(&mut env, &keeper, false, r.in_amount, &r, spy_price());
    assert_ok(&res);
    let e = &events::<Swapped>(&res)[0];
    assert!(!e.buy);
    assert!(e.amount_out >= e.min_out);
    assert_eq!(env.balance(&env.p.buffer), buffer + e.amount_out);
    assert_eq!(env.balance(&vault), 0);
    assert_eq!(env.balance(&env.p.transit_usdc), 0);
}

#[test]
fn refuses_fills_below_the_pyth_price_and_foreign_routes() {
    let (mut env, r) = world("buy");
    let u = env.user(20_000 * USD);
    assert_ok(&env.mint_with_usdc(&u, 20_000 * USD, 0));
    let keeper = clone(&env.keeper);
    // At a Pyth price 2% under the pool, the fill would be 2% short of fair: refused.
    assert_err(&swap(&mut env, &keeper, true, r.in_amount, &r, spy_price() * 0.98), "SwapBelowMinimum");
    // A route for another amount.
    assert_err(&swap(&mut env, &keeper, true, r.in_amount - 1, &r, spy_price()), "InvalidRoute");
    // A route with a platform fee.
    let mut fee = Env::new().load_route("buy");
    fee.data[27] = 10;
    assert_err(&swap(&mut env, &keeper, true, fee.in_amount, &fee, spy_price()), "InvalidRoute");
    // Only the keeper.
    let stranger = Keypair::new();
    env.svm.airdrop(&stranger.pubkey(), 10_000_000_000).unwrap();
    assert_err(&swap(&mut env, &stranger, true, r.in_amount, &r, spy_price()), "Unauthorized");
    // A purchase past the spot cap.
    let accounts = aequa::accounts::SetCollateral { config: env.p.config, admin: env.admin.pubkey(), collateral: Pdas::collateral(&SPYX) };
    let c: aequa::state::Collateral = env.account(&Pdas::collateral(&SPYX));
    let args = aequa::instructions::ListingArgs { feed: c.feed, bands_bps: c.bands_bps, spot_cap: 5_000 * USD, hourly_cap: c.hourly_cap, perp: c.perp, perp_asset: c.perp_asset };
    let admin = clone(&env.admin);
    assert_ok(&env.send(&[ix(accounts, aequa::instruction::SetCollateral { args })], &[&admin]));
    assert_err(&swap(&mut env, &keeper, true, r.in_amount, &r, spy_price()), "SpotCapExceeded");
}

#[test]
fn a_paused_collateral_stops_the_keeper_too() {
    let (mut env, r) = world("buy");
    let u = env.user(20_000 * USD);
    assert_ok(&env.mint_with_usdc(&u, 20_000 * USD, 0));
    let accounts = aequa::accounts::Pause { config: env.p.config, signer: env.guardian.pubkey(), collateral: Some(Pdas::collateral(&SPYX)) };
    let guardian = clone(&env.guardian);
    assert_ok(&env.send(&[ix(accounts, aequa::instruction::Pause { paths: 0 })], &[&guardian]));
    let keeper = clone(&env.keeper);
    assert_err(&swap(&mut env, &keeper, true, r.in_amount, &r, spy_price()), "Paused");
}
