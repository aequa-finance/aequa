//! Setup, the USDC paths, buckets, fee routing, the supply ceiling, sUSDq
//! stake, vesting and cooldown, roles and pauses.

use aequa::{events::*, params::*, state::*};
use aequa_tests::*;
use solana_keypair::Keypair;
use solana_signer::Signer;

#[test]
fn only_the_upgrade_authority_initializes_once() {
    let mut env = Env::bare();
    let stranger = Keypair::new();
    env.svm.airdrop(&stranger.pubkey(), 10_000_000_000).unwrap();
    assert_err(&env.try_initialize(Params::launch(), Some(&stranger)), "Unauthorized");

    let mut bad = Params::launch();
    bad.base_fee_bps = 31;
    assert_err(&env.try_initialize(bad, None), "ParamOutOfRange");

    env.initialize(Params::launch());
    assert!(env.try_initialize(Params::launch(), None).is_err());

    let c = env.config();
    assert_eq!(c.paused, path::ALL, "every path waits for the admin");
    assert_eq!(c.usdq_mint, env.p.usdq_mint);
    assert_eq!(c.params.spot_share_bps(), 6_750);
}

#[test]
fn mint_and_redeem_with_usdc() {
    let mut env = Env::new();
    // A ceiling to mint under: list SPYx with a $15M cap.
    list(&mut env, SPYX, 15_000_000 * USD);
    let u = env.user(100_000 * USD);

    let r = env.mint_with_usdc(&u, 10_000 * USD, 0);
    assert_ok(&r);
    // 10 bps: 9,990 USDq to the user, 10 USDq minted to the Reserve Fund.
    assert_eq!(env.balance(&u.usdq), 9_990 * USD);
    assert_eq!(env.balance(&env.p.reserve_fund), 10 * USD);
    assert_eq!(env.balance(&env.p.buffer), 10_000 * USD);
    assert_eq!(env.supply(&env.p.usdq_mint), 10_000 * USD);
    let e = events::<Minted>(&r);
    assert_eq!((e[0].usdq_out, e[0].fee, e[0].fee_to, e[0].fee_bps), (9_990 * USD, 10 * USD, TO_RESERVE_FUND, 10));

    // Redeem: the net is burned, the fee moves to the Reserve Fund, USDC leaves the buffer.
    let r = env.redeem_for_usdc(&u, 1_000 * USD, 0);
    assert_ok(&r);
    assert_eq!(env.balance(&u.usdq), 8_990 * USD);
    assert_eq!(env.balance(&u.usdc), 90_999 * USD);
    assert_eq!(env.balance(&env.p.reserve_fund), 11 * USD);
    assert_eq!(env.balance(&env.p.buffer), 9_001 * USD);
    assert_eq!(env.supply(&env.p.usdq_mint), 9_001 * USD);

    // The user's minimum is respected.
    assert_err(&env.mint_with_usdc(&u, 1_000 * USD, 1_000 * USD), "SlippageExceeded");
    assert_err(&env.mint_with_usdc(&u, 0, 0), "ZeroAmount");
    assert_err(&env.redeem_for_usdc(&u, 100_000 * USD, 0), "InsufficientFunds");
}

#[test]
fn redeem_is_bounded_by_the_buffer() {
    let mut env = Env::new();
    list(&mut env, SPYX, 15_000_000 * USD);
    let u = env.user(10_000 * USD);
    assert_ok(&env.mint_with_usdc(&u, 10_000 * USD, 0));
    // The desk moved most of the buffer out: simulate by bridging it away in this world.
    let buffer_now = env.balance(&env.p.buffer);
    drain(&mut env, buffer_now - 500 * USD);
    assert_err(&env.redeem_for_usdc(&u, 1_000 * USD, 0), "BufferInsufficient");
    assert_ok(&env.redeem_for_usdc(&u, 500 * USD, 0));
}

#[test]
fn hourly_bucket_draws_and_refills() {
    let mut env = Env::new();
    list(&mut env, SPYX, 15_000_000 * USD);
    let u = env.user(3_000_000 * USD);
    assert_ok(&env.mint_with_usdc(&u, 1_000_000 * USD, 0));
    assert_err(&env.mint_with_usdc(&u, 1 * USD, 0), "CapacityExceeded");
    env.warp(1_800);
    // Half an hour refills half the cap.
    assert_err(&env.mint_with_usdc(&u, 500_001 * USD, 0), "CapacityExceeded");
    assert_ok(&env.mint_with_usdc(&u, 500_000 * USD, 0));
    env.warp(7_200);
    assert_ok(&env.mint_with_usdc(&u, 1_000_000 * USD, 0));
}

#[test]
fn supply_ceiling_holds() {
    let mut env = Env::new();
    // $1M of spot cap allows $1M / 0.675 of USDq.
    list(&mut env, SPYX, 1_000_000 * USD);
    let ceiling = env.config().supply_cap();
    assert_eq!(ceiling, 1_481_481_481_481);
    let u = env.user(2_000_000 * USD);
    assert_ok(&env.mint_with_usdc(&u, 1_000_000 * USD, 0));
    env.warp(3_600);
    assert_err(&env.mint_with_usdc(&u, 481_482 * USD, 0), "SupplyCapExceeded");
    assert_ok(&env.mint_with_usdc(&u, 481_481 * USD, 0));
}

#[test]
fn fees_go_to_the_protocol_vault_once_the_reserve_fund_is_full() {
    let mut env = Env::new();
    list(&mut env, SPYX, 15_000_000 * USD);
    let mut p = env.config().params;
    p.reserve_fund_target_bps = 100;
    assert_ok(&env.admin_ix(aequa::instruction::SetParams { params: p }));
    let u = env.user(1_000_000 * USD);
    // 1,000 USDq: the fund's 1 USDq is under 1% of supply, so the fee goes to the fund.
    let r = env.mint_with_usdc(&u, 1_000 * USD, 0);
    assert_eq!(events::<Minted>(&r)[0].fee_to, TO_RESERVE_FUND);
    assert_eq!(env.balance(&env.p.reserve_fund), 1 * USD);
    // Redeeming 990 leaves 10.99 USDq of supply: the fund's 1 USDq is over 1% of it, so
    // this fee goes to the protocol vault.
    let r = env.redeem_for_usdc(&u, 990 * USD, 0);
    assert_ok(&r);
    let e = &events::<Redeemed>(&r)[0];
    assert_eq!((e.fee, e.fee_to), (990_000, TO_PROTOCOL));
    assert_eq!(env.balance(&env.p.protocol), 990_000);
    assert_eq!(env.balance(&env.p.reserve_fund), 1 * USD);
}

#[test]
fn susdq_stake_cooldown_and_claim() {
    let mut env = Env::new();
    list(&mut env, SPYX, 15_000_000 * USD);
    let a = env.user(20_000 * USD);
    assert_ok(&env.mint_with_usdc(&a, 20_000 * USD, 0));
    let usdq = env.balance(&a.usdq);

    // 1:1 for the first stake.
    assert_ok(&env.stake_usdq(&a, 10_000 * USD));
    assert_eq!(env.balance(&a.susdq), 10_000 * USD);
    assert_eq!(env.usdq_vault().base, 10_000 * USD);

    // USDq sent straight to the assets account doesn't move the rate.
    let assets = env.p.susdq_assets;
    transfer_usdq(&mut env, &a, &assets, 1_000 * USD);
    let b = env.user(0);
    transfer_usdq(&mut env, &a, &b.usdq, 1_000 * USD);
    assert_ok(&env.stake_usdq(&b, 1_000 * USD));
    assert_eq!(env.balance(&b.susdq), 1_000 * USD);

    // Unstake: shares burn, USDq moves to the silo for 7 days.
    let r = env.start_usdq_cooldown(&a, 4_000 * USD);
    assert_ok(&r);
    let e = &events::<UsdqCooldownStarted>(&r)[0];
    assert_eq!((e.usdq, e.unlock_at), (4_000 * USD, START + 7 * DAY));
    assert_eq!(env.balance(&env.p.susdq_silo), 4_000 * USD);
    assert_err(&env.claim_usdq(&a), "CooldownActive");

    // A second unstake adds to the locked amount and restarts the 7 days.
    env.warp(3 * DAY);
    assert_ok(&env.start_usdq_cooldown(&a, 1_000 * USD));
    env.warp(5 * DAY);
    assert_err(&env.claim_usdq(&a), "CooldownActive");
    env.warp(2 * DAY);
    let before = env.balance(&a.usdq);
    assert_ok(&env.claim_usdq(&a));
    assert_eq!(env.balance(&a.usdq) - before, 5_000 * USD);
    assert!(!env.exists(&Pdas::cooldown(&a.key())), "the cooldown account closes");
    assert_eq!(env.balance(&a.susdq), 5_000 * USD);
    assert_eq!(usdq - 10_000 * USD - 2_000 * USD + 5_000 * USD, env.balance(&a.usdq));
}

#[test]
fn an_ended_cooldown_is_claimed_and_a_new_one_started_in_one_transaction() {
    // What the SDK's unstake sends once a cooldown has ended: the claim closes the cooldown
    // account and the new cooldown opens it again.
    let mut env = Env::new();
    list(&mut env, SPYX, 15_000_000 * USD);
    let a = env.user(20_000 * USD);
    assert_ok(&env.mint_with_usdc(&a, 20_000 * USD, 0));
    assert_ok(&env.stake_usdq(&a, 10_000 * USD));
    assert_ok(&env.start_usdq_cooldown(&a, 4_000 * USD));
    env.warp(7 * DAY);
    let before = env.balance(&a.usdq);
    let ixs = [env.claim_usdq_ix(&a), env.start_usdq_cooldown_ix(&a, 1_000 * USD)];
    let r = env.send(&ixs, &[&a.kp]);
    assert_ok(&r);
    assert_eq!(env.balance(&a.usdq) - before, 4_000 * USD, "the ended cooldown paid out");
    let e = &events::<UsdqCooldownStarted>(&r)[0];
    assert_eq!((e.usdq, e.cooling), (1_000 * USD, 1_000 * USD), "the new cooldown holds only the new USDq");
}

#[test]
fn hedge_report_and_landing() {
    let mut env = Env::new();
    list(&mut env, SPYX, 15_000_000 * USD);
    list(&mut env, QQQX, 12_800_000 * USD);
    let now_ms = env.now() * 1_000;
    let mut args = report(now_ms, 2_250_000 * USD);
    args.market_live = 0xff;
    assert_ok(&env.report(args.clone()));
    let r = env.reserve();
    assert_eq!(r.report.market_live, 0b11, "bits past the listed collateral are dropped");
    assert_eq!(r.report.reported_at, env.now());
    // An older report never replaces a newer one.
    assert_err(&env.report(report(now_ms - 1, 1)), "ReportOutOfOrder");
    // HyperCore's time runs at most a minute ahead of Solana's clock: a time in the wrong unit
    // (µs) or far ahead can't land and lock every later report out.
    assert_err(&env.report(report(now_ms * 1_000, 1)), "ReportTimeOutOfRange");
    assert_err(&env.report(report(now_ms + 61_000, 1)), "ReportTimeOutOfRange");
    assert_ok(&env.report(report(now_ms + 59_000, 1)));
    // Nor older than the window a report counts for.
    env.warp(300);
    assert_err(&env.report(report(now_ms + 60_000, 1)), "ReportTimeOutOfRange");
    // Only the reporter reports.
    let accounts = aequa::accounts::ReportHedge { config: env.p.config, reporter: env.keeper.pubkey(), reserve: env.p.reserve };
    let keeper = clone(&env.keeper);
    let r = env.send(&[ix(accounts, aequa::instruction::ReportHedge { args: report(env.now() * 1_000, 1) })], &[&keeper]);
    assert_err(&r, "Unauthorized");

    // USDC that CCTP minted into the landing account moves to the buffer.
    let caller = Keypair::new();
    env.svm.airdrop(&caller.pubkey(), 1_000_000_000).unwrap();
    assert_err(&env.land_usdc(&caller), "NothingToMove");
    let landing = env.p.landing;
    env.mint_to(&USDC, &landing, 250 * USD);
    let r = env.land_usdc(&caller);
    assert_ok(&r);
    assert_eq!(env.balance(&env.p.buffer), 250 * USD);
    assert_eq!(env.reserve().landed_total, 250 * USD);
    assert_eq!(events::<Landed>(&r)[0].amount, 250 * USD);
}

#[test]
fn guardian_pauses_admin_unpauses() {
    let mut env = Env::new();
    list(&mut env, SPYX, 15_000_000 * USD);
    let u = env.user(10_000 * USD);
    let accounts = aequa::accounts::Pause { config: env.p.config, signer: env.guardian.pubkey(), collateral: None };
    let guardian = clone(&env.guardian);
    assert_ok(&env.send(&[ix(accounts, aequa::instruction::Pause { paths: path::MINT_USDC })], &[&guardian]));
    assert_err(&env.mint_with_usdc(&u, 1_000 * USD, 0), "Paused");

    // The guardian can't unpause.
    let accounts = aequa::accounts::AdminOnly { config: env.p.config, admin: env.guardian.pubkey() };
    let r = env.send(&[ix(accounts, aequa::instruction::Unpause { paths: path::MINT_USDC })], &[&guardian]);
    assert!(r.is_err());
    assert_ok(&env.admin_ix(aequa::instruction::Unpause { paths: path::MINT_USDC }));
    assert_ok(&env.mint_with_usdc(&u, 1_000 * USD, 0));

    // A stranger can't pause.
    let stranger = Keypair::new();
    env.svm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();
    let accounts = aequa::accounts::Pause { config: env.p.config, signer: stranger.pubkey(), collateral: None };
    assert_err(&env.send(&[ix(accounts, aequa::instruction::Pause { paths: path::ALL })], &[&stranger]), "Unauthorized");
}

#[test]
fn params_stay_in_range() {
    let mut env = Env::new();
    let mut p = env.config().params;
    p.usdc_hourly_cap = 2_000_000 * USD;
    assert_ok(&env.admin_ix(aequa::instruction::SetParams { params: p }));
    p.usdc_hourly_cap = 4_000_001 * USD;
    assert_err(&env.admin_ix(aequa::instruction::SetParams { params: p }), "ParamOutOfRange");
    let mut p = env.config().params;
    p.pyth_freshness_secs = 31;
    assert_err(&env.admin_ix(aequa::instruction::SetParams { params: p }), "ParamOutOfRange");
    let mut p = env.config().params;
    p.yield_cap_bps_per_day = 0;
    assert_err(&env.admin_ix(aequa::instruction::SetParams { params: p }), "ParamOutOfRange");

    // A new admin takes over only once it accepts.
    let next = Keypair::new();
    env.svm.airdrop(&next.pubkey(), 1_000_000_000).unwrap();
    let c = env.config();
    assert_ok(&env.admin_ix(aequa::instruction::SetRoles { guardian: c.guardian, keeper: c.keeper, reporter: c.reporter, pending_admin: next.pubkey() }));
    assert_eq!(env.config().admin, env.admin.pubkey());
    let accounts = aequa::accounts::AcceptAdmin { config: env.p.config, pending_admin: next.pubkey() };
    assert_ok(&env.send(&[ix(accounts, aequa::instruction::AcceptAdmin {})], &[&next]));
    assert_eq!(env.config().admin, next.pubkey());
}

// Helpers

/// Lists a collateral with placeholder feed ids and the launch bands.
pub fn list(env: &mut Env, mint: solana_address::Address, spot_cap: u64) {
    let accounts = aequa::accounts::AddCollateral {
        payer: env.payer.pubkey(),
        config: env.p.config,
        admin: env.admin.pubkey(),
        mint,
        collateral: Pdas::collateral(&mint),
        vault: Pdas::collateral_vault(&mint),
        swap_authority: env.p.swap_authority,
        transit: Pdas::transit(&mint, &TOKEN_2022),
        token_program: TOKEN_2022,
        system_program: SYSTEM,
    };
    let mut perp = [0u8; 24];
    perp[..9].copy_from_slice(b"xyz:SP500");
    let args = aequa::instructions::ListingArgs {
        feed: 1398,
        bands_bps: [5, 15, 15, 25],
        spot_cap,
        hourly_cap: 500_000 * USD,
        perp,
        perp_asset: 110_000,
    };
    let (payer, admin) = (clone(&env.payer), clone(&env.admin));
    let transit = env.ata_ix(&env.p.swap_authority, &mint, &TOKEN_2022);
    let r = env.send(&[transit, ix(accounts, aequa::instruction::AddCollateral { args })], &[&payer, &admin]);
    assert_ok(&r);
}

fn report(hl_time: i64, account_value: u64) -> aequa::instructions::ReportArgs {
    aequa::instructions::ReportArgs {
        account_value: account_value as i64,
        short_notional: [0; 8],
        market_live: 0b11,
        credited_total: 0,
        withdrawn_total: 0,
        hl_time,
    }
}

fn transfer_usdq(env: &mut Env, from: &User, to: &solana_address::Address, amount: u64) {
    let mut data = vec![3u8];
    data.extend_from_slice(&amount.to_le_bytes());
    let ix = solana_instruction::Instruction {
        program_id: TOKEN,
        accounts: vec![
            solana_instruction::AccountMeta::new(from.usdq, false),
            solana_instruction::AccountMeta::new(*to, false),
            solana_instruction::AccountMeta::new_readonly(from.key(), true),
        ],
        data,
    };
    assert_ok(&env.send(&[ix], &[&from.kp]));
}

/// Moves buffer USDC out of this world, as `bridge_out` would.
fn drain(env: &mut Env, amount: u64) {
    let buffer = env.p.buffer;
    let mut acc = env.svm.get_account(&buffer).unwrap();
    let left = u64::from_le_bytes(acc.data[64..72].try_into().unwrap()) - amount;
    acc.data[64..72].copy_from_slice(&left.to_le_bytes());
    env.svm.set_account(buffer, acc).unwrap();
    // Keep the mint's supply consistent with the accounts.
    let mut mint = env.svm.get_account(&USDC).unwrap();
    let supply = u64::from_le_bytes(mint.data[36..44].try_into().unwrap()) - amount;
    mint.data[36..44].copy_from_slice(&supply.to_le_bytes());
    env.svm.set_account(USDC, mint).unwrap();
}

#[test]
fn token_metadata_names_usdq_and_susdq() {
    let mut env = Env::new();
    for (mint, name, symbol) in [(env.p.usdq_mint, "USDq", "USDq"), (env.p.susdq_mint, "Staked USDq", "sUSDq")] {
        let metadata = solana_address::Address::find_program_address(&[b"metadata", TOKEN_METADATA.as_ref(), mint.as_ref()], &TOKEN_METADATA).0;
        let set = |env: &mut Env, uri: &str| {
            let accounts = aequa::accounts::SetTokenMetadata {
                payer: env.payer.pubkey(),
                config: env.p.config,
                admin: env.admin.pubkey(),
                usdq_vault: env.p.usdq_vault,
                mint,
                metadata,
                token_metadata_program: TOKEN_METADATA,
                system_program: SYSTEM,
            };
            let args = aequa::instructions::TokenMetadataArgs { name: name.into(), symbol: symbol.into(), uri: uri.into() };
            let (payer, admin) = (clone(&env.payer), clone(&env.admin));
            env.send(&[ix(accounts, aequa::instruction::SetTokenMetadata { args })], &[&payer, &admin])
        };
        assert_ok(&set(&mut env, "https://aequa.example/usdq.json"));
        let data = env.svm.get_account(&metadata).unwrap().data;
        // key, update authority, mint, then the name.
        assert_eq!(&data[1..33], env.p.config.as_ref(), "the Config PDA holds the update authority");
        assert_eq!(&data[33..65], mint.as_ref());
        let len = u32::from_le_bytes(data[65..69].try_into().unwrap()) as usize;
        assert_eq!(std::str::from_utf8(&data[69..69 + len]).unwrap().trim_end_matches('\0'), name);
        // A second call updates in place.
        assert_ok(&set(&mut env, "https://aequa.example/v2.json"));
        assert!(String::from_utf8_lossy(&env.svm.get_account(&metadata).unwrap().data).contains("v2.json"));
    }
}

#[test]
fn associated_accounts_created_first_hold_nothing_up() {
    // Anyone can create the landing and transit accounts ahead of the setup; it goes on.
    let mut env = Env::bare();
    env.initialize(Params::launch());
    let (landing_authority, swap_authority) = (env.p.landing_authority, env.p.swap_authority);
    env.create_ata(&landing_authority, &USDC, &TOKEN);
    env.create_ata(&swap_authority, &USDC, &TOKEN);
    env.create_ata(&swap_authority, &SPYX, &TOKEN_2022);
    assert_ok(&env.try_create_vaults());
    env.admin_ix(aequa::instruction::Unpause { paths: path::ALL }).unwrap();
    list(&mut env, SPYX, 15_000_000 * USD);
    assert_eq!(env.config().transit_usdc, env.p.transit_usdc);
}
