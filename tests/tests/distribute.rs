//! `distribute` and its split and rounding, Reserve Fund overflow, the yield
//! cap, sessions and windows that hold it back, the deficit burn, and `cover_deficit`.

use aequa::{events::*, math, state::*};
use aequa_tests::*;
use solana_keypair::Keypair;
use solana_signer::Signer;

const H: i64 = 3_600;
const AEQUA: u64 = 1_000_000;

fn world() -> (Env, Keypair) {
    let mut env = Env::new();
    env.list(SPYX, 15_000_000 * USD);
    env.list(QQQX, 12_800_000 * USD);
    let cranker = Keypair::new();
    env.svm.airdrop(&cranker.pubkey(), 10_000_000_000).unwrap();
    (env, cranker)
}

fn run(env: &mut Env, cranker: &Keypair) -> TxResult {
    let e = env.both_feeds(0);
    env.distribute(cranker, e)
}

#[test]
fn waits_for_its_interval_and_live_sources() {
    let (mut env, cranker) = world();
    assert_ok(&env.report_now(0, 0, 0, 0b11));
    assert_err(&run(&mut env, &cranker), "NotDue");
    env.warp(8 * H);
    assert_err(&run(&mut env, &cranker), "HedgeNotLive");
    assert_ok(&env.report_now(0, 0, 0, 0b11));
    // The weekend: every collateral's session is closed.
    let closed = env.both_feeds(4);
    assert_err(&env.distribute(&cranker, closed), "CollateralNotLive");
    // One collateral without a price.
    let t = env.now();
    let spy_only = pyth_envelope(&env.pyth, t, 3, &[Feed::at(SPY, 665.0, 0, t)]);
    assert_err(&env.distribute(&cranker, spy_only), "CollateralNotLive");
    // Nothing above the cushion: it runs and mints nothing.
    let r = run(&mut env, &cranker);
    assert_ok(&r);
    assert_eq!(events::<Distributed>(&r)[0].amount, 0);
    assert_eq!(env.reserve().last_distribution, env.now());
}

#[test]
fn mints_the_surplus_capped_and_split() {
    let (mut env, cranker) = world();
    let u = env.user(1_000_000 * USD);
    assert_ok(&env.mint_with_usdc(&u, 1_000_000 * USD, 0));
    assert_ok(&env.stake_usdq(&u, 500_000 * USD));
    let fund0 = env.balance(&env.p.reserve_fund);

    env.warp(8 * H);
    // The hedge account earned $10,000: NAV is $1,010,000 against $1,000,000 of supply.
    assert_ok(&env.report_now(10_000 * USD as i64, 0, 0, 0b11));
    let r = run(&mut env, &cranker);
    assert_ok(&r);
    let e = &events::<Distributed>(&r)[0];
    // 5 bps a day for 8 hours of $1,000,000: 166.666666 USDq, under the $7,500 surplus.
    assert_eq!(e.amount, 166_666_666);
    assert_eq!(e.nav, 1_010_000 * USD as i64);
    assert_eq!(e.surplus, 7_500 * USD as i64);
    assert_eq!((e.to_susdq, e.to_reserve_fund, e.to_protocol), (133_333_334, 16_666_666, 16_666_666));
    assert_eq!(env.balance(&env.p.reserve_fund), fund0 + 16_666_666);
    assert_eq!(env.balance(&env.p.protocol), 16_666_666);
    assert_eq!(env.supply(&env.p.usdq_mint), 1_000_000 * USD + 166_666_666);

    // The sUSDq part vests over 8 hours.
    let v = env.usdq_vault();
    assert_eq!((v.vesting.amount, v.vesting.end - v.vesting.start), (133_333_334, 8 * H));
    env.warp(4 * H);
    let v = env.usdq_vault();
    assert_eq!(v.base + math::vested(v.vesting.amount, v.vesting.start, v.vesting.end, env.now()), 500_000 * USD + 66_666_667);
    // A stake now pays the half-vested rate.
    let late = env.user(0);
    let usdq = 10_000 * USD;
    transfer(&mut env, &u.kp, &u.usdq, &late.usdq, usdq);
    assert_ok(&env.stake_usdq(&late, usdq));
    let shares = env.balance(&late.susdq);
    assert_eq!(shares, math::shares_for(usdq, 500_000 * USD + 66_666_667, 500_000 * USD));
}

#[test]
fn fund_overflow_and_no_stakers() {
    let (mut env, cranker) = world();
    let u = env.user(1_000_000 * USD);
    assert_ok(&env.mint_with_usdc(&u, 1_000_000 * USD, 0));
    // A full Reserve Fund: its target is 1% and it already holds more than that.
    let mut p = env.config().params;
    p.reserve_fund_target_bps = 100;
    assert_ok(&env.admin_ix(aequa::instruction::SetParams { params: p }));
    let fund = env.p.reserve_fund;
    env.give_usdq(&u, &fund, 20_000 * USD);

    env.warp(8 * H);
    assert_ok(&env.report_now(50_000 * USD as i64, 0, 0, 0b11));
    let r = run(&mut env, &cranker);
    assert_ok(&r);
    let e = &events::<Distributed>(&r)[0];
    // No sUSDq yet, so sUSDq's part would join the fund's; the fund is full, so both go to
    // the protocol vault.
    assert_eq!(e.to_susdq, 0);
    assert_eq!(e.to_reserve_fund, 0);
    assert_eq!(e.to_protocol, e.amount);
}

#[test]
fn shortfalls_burn_the_fund_then_count_the_streak() {
    let (mut env, cranker) = world();
    let u = env.user(1_000_000 * USD);
    assert_ok(&env.mint_with_usdc(&u, 1_000_000 * USD, 0));
    assert_eq!(env.balance(&env.p.reserve_fund), 1_000 * USD);

    env.warp(8 * H);
    assert_ok(&env.report_now(-500 * USD as i64, 0, 0, 0b11));
    let r = run(&mut env, &cranker);
    assert_ok(&r);
    let e = &events::<Distributed>(&r)[0];
    assert_eq!((e.amount, e.burned, e.deficit_streak), (0, 500 * USD, 0));
    assert_eq!(env.balance(&env.p.reserve_fund), 500 * USD);
    assert_eq!(env.supply(&env.p.usdq_mint), 999_500 * USD);

    for (loss, burned, streak) in [(2_000, 500, 1), (2_000, 0, 2), (2_000, 0, 3)] {
        env.warp(8 * H);
        assert_ok(&env.report_now(-(loss * USD as i64), 0, 0, 0b11));
        let r = run(&mut env, &cranker);
        assert_ok(&r);
        let e = &events::<Distributed>(&r)[0];
        assert_eq!((e.burned, e.deficit_streak), (burned * USD, streak), "burned {} streak {}", e.burned, e.deficit_streak);
    }
    assert_eq!(env.reserve().deficit, 1_000 * USD);

    // The backstop: AEQUA moves from the stake pool to the sale vault, priced on the pool's TWAP.
    let (aequa, quote) = (Keypair::new().pubkey(), Keypair::new().pubkey());
    env.create_mint(aequa, 6);
    env.create_mint(quote, 6);
    let w = env.setup_aequa(aequa, quote, 100_000_000 * AEQUA, 5_000 * USD);
    let staker = Keypair::new();
    env.svm.airdrop(&staker.pubkey(), 10_000_000_000).unwrap();
    let acc = env.create_ata(&staker.pubkey(), &aequa, &TOKEN);
    env.mint_to(&aequa, &acc, 300_000_000 * AEQUA);
    assert_ok(&env.stake_aequa(&w, &staker, &acc, 300_000_000 * AEQUA));
    assert_ok(&env.observe(&w));
    assert_err(&cover(&mut env, &w, &cranker), "TwapNotReady");
    env.observe_for_twap(&w);

    let r = cover(&mut env, &w, &cranker);
    assert_ok(&r);
    // $1,000 at $0.00005 per AEQUA, grossed up for the 5% sale bound.
    let needed = 20_000_000 * AEQUA * 10_000 / 9_500;
    assert_eq!(events::<DeficitCovered>(&r)[0].aequa, needed);
    assert_eq!(env.balance(&w.slash_sale), needed);
    let pool: AequaPool = env.account(&env.p.aequa_pool);
    assert_eq!(pool.base, 300_000_000 * AEQUA - needed);
    assert_eq!(pool.total_shares, 300_000_000 * AEQUA, "shares stay; AEQUA per share falls");
    assert_err(&cover(&mut env, &w, &cranker), "NotDue");

    // The guardian can stop it.
    let accounts = aequa::accounts::Pause { config: env.p.config, signer: env.guardian.pubkey(), collateral: None };
    let guardian = clone(&env.guardian);
    assert_ok(&env.send(&[ix(accounts, aequa::instruction::Pause { paths: path::COVER_DEFICIT })], &[&guardian]));
    env.warp(8 * H);
    assert_ok(&env.report_now(-2_000 * USD as i64, 0, 0, 0b11));
    assert_ok(&run(&mut env, &cranker));
    assert_err(&cover(&mut env, &w, &cranker), "Paused");
}

#[test]
fn slash_cap_bounds_each_window() {
    let (mut env, cranker) = world();
    let u = env.user(10_000_000 * USD);
    for _ in 0..3 {
        assert_ok(&env.mint_with_usdc(&u, 1_000_000 * USD, 0));
        env.warp(H);
    }
    // A deep shortfall with the Reserve Fund gone.
    for _ in 0..4 {
        env.warp(8 * H);
        assert_ok(&env.report_now(-400_000 * USD as i64, 0, 0, 0b11));
        assert_ok(&run(&mut env, &cranker));
    }
    assert_eq!(env.balance(&env.p.reserve_fund), 0);
    assert!(env.reserve().deficit_streak >= 3);

    let (aequa, quote) = (Keypair::new().pubkey(), Keypair::new().pubkey());
    env.create_mint(aequa, 6);
    env.create_mint(quote, 6);
    let w = env.setup_aequa(aequa, quote, 100_000_000 * AEQUA, 5_000 * USD);
    let staker = Keypair::new();
    env.svm.airdrop(&staker.pubkey(), 10_000_000_000).unwrap();
    let acc = env.create_ata(&staker.pubkey(), &aequa, &TOKEN);
    env.mint_to(&aequa, &acc, 1_000_000_000 * AEQUA);
    assert_ok(&env.stake_aequa(&w, &staker, &acc, 1_000_000_000 * AEQUA));
    env.observe_for_twap(&w);

    // $400,000 would take far more than 30% of the pool: the cap binds.
    let r = cover(&mut env, &w, &cranker);
    assert_ok(&r);
    assert_eq!(events::<DeficitCovered>(&r)[0].aequa, 300_000_000 * AEQUA);
    // Within the same 30 days nothing more moves.
    env.warp(8 * H);
    assert_ok(&env.report_now(-400_000 * USD as i64, 0, 0, 0b11));
    assert_ok(&run(&mut env, &cranker));
    // Eight hours without observations: the old price no longer counts.
    assert_err(&cover(&mut env, &w, &cranker), "TwapNotReady");
    env.observe_for_twap(&w);
    assert_err(&cover(&mut env, &w, &cranker), "SlashCapReached");
}

fn cover(env: &mut Env, w: &AequaWorld, caller: &Keypair) -> TxResult {
    let accounts = aequa::accounts::CoverDeficit {
        caller: caller.pubkey(),
        config: env.p.config,
        reserve: env.p.reserve,
        aequa_pool: env.p.aequa_pool,
        twap: w.twap,
        mint: w.mint,
        aequa_pool_vault: w.pool_vault,
        slash_sale: w.slash_sale,
        usdq_mint: env.p.usdq_mint,
        reserve_fund: env.p.reserve_fund,
        token_program: TOKEN,
        usdq_token_program: TOKEN,
        pyth: pyth_accounts(),
    };
    env.send(&[ix(accounts, aequa::instruction::CoverDeficit { message: vec![], ed25519_index: 0 })], &[caller])
}

fn transfer(env: &mut Env, owner: &Keypair, from: &solana_address::Address, to: &solana_address::Address, amount: u64) {
    let mut data = vec![3u8];
    data.extend_from_slice(&amount.to_le_bytes());
    let ix = solana_instruction::Instruction {
        program_id: TOKEN,
        accounts: vec![
            solana_instruction::AccountMeta::new(*from, false),
            solana_instruction::AccountMeta::new(*to, false),
            solana_instruction::AccountMeta::new_readonly(owner.pubkey(), true),
        ],
        data,
    };
    assert_ok(&env.send(&[ix], &[owner]));
}

trait FundHelpers {
    fn give_usdq(&mut self, u: &User, to: &solana_address::Address, amount: u64);
}

impl FundHelpers for Env {
    /// Moves USDq the user minted into another account.
    fn give_usdq(&mut self, u: &User, to: &solana_address::Address, amount: u64) {
        let have = self.balance(&u.usdq);
        assert!(have >= amount);
        transfer(self, &u.kp, &u.usdq, to, amount);
    }
}

#[test]
fn usdq_sent_to_the_reserve_fund_burns_before_the_stake_and_holds_nothing_off() {
    let (mut env, cranker) = world();
    let u = env.user(1_000_000 * USD);
    assert_ok(&env.mint_with_usdc(&u, 1_000_000 * USD, 0));
    for _ in 0..4 {
        env.warp(8 * H);
        assert_ok(&env.report_now(-3_000 * USD as i64, 0, 0, 0b11));
        assert_ok(&run(&mut env, &cranker));
    }
    assert_eq!(env.balance(&env.p.reserve_fund), 0);
    let deficit = env.reserve().deficit;
    assert!(deficit > 0 && env.reserve().deficit_streak >= 3);

    let (aequa, quote) = (Keypair::new().pubkey(), Keypair::new().pubkey());
    env.create_mint(aequa, 6);
    env.create_mint(quote, 6);
    let w = env.setup_aequa(aequa, quote, 100_000_000 * AEQUA, 5_000 * USD);
    let staker = Keypair::new();
    env.svm.airdrop(&staker.pubkey(), 10_000_000_000).unwrap();
    let acc = env.create_ata(&staker.pubkey(), &aequa, &TOKEN);
    env.mint_to(&aequa, &acc, 300_000_000 * AEQUA);
    assert_ok(&env.stake_aequa(&w, &staker, &acc, 300_000_000 * AEQUA));
    env.observe_for_twap(&w);

    // One base unit of USDq lands in the Reserve Fund just before the cover.
    let fund = env.p.reserve_fund;
    env.give_usdq(&u, &fund, 1);
    let r = cover(&mut env, &w, &cranker);
    assert_ok(&r);
    let e = &events::<DeficitCovered>(&r)[0];
    assert_eq!((e.fund_burned, e.deficit), (1, deficit - 1));
    assert!(e.aequa > 0);
    assert_eq!(env.balance(&env.p.reserve_fund), 0);
}

#[test]
fn both_legs_are_marked_at_one_moment() {
    let (mut env, cranker) = world();
    let u = env.user(1_000_000 * USD);
    assert_ok(&env.mint_with_usdc(&u, 1_000_000 * USD, 0));
    env.warp(8 * H);
    assert_ok(&env.report_now(0, 0, 0, 0b11));
    // A price 30 s after the report's HyperCore time: the short isn't in the same picture.
    env.warp(30);
    assert_err(&run(&mut env, &cranker), "HedgeNotLive");
    assert_ok(&env.report_now(0, 0, 0, 0b11));
    assert_ok(&run(&mut env, &cranker));
}
