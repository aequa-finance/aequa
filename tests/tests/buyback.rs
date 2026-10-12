//! AEQUA's swaps: `buyback` and `sell_slashed` through Jupiter on a mainnet
//! snapshot, with JUP standing in for AEQUA (the `jupiter_aequa_*.json` routes). The pool the
//! TWAP reads is this world's own reserves, set to the price the test needs.

use aequa::{events::*, state::*};
use aequa_tests::*;
use solana_keypair::Keypair;
use solana_signer::Signer;

const JUP: u64 = 1_000_000; // 6 decimals

/// A world whose AEQUA is JUP, priced by a pool at `price` dollars, with the route loaded and the
/// TWAP's window filled.
fn world(route: &str, price: f64) -> (Env, AequaWorld, Route, Keypair) {
    let mut env = Env::new();
    env.list(SPYX, 15_000_000 * USD);
    let r = env.load_route(route);
    env.set_time(r.block_time - 37 * 60);
    // The pool: 1,000,000 JUP against `price` × 1,000,000 USDC.
    let w = env.setup_aequa(JUP_MINT, USDC, 1_000_000 * JUP, (price * 1e6 * 1e6) as u64);
    env.observe_for_twap(&w);
    env.set_time(r.block_time);
    assert_ok(&env.observe(&w));
    let cranker = Keypair::new();
    env.svm.airdrop(&cranker.pubkey(), 10_000_000_000).unwrap();
    (env, w, r, cranker)
}

fn swap_accounts(env: &Env, w: &AequaWorld) -> aequa::accounts::AequaSwap {
    aequa::accounts::AequaSwap {
        aequa_mint: w.mint,
        aequa_token_program: TOKEN,
        swap_authority: env.p.swap_authority,
        transit_usdc: env.p.transit_usdc,
        transit_aequa: Pdas::transit(&w.mint, &TOKEN),
        jupiter_program: aequa::jupiter::JUPITER,
    }
}

fn buyback(env: &mut Env, w: &AequaWorld, r: &Route, caller: &Keypair) -> TxResult {
    let accounts = aequa::accounts::Buyback {
        caller: caller.pubkey(),
        config: env.p.config,
        aequa_pool: env.p.aequa_pool,
        twap: w.twap,
        usdq_mint: env.p.usdq_mint,
        usdc_mint: USDC,
        protocol: env.p.protocol,
        buffer: env.p.buffer,
        aequa_pool_vault: w.pool_vault,
        token_program: TOKEN,
        swap: swap_accounts(env, w),
        pyth: pyth_accounts(),
    };
    let mut program_ix = ix(accounts, aequa::instruction::Buyback { message: vec![], ed25519_index: 0, amount: r.in_amount, route: r.data.clone() });
    program_ix.accounts.extend(r.accounts.iter().cloned());
    env.send(&[compute_budget(1_400_000), program_ix], &[caller])
}

fn sell_slashed(env: &mut Env, w: &AequaWorld, r: &Route, caller: &Keypair) -> TxResult {
    let accounts = aequa::accounts::SellSlashed {
        caller: caller.pubkey(),
        config: env.p.config,
        reserve: env.p.reserve,
        aequa_pool: env.p.aequa_pool,
        twap: w.twap,
        usdq_mint: env.p.usdq_mint,
        usdc_mint: USDC,
        slash_sale: w.slash_sale,
        buffer: env.p.buffer,
        reserve_fund: env.p.reserve_fund,
        token_program: TOKEN,
        swap: swap_accounts(env, w),
        pyth: pyth_accounts(),
    };
    let mut program_ix = ix(accounts, aequa::instruction::SellSlashed { message: vec![], ed25519_index: 0, amount: r.in_amount, route: r.data.clone() });
    program_ix.accounts.extend(r.accounts.iter().cloned());
    env.send(&[compute_budget(1_400_000), program_ix], &[caller])
}

/// USDq into the protocol vault and USDC into the buffer, as distributions and fees would.
fn fund_protocol(env: &mut Env, usdq: u64) {
    let u = env.user(usdq * 2);
    assert_ok(&env.mint_with_usdc(&u, usdq * 2, 0));
    let protocol = env.p.protocol;
    let mut data = vec![3u8];
    data.extend_from_slice(&usdq.to_le_bytes());
    let ix = solana_instruction::Instruction {
        program_id: TOKEN,
        accounts: vec![
            solana_instruction::AccountMeta::new(u.usdq, false),
            solana_instruction::AccountMeta::new(protocol, false),
            solana_instruction::AccountMeta::new_readonly(u.key(), true),
        ],
        data,
    };
    assert_ok(&env.send(&[ix], &[&u.kp]));
}

fn staker(env: &mut Env, w: &AequaWorld, amount: u64) -> Keypair {
    let kp = Keypair::new();
    env.svm.airdrop(&kp.pubkey(), 10_000_000_000).unwrap();
    let acc = env.create_ata(&kp.pubkey(), &w.mint, &TOKEN);
    env.mint_to(&w.mint, &acc, amount);
    assert_ok(&env.stake_aequa(w, &kp, &acc, amount));
    kp
}

#[test]
fn buys_back_aequa_burns_half_and_streams_half() {
    let r0 = Env::new().load_route("aequa_buy");
    // The fill's own price, so it sits within the 1% bound of the TWAP.
    let fill = r0.in_amount as f64 / r0.out_amount as f64;
    let (mut env, w, r, cranker) = world("aequa_buy", fill * 0.995);
    fund_protocol(&mut env, r.in_amount);
    staker(&mut env, &w, 100_000 * JUP);
    let supply = env.supply(&env.p.usdq_mint);
    let buffer = env.balance(&env.p.buffer);
    let aequa_supply = env.supply(&w.mint);

    let res = buyback(&mut env, &w, &r, &cranker);
    assert_ok(&res);
    let e = &events::<BoughtBack>(&res)[0];
    assert_eq!(e.usdq_in, r.in_amount);
    assert!(e.aequa_out as f64 >= r.out_amount as f64 * 0.995, "{} vs quote {}", e.aequa_out, r.out_amount);
    assert_eq!(e.burned + e.streamed, e.aequa_out);
    assert_eq!(e.burned, e.aequa_out - e.streamed);
    assert_eq!(e.streamed, e.aequa_out - e.aequa_out / 2);
    // The protocol vault's USDq retired at par against buffer USDC.
    assert_eq!(env.balance(&env.p.protocol), 0);
    assert_eq!(env.supply(&env.p.usdq_mint), supply - r.in_amount);
    assert_eq!(env.balance(&env.p.buffer), buffer - r.in_amount);
    // Half burned, half streaming to stakers over 7 days.
    assert_eq!(env.supply(&w.mint), aequa_supply - e.burned);
    let pool: AequaPool = env.account(&env.p.aequa_pool);
    assert_eq!((pool.stream.amount, pool.stream.end - pool.stream.start), (e.streamed, 7 * DAY));
    assert_eq!((pool.usdq_spent, pool.aequa_burned, pool.aequa_streamed, pool.buybacks), (r.in_amount, e.burned, e.streamed, 1));
    assert_eq!(env.balance(&w.pool_vault), 100_000 * JUP + e.streamed);
    assert_eq!(env.balance(&env.p.transit_usdc), 0);
    assert_eq!(env.balance(&Pdas::transit(&w.mint, &TOKEN)), 0);

    // At most `buyback_max_chunk` ($5,000, this route's size) an hour.
    fund_protocol(&mut env, r.in_amount);
    assert_err(&buyback(&mut env, &w, &r, &cranker), "CapacityExceeded");
}

#[test]
fn small_buybacks_spend_only_their_own_share_of_the_hour() {
    let r0 = Env::new().load_route("aequa_buy");
    let fill = r0.in_amount as f64 / r0.out_amount as f64;
    let (mut env, w, r, cranker) = world("aequa_buy", fill);
    fund_protocol(&mut env, r.in_amount);
    // Calls since the hour began left one base unit short of this chunk.
    let (pool, now) = (env.p.aequa_pool, env.now());
    env.edit::<AequaPool>(&pool, |p| p.buyback_bucket = Bucket { available: r.in_amount - 1, updated_at: now });
    assert_err(&buyback(&mut env, &w, &r, &cranker), "CapacityExceeded");
    // A second later the bucket has refilled past it ($5,000 an hour, continuously).
    env.warp(1);
    assert_ok(&buyback(&mut env, &w, &r, &cranker));
}

#[test]
fn buyback_fills_must_sit_near_the_twap() {
    let r0 = Env::new().load_route("aequa_buy");
    let fill = r0.in_amount as f64 / r0.out_amount as f64;
    // The pool reads 2% under the fill: the fill is outside the 1% bound.
    let (mut env, w, r, cranker) = world("aequa_buy", fill * 0.98);
    fund_protocol(&mut env, r.in_amount);
    assert_err(&buyback(&mut env, &w, &r, &cranker), "TwapBound");
    // More than the protocol vault holds.
    let (mut env, w, r, cranker) = world("aequa_buy", fill);
    fund_protocol(&mut env, r.in_amount - 1);
    assert_err(&buyback(&mut env, &w, &r, &cranker), "InsufficientFunds");
}

#[test]
fn buyback_without_stakers_burns_it_all() {
    let r0 = Env::new().load_route("aequa_buy");
    let fill = r0.in_amount as f64 / r0.out_amount as f64;
    let (mut env, w, r, cranker) = world("aequa_buy", fill);
    fund_protocol(&mut env, r.in_amount);
    let res = buyback(&mut env, &w, &r, &cranker);
    assert_ok(&res);
    let e = &events::<BoughtBack>(&res)[0];
    assert_eq!((e.streamed, e.burned), (0, e.aequa_out));
}

#[test]
fn slash_sale_closes_the_shortfall_and_funds_the_reserve_fund_with_the_rest() {
    let price = snapshot_price("jup");
    let (mut env, w, r, cranker) = world("aequa_sell", price);
    env.mint_to(&w.mint, &w.slash_sale, r.in_amount);
    let reserve = env.p.reserve;
    env.edit::<Reserve>(&reserve, |res| res.deficit = 500 * USD);
    let buffer = env.balance(&env.p.buffer);
    let fund = env.balance(&env.p.reserve_fund);

    let res = sell_slashed(&mut env, &w, &r, &cranker);
    assert_ok(&res);
    let pool: AequaPool = env.account(&env.p.aequa_pool);
    assert!(pool.slash_bucket.available < 5_000 * USD, "the sale drew on the hour's dollars");
    let e = &events::<SlashSold>(&res)[0];
    assert_eq!(e.aequa_in, r.in_amount);
    assert!(e.usdq_out > 500 * USD, "the sale raised {}", e.usdq_out);
    assert_eq!((e.burned, e.to_reserve_fund), (500 * USD, e.usdq_out - 500 * USD));
    assert_eq!(env.balance(&env.p.buffer), buffer + e.usdq_out);
    assert_eq!(env.balance(&env.p.reserve_fund), fund + e.to_reserve_fund);
    assert_eq!(env.reserve().deficit, 0);
    assert_eq!(env.balance(&w.slash_sale), 0);

    // A pool reading well above the fill: the sale is more than 5% under it.
    let (mut env, w, r, cranker) = world("aequa_sell", price * 1.2);
    env.mint_to(&w.mint, &w.slash_sale, r.in_amount);
    assert_err(&sell_slashed(&mut env, &w, &r, &cranker), "TwapBound");
}

#[test]
fn slash_sales_sell_at_most_a_chunk_of_dollars_an_hour() {
    let price = snapshot_price("jup");
    let (mut env, w, r, cranker) = world("aequa_sell", price);
    env.mint_to(&w.mint, &w.slash_sale, r.in_amount);
    let reserve = env.p.reserve;
    env.edit::<Reserve>(&reserve, |res| res.deficit = 500 * USD);
    // The hour's dollars are spent: the sale waits, however large the vault.
    let (pool, now) = (env.p.aequa_pool, env.now());
    env.edit::<AequaPool>(&pool, |p| p.slash_bucket = Bucket { available: 0, updated_at: now });
    assert_err(&sell_slashed(&mut env, &w, &r, &cranker), "CapacityExceeded");
}
