//! AEQUA, the parts without a swap: setup, stAEQUA positions through cooldown
//! and window, the fee tiers, the voter-weight plugin and the pool's TWAP.

use aequa::{events::*, instructions::VoterWeightRecord, state::*};
use aequa_tests::*;
use solana_address::Address as Pubkey;
use solana_keypair::Keypair;
use solana_signer::Signer;

const AEQUA: u64 = 1_000_000; // 6 decimals

struct World {
    env: Env,
    w: AequaWorld,
}

fn world() -> World {
    let mut env = Env::new();
    usdc_list(&mut env);
    let aequa = Keypair::new().pubkey();
    let quote = Keypair::new().pubkey();
    env.create_mint(aequa, 6);
    env.create_mint(quote, 6);
    // 100M AEQUA against $5,000 of a dollar quote: $0.00005 per AEQUA.
    let w = env.setup_aequa(aequa, quote, 100_000_000 * AEQUA, 5_000 * USD);
    World { env, w }
}

fn holder(world: &mut World, aequa: u64) -> (Keypair, Pubkey) {
    let kp = Keypair::new();
    world.env.svm.airdrop(&kp.pubkey(), 10_000_000_000).unwrap();
    let mint = world.w.mint;
    let acc = world.env.create_ata(&kp.pubkey(), &mint, &TOKEN);
    if aequa > 0 {
        world.env.mint_to(&mint, &acc, aequa);
    }
    (kp, acc)
}

fn stake(world: &World, owner: &Pubkey) -> AequaStake {
    world.env.account(&Pdas::aequa_stake(owner))
}

#[test]
fn positions_cool_open_a_window_and_return() {
    let mut world = world();
    let (a, a_acc) = holder(&mut world, 1_000_000 * AEQUA);
    let w = clone_world(&world.w);

    let r = world.env.stake_aequa(&w, &a, &a_acc, 400_000 * AEQUA);
    assert_ok(&r);
    assert_eq!(events::<AequaStaked>(&r)[0].shares, 400_000 * AEQUA);

    // AEQUA sent straight to the vault doesn't move the rate.
    let vault = w.pool_vault;
    let (b, b_acc) = holder(&mut world, 200_000 * AEQUA);
    world.env.mint_to(&w.mint, &vault, 50_000 * AEQUA);
    assert_ok(&world.env.stake_aequa(&w, &b, &b_acc, 100_000 * AEQUA));
    assert_eq!(stake(&world, &b.pubkey()).shares, 100_000 * AEQUA);

    // Unstaking cools the whole position: no more stake, no withdraw before the window.
    assert_err(&world.env.withdraw_aequa(&w, &a, &a_acc), "WindowNotOpen");
    let r = world.env.start_aequa_unstake(&a);
    assert_ok(&r);
    let e = &events::<AequaUnstakeStarted>(&r)[0];
    assert_eq!(e.window_opens, START + 10 * DAY);
    assert_eq!(e.window_closes, START + 12 * DAY);
    assert_err(&world.env.stake_aequa(&w, &a, &a_acc, 1 * AEQUA), "PositionCooling");
    assert_err(&world.env.start_aequa_unstake(&a), "PositionCooling");
    world.env.warp(10 * DAY - 1);
    assert_err(&world.env.withdraw_aequa(&w, &a, &a_acc), "WindowNotOpen");

    // The window: withdraw the whole position.
    world.env.warp(1);
    let before = world.env.balance(&a_acc);
    let r = world.env.withdraw_aequa(&w, &a, &a_acc);
    assert_ok(&r);
    assert_eq!(world.env.balance(&a_acc) - before, 400_000 * AEQUA);
    assert!(!world.env.exists(&Pdas::aequa_stake(&a.pubkey())));
    assert_eq!(world.env.account::<AequaPool>(&world.env.p.aequa_pool).total_shares, 100_000 * AEQUA);

    // A window that passes unused leaves the position active again.
    assert_ok(&world.env.start_aequa_unstake(&b));
    world.env.warp(12 * DAY + 1);
    assert_err(&world.env.withdraw_aequa(&w, &b, &b_acc), "WindowNotOpen");
    assert_ok(&world.env.stake_aequa(&w, &b, &b_acc, 1_000 * AEQUA));
    assert_eq!(stake(&world, &b.pubkey()).cooldown_start, 0);
    assert_ok(&world.env.start_aequa_unstake(&b));
}

#[test]
fn active_stake_lowers_the_fee() {
    let mut world = world();
    let (a, a_acc) = holder(&mut world, 2_000_000 * AEQUA);
    let w = clone_world(&world.w);
    let u = user_from(&mut world.env, &a, 100_000 * USD);

    let fee_of = |env: &mut Env, u: &User| events::<Minted>(&env.mint_with_usdc(u, 1_000 * USD, 0))[0].fee_bps;
    assert_eq!(fee_of(&mut world.env, &u), 10);
    assert_ok(&world.env.stake_aequa(&w, &a, &a_acc, 100_000 * AEQUA));
    assert_eq!(fee_of(&mut world.env, &u), 5, "exactly the threshold reaches the tier");
    assert_ok(&world.env.stake_aequa(&w, &a, &a_acc, 900_000 * AEQUA));
    assert_eq!(fee_of(&mut world.env, &u), 2);
    // A cooling position has no tier.
    assert_ok(&world.env.start_aequa_unstake(&a));
    assert_eq!(fee_of(&mut world.env, &u), 10);
    // Another holder's position doesn't count for this wallet.
    let (b, b_acc) = holder(&mut world, 2_000_000 * AEQUA);
    assert_ok(&world.env.stake_aequa(&w, &b, &b_acc, 2_000_000 * AEQUA));
    let accounts = aequa::accounts::MintWithUsdc {
        user: u.key(),
        config: world.env.p.config,
        usdq_mint: world.env.p.usdq_mint,
        usdc_mint: USDC,
        user_usdc: u.usdc,
        user_usdq: u.usdq,
        buffer: world.env.p.buffer,
        reserve_fund: world.env.p.reserve_fund,
        protocol: world.env.p.protocol,
        aequa_stake: Some(Pdas::aequa_stake(&b.pubkey())),
        aequa_pool: Some(world.env.p.aequa_pool),
        token_program: TOKEN,
    };
    let r = world.env.send(&[ix(accounts, aequa::instruction::MintWithUsdc { amount: 1_000 * USD, min_out: 0 })], &[&u.kp]);
    assert!(r.is_err(), "the stake account must be the signer's own");
}

#[test]
fn voter_weight_follows_active_stake() {
    let mut world = world();
    let (a, a_acc) = holder(&mut world, 500_000 * AEQUA);
    let w = clone_world(&world.w);
    let realm = Keypair::new().pubkey();
    let record = pda(&[aequa::constants::seeds::VOTER_WEIGHT, realm.as_ref(), w.mint.as_ref(), a.pubkey().as_ref()]);
    let max = pda(&[aequa::constants::seeds::MAX_VOTER_WEIGHT, realm.as_ref(), w.mint.as_ref()]);

    let payer = clone(&world.env.payer);
    let create = aequa::accounts::CreateVoterWeightRecord { payer: payer.pubkey(), config: world.env.p.config, voter: a.pubkey(), record, system_program: SYSTEM };
    assert_ok(&world.env.send(&[ix(create, aequa::instruction::CreateVoterWeightRecord { realm })], &[&payer]));
    let create_max = aequa::accounts::CreateMaxVoterWeightRecord { payer: payer.pubkey(), config: world.env.p.config, record: max, system_program: SYSTEM };
    assert_ok(&world.env.send(&[ix(create_max, aequa::instruction::CreateMaxVoterWeightRecord { realm })], &[&payer]));

    assert_ok(&world.env.stake_aequa(&w, &a, &a_acc, 300_000 * AEQUA));
    let update = |env: &mut Env, stake: Option<Pubkey>| {
        let accounts = aequa::accounts::UpdateVoterWeightRecord { config: env.p.config, aequa_pool: env.p.aequa_pool, record, aequa_stake: stake };
        let payer = clone(&env.payer);
        assert_ok(&env.send(&[ix(accounts, aequa::instruction::UpdateVoterWeightRecord { action: None, target: None })], &[&payer]));
        env.account::<VoterWeightRecord>(&record)
    };
    let stake_pda = Pdas::aequa_stake(&a.pubkey());
    let r = update(&mut world.env, Some(stake_pda));
    assert_eq!(r.voter_weight, 300_000 * AEQUA);
    assert_eq!(r.governing_token_owner, a.pubkey());
    assert_eq!(r.voter_weight_expiry, Some(world.env.svm.get_sysvar::<solana_clock::Clock>().slot));
    // The record's bytes are what spl-governance reads: its discriminator comes first.
    let raw = world.env.svm.get_account(&record).unwrap().data;
    assert_eq!(&raw[..8], &[46, 249, 155, 75, 153, 248, 116, 9]);

    assert_ok(&world.env.start_aequa_unstake(&a));
    assert_eq!(update(&mut world.env, Some(stake_pda)).voter_weight, 0, "cooling positions have no votes");

    let accounts = aequa::accounts::UpdateMaxVoterWeightRecord { config: world.env.p.config, aequa_pool: world.env.p.aequa_pool, record: max };
    assert_ok(&world.env.send(&[ix(accounts, aequa::instruction::UpdateMaxVoterWeightRecord {})], &[&payer]));
    let raw = world.env.svm.get_account(&max).unwrap().data;
    assert_eq!(&raw[..8], &[157, 95, 242, 151, 16, 98, 26, 118]);
    assert_eq!(world.env.account::<aequa::instructions::MaxVoterWeightRecord>(&max).max_voter_weight, 300_000 * AEQUA);
}

#[test]
fn pool_observations_build_the_twap() {
    let mut world = world();
    let w = clone_world(&world.w);
    assert_ok(&world.env.observe(&w));
    assert_err(&world.env.observe(&w), "NotDue");
    world.env.warp(30);
    // Only the keeper observes, so no one picks the moments the average is built from.
    let stranger = Keypair::new();
    world.env.svm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();
    assert_err(&world.env.observe_as(&w, &stranger), "Unauthorized");
    assert_ok(&world.env.observe(&w));
    let t: Twap = world.env.account(&w.twap);
    assert_eq!(t.count, 2);
    // $5,000 against 100M AEQUA: 0.00005 quote per AEQUA at 1e12.
    assert_eq!(t.last_price, 50_000_000);
}

// Helpers

fn clone_world(w: &AequaWorld) -> AequaWorld {
    AequaWorld {
        mint: w.mint,
        quote_mint: w.quote_mint,
        pool: w.pool,
        pool_aequa_vault: w.pool_aequa_vault,
        pool_quote_vault: w.pool_quote_vault,
        pool_vault: w.pool_vault,
        slash_sale: w.slash_sale,
        twap: w.twap,
    }
}

/// The USDq accounts of a wallet that already exists, funded with USDC.
fn user_from(env: &mut Env, kp: &Keypair, usdc: u64) -> User {
    let k = kp.pubkey();
    let u = User {
        usdc: env.create_ata(&k, &USDC, &TOKEN),
        usdq: env.create_ata(&k, &env.p.usdq_mint.clone(), &TOKEN),
        susdq: env.create_ata(&k, &env.p.susdq_mint.clone(), &TOKEN),
        kp: clone(kp),
    };
    env.mint_to(&USDC, &u.usdc, usdc);
    u
}

/// One listing so the supply ceiling allows minting.
fn usdc_list(env: &mut Env) {
    let mint = SPYX;
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
    let args = aequa::instructions::ListingArgs {
        feed: 1398,
        bands_bps: [5, 15, 15, 25],
        spot_cap: 15_000_000 * USD,
        hourly_cap: 500_000 * USD,
        perp: [0; 24],
        perp_asset: 0,
    };
    let (payer, admin) = (clone(&env.payer), clone(&env.admin));
    let transit = env.ata_ix(&env.p.swap_authority, &mint, &TOKEN_2022);
    assert_ok(&env.send(&[transit, ix(accounts, aequa::instruction::AddCollateral { args })], &[&payer, &admin]));
}

#[test]
fn staking_takes_a_minimum_of_shares() {
    let mut world = world();
    let (a, a_acc) = holder(&mut world, 1_000 * AEQUA);
    let w = clone_world(&world.w);
    // The first stake mints shares one for one; asking for more fails.
    assert_err(&world.env.stake_aequa_min(&w, &a, &a_acc, 1_000 * AEQUA, 1_000 * AEQUA + 1), "SlippageExceeded");
    assert_ok(&world.env.stake_aequa_min(&w, &a, &a_acc, 1_000 * AEQUA, 1_000 * AEQUA));
}
