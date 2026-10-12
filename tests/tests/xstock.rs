//! Collateral vaults, admission checks, Pyth Pro verification through the
//! mainnet Lazer program, the multiplier, sessions and bands, and the xStock paths.

use aequa::{events::*, math, state::*};
use aequa_tests::*;
use solana_address::Address as Pubkey;
use std::sync::atomic::{AtomicI64, Ordering};
use solana_keypair::Keypair;
use solana_signer::Signer;

const X: u64 = 100_000_000; // one xStock, 8 decimals
const SPY_PRICE: f64 = 665.1234;

struct XUser {
    u: User,
    spyx: Pubkey,
}

fn world() -> (Env, XUser) {
    let mut env = Env::new();
    for (mint, cap) in [(SPYX, 15_000_000 * USD), (QQQX, 12_800_000 * USD)] {
        list(&mut env, mint, cap);
    }
    live_report(&mut env);
    let u = env.user(1_000_000 * USD);
    let spyx = env.create_ata(&u.key(), &SPYX, &TOKEN_2022);
    env.mint_to(&SPYX, &spyx, 2_000 * X);
    (env, XUser { u, spyx })
}

fn list(env: &mut Env, mint: Pubkey, spot_cap: u64) {
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
        feed: if mint == SPYX { SPY } else { QQQ },
        bands_bps: [5, 15, 15, 25],
        spot_cap,
        hourly_cap: 500_000 * USD,
        perp: [0; 24],
        perp_asset: 0,
    };
    let (payer, admin) = (clone(&env.payer), clone(&env.admin));
    let transit = env.ata_ix(&env.p.swap_authority, &mint, &TOKEN_2022);
    assert_ok(&env.send(&[transit, ix(accounts, aequa::instruction::AddCollateral { args })], &[&payer, &admin]));
}

static HL_TIME: AtomicI64 = AtomicI64::new(1);

/// A fresh report with both perps in session.
fn live_report(env: &mut Env) {
    let hl_time = HL_TIME.fetch_add(1, Ordering::Relaxed);
    let args = aequa::instructions::ReportArgs {
        account_value: 0,
        short_notional: [0; 8],
        market_live: 0b11,
        credited_total: 0,
        withdrawn_total: 0,
        hl_time: env.now() * 1_000 + hl_time,
    };
    assert_ok(&env.report(args));
}

fn spy(env: &Env, session: i16) -> Vec<u8> {
    pyth_envelope(&env.pyth, env.now(), 3, &[Feed::at(SPY, SPY_PRICE, session, env.now())])
}

fn mint_x(env: &mut Env, x: &XUser, envelope: Vec<u8>, amount: u64, min_out: u64) -> TxResult {
    let accounts = aequa::accounts::MintWithCollateral {
        user: x.u.key(),
        config: env.p.config,
        reserve: env.p.reserve,
        collateral: Pdas::collateral(&SPYX),
        mint: SPYX,
        vault: Pdas::collateral_vault(&SPYX),
        user_xstock: x.spyx,
        usdq_mint: env.p.usdq_mint,
        user_usdq: x.u.usdq,
        reserve_fund: env.p.reserve_fund,
        protocol: env.p.protocol,
        aequa_stake: None,
        aequa_pool: Some(env.p.aequa_pool),
        token_program: TOKEN,
        xstock_token_program: TOKEN_2022,
        pyth: pyth_accounts(),
    };
    let ed = ed25519_ix(&envelope, 1);
    let program_ix = ix(accounts, aequa::instruction::MintWithCollateral { message: envelope, ed25519_index: 0, amount, min_out });
    env.send(&[ed, program_ix], &[&x.u.kp])
}

fn mint_s(env: &mut Env, x: &XUser, session: i16, amount: u64, min_out: u64) -> TxResult {
    let e = spy(env, session);
    mint_x(env, x, e, amount, min_out)
}

fn redeem_s(env: &mut Env, x: &XUser, session: i16, amount: u64, min_out: u64) -> TxResult {
    let e = spy(env, session);
    redeem_x(env, x, e, amount, min_out)
}

fn redeem_x(env: &mut Env, x: &XUser, envelope: Vec<u8>, amount: u64, min_out: u64) -> TxResult {
    let accounts = aequa::accounts::RedeemForCollateral {
        user: x.u.key(),
        config: env.p.config,
        reserve: env.p.reserve,
        collateral: Pdas::collateral(&SPYX),
        mint: SPYX,
        vault: Pdas::collateral_vault(&SPYX),
        user_xstock: x.spyx,
        usdq_mint: env.p.usdq_mint,
        user_usdq: x.u.usdq,
        reserve_fund: env.p.reserve_fund,
        protocol: env.p.protocol,
        aequa_stake: None,
        aequa_pool: Some(env.p.aequa_pool),
        token_program: TOKEN,
        xstock_token_program: TOKEN_2022,
        pyth: pyth_accounts(),
    };
    let ed = ed25519_ix(&envelope, 1);
    let program_ix = ix(accounts, aequa::instruction::RedeemForCollateral { message: envelope, ed25519_index: 0, amount, min_out });
    env.send(&[ed, program_ix], &[&x.u.kp])
}

/// The SPYx mint's ScaledUiAmount fields as the fixture holds them.
fn multiplier(env: &Env) -> (f64, i64, f64) {
    let data = env.svm.get_account(&SPYX).unwrap().data;
    let v = extension(&data, 25).expect("ScaledUiAmount");
    let m = f64::from_le_bytes(data[v + 32..v + 40].try_into().unwrap());
    let at = i64::from_le_bytes(data[v + 40..v + 48].try_into().unwrap());
    let next = f64::from_le_bytes(data[v + 48..v + 56].try_into().unwrap());
    (m, at, next)
}

/// Offset of a Token-2022 mint extension's value.
fn extension(data: &[u8], kind: u16) -> Option<usize> {
    let mut i = 166;
    while i + 4 <= data.len() {
        let t = u16::from_le_bytes([data[i], data[i + 1]]);
        let len = u16::from_le_bytes([data[i + 2], data[i + 3]]) as usize;
        if t == kind {
            return Some(i + 4);
        }
        if t == 0 {
            return None;
        }
        i += 4 + len;
    }
    None
}

fn patch_mint(env: &mut Env, kind: u16, f: impl FnOnce(&mut [u8])) {
    let mut acc = env.svm.get_account(&SPYX).unwrap();
    let v = extension(&acc.data, kind).expect("extension");
    f(&mut acc.data[v..]);
    env.svm.set_account(SPYX, acc).unwrap();
}

#[test]
fn mints_at_the_price_less_the_session_band() {
    let (mut env, x) = world();
    let (m, at, next) = multiplier(&env);
    let mult = math::multiplier_e12(if env.now() >= at { next } else { m }).unwrap();
    let p = (SPY_PRICE * 1e5).round() as u64 * 1_000;

    let r = mint_s(&mut env, &x, 0, 10 * X, 0);
    assert_ok(&r);
    let value = math::xstock_to_usd(10 * X, mult, p, 5, 8).unwrap();
    let (out, fee) = math::after_fee(value, 10);
    let e = &events::<Minted>(&r)[0];
    assert_eq!((e.usdq_out, e.fee, e.price_e8, e.session), (out, fee, p, 0));
    assert_eq!(env.balance(&x.u.usdq), out);
    assert_eq!(env.balance(&Pdas::collateral_vault(&SPYX)), 10 * X);
    // 10 SPYx at $665.1234 × M less 5 bps, less the 10 bps fee.
    let expected = 10.0 * SPY_PRICE * (if env.now() >= at { next } else { m }) * (1.0 - 0.0005) * (1.0 - 0.001);
    assert!(((out as f64) / 1e6 - expected).abs() < 0.01, "{out} vs {expected}");

    // Pre-market: 15 bps. Overnight: 25 bps.
    for (session, band) in [(1, 15), (2, 15), (3, 25)] {
        env.warp(1);
        live_report(&mut env);
        let r = mint_s(&mut env, &x, session, X, 0);
        assert_ok(&r);
        let e = &events::<Minted>(&r)[0];
        let value = math::xstock_to_usd(X, mult, p, band, 8).unwrap();
        assert_eq!(e.usdq_out + e.fee, value, "session {session}");
        assert_eq!(e.session, session as u8);
    }
}

#[test]
fn needs_a_live_price() {
    let (mut env, x) = world();
    // Closed session.
    assert_err(&mint_s(&mut env, &x, 4, X, 0), "CollateralNotLive");
    // A price carried forward from a minute ago.
    let t = env.now();
    let stale_feed = pyth_envelope(&env.pyth, t, 3, &[Feed::at(SPY, SPY_PRICE, 0, t - 60)]);
    assert_err(&mint_x(&mut env, &x, stale_feed, X, 0), "CollateralNotLive");
    // An update older than 10 s.
    let old = pyth_envelope(&env.pyth, t - 11, 3, &[Feed::at(SPY, SPY_PRICE, 0, t - 11)]);
    assert_err(&mint_x(&mut env, &x, old, X, 0), "StalePrice");
    // The one-second channel.
    let slow = pyth_envelope(&env.pyth, t, 4, &[Feed::at(SPY, SPY_PRICE, 0, t)]);
    assert_err(&mint_x(&mut env, &x, slow, X, 0), "WrongPythChannel");
    // QQQ's feed doesn't price SPYx.
    let other = pyth_envelope(&env.pyth, t, 3, &[Feed::at(QQQ, 600.0, 0, t)]);
    assert_err(&mint_x(&mut env, &x, other, X, 0), "CollateralNotLive");
    // Exactly 10 s old still counts.
    let edge = pyth_envelope(&env.pyth, t - 10, 3, &[Feed::at(SPY, SPY_PRICE, 0, t - 10)]);
    assert_ok(&mint_x(&mut env, &x, edge, X, 0));
}

#[test]
fn only_trusted_untampered_signatures_count() {
    let (mut env, x) = world();
    // A signer the Lazer storage doesn't list.
    let stranger = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    let t = env.now();
    let forged = pyth_envelope(&stranger, t, 3, &[Feed::at(SPY, SPY_PRICE, 0, t)]);
    let r = mint_x(&mut env, &x, forged, X, 0);
    assert!(r.is_err());
    assert!(logs_of(&r).contains("signature verification error"), "{}", logs_of(&r));

    // A trusted signature over different bytes: the ed25519 precompile refuses the transaction.
    let mut tampered = spy(&env, 0);
    let n = tampered.len();
    tampered[n - 20] ^= 1;
    assert!(mint_x(&mut env, &x, tampered, X, 0).is_err());

    // The ed25519 instruction must point at the program's own instruction.
    let envelope = spy(&env, 0);
    let mut ed = ed25519_ix(&envelope, 0);
    ed.data[4] = 0;
    let accounts = aequa::accounts::MintWithCollateral {
        user: x.u.key(),
        config: env.p.config,
        reserve: env.p.reserve,
        collateral: Pdas::collateral(&SPYX),
        mint: SPYX,
        vault: Pdas::collateral_vault(&SPYX),
        user_xstock: x.spyx,
        usdq_mint: env.p.usdq_mint,
        user_usdq: x.u.usdq,
        reserve_fund: env.p.reserve_fund,
        protocol: env.p.protocol,
        aequa_stake: None,
        aequa_pool: Some(env.p.aequa_pool),
        token_program: TOKEN,
        xstock_token_program: TOKEN_2022,
        pyth: pyth_accounts(),
    };
    let program_ix = ix(accounts, aequa::instruction::MintWithCollateral { message: envelope, ed25519_index: 0, amount: X, min_out: 0 });
    assert!(env.send(&[ed, program_ix], &[&x.u.kp]).is_err());

    // The production signer stays trusted next to this world's: a correct one still mints.
    assert_ok(&mint_s(&mut env, &x, 0, X, 0));
}

#[test]
fn needs_the_hedge_live() {
    let (mut env, x) = world();
    env.warp(121);
    assert_err(&mint_s(&mut env, &x, 0, X, 0), "HedgeNotLive");
    live_report(&mut env);
    assert_ok(&mint_s(&mut env, &x, 0, X, 0));
    // The SP500 perp paused: its bit is off.
    let args = aequa::instructions::ReportArgs {
        account_value: 0,
        short_notional: [0; 8],
        market_live: 0b10,
        credited_total: 0,
        withdrawn_total: 0,
        hl_time: env.now() * 1_000 + HL_TIME.fetch_add(1, Ordering::Relaxed),
    };
    assert_ok(&env.report(args));
    assert_err(&mint_s(&mut env, &x, 0, X, 0), "HedgeNotLive");
}

#[test]
fn corporate_action_window_and_issuer_pause() {
    let (mut env, x) = world();
    let now = env.now();
    // A multiplier change scheduled 10 h ahead: closed from 24 h before until 1 h after.
    patch_mint(&mut env, 25, |v| {
        let m = f64::from_le_bytes(v[32..40].try_into().unwrap());
        v[40..48].copy_from_slice(&(now + 10 * 3_600).to_le_bytes());
        v[48..56].copy_from_slice(&(m * 1.002).to_le_bytes());
    });
    assert_err(&mint_s(&mut env, &x, 0, X, 0), "CorporateActionWindow");
    env.warp(11 * 3_600 - 1);
    live_report(&mut env);
    assert_err(&mint_s(&mut env, &x, 0, X, 0), "CorporateActionWindow");
    env.warp(1);
    live_report(&mut env);
    let r = mint_s(&mut env, &x, 0, X, 0);
    assert_ok(&r);
    // The new multiplier is in force.
    let (m, _, next) = multiplier(&env);
    assert!(next > m);
    let p = (SPY_PRICE * 1e5).round() as u64 * 1_000;
    let value = math::xstock_to_usd(X, math::multiplier_e12(next).unwrap(), p, 5, 8).unwrap();
    let e = &events::<Minted>(&r)[0];
    assert_eq!(e.usdq_out + e.fee, value);

    // The issuer pauses the mint.
    patch_mint(&mut env, 26, |v| v[32] = 1);
    assert_err(&mint_s(&mut env, &x, 0, X, 0), "IssuerPaused");
    patch_mint(&mut env, 26, |v| v[32] = 0);

    // A transfer-hook program set by the issuer closes the collateral.
    patch_mint(&mut env, 14, |v| v[32..64].copy_from_slice(Keypair::new().pubkey().as_ref()));
    assert_err(&mint_s(&mut env, &x, 0, X, 0), "CollateralNotAdmissible");
}

#[test]
fn caps_and_buckets() {
    let (mut env, x) = world();
    // $500,000 an hour: about 750 SPYx.
    let r = mint_s(&mut env, &x, 0, 700 * X, 0);
    assert_ok(&r);
    assert_err(&mint_s(&mut env, &x, 0, 100 * X, 0), "CapacityExceeded");
    env.warp(3_600);
    live_report(&mut env);
    assert_ok(&mint_s(&mut env, &x, 0, 700 * X, 0));

    // Spot cap: lower it under what the vault holds.
    let mut args = listing(&env);
    args.spot_cap = 900_000 * USD;
    set_listing(&mut env, args);
    env.warp(3_600);
    live_report(&mut env);
    assert_err(&mint_s(&mut env, &x, 0, X, 0), "SpotCapExceeded");

    // The user's minimum.
    let mut args = listing(&env);
    args.spot_cap = 1_800_000 * USD;
    set_listing(&mut env, args);
    assert_err(&mint_s(&mut env, &x, 0, X, 10_000 * USD), "SlippageExceeded");
}

#[test]
fn redeems_at_the_price_plus_the_band() {
    let (mut env, x) = world();
    let start = env.balance(&x.spyx);
    assert_ok(&mint_s(&mut env, &x, 0, 100 * X, 0));
    let (m, at, next) = multiplier(&env);
    let mult = math::multiplier_e12(if env.now() >= at { next } else { m }).unwrap();
    let p = (SPY_PRICE * 1e5).round() as u64 * 1_000;

    let before = env.balance(&x.spyx);
    let r = redeem_s(&mut env, &x, 3, 10_000 * USD, 0);
    assert_ok(&r);
    let (net, fee) = math::after_fee(10_000 * USD, 10);
    let raw = math::usd_to_xstock(net, mult, p, 25, 8).unwrap();
    let e = &events::<Redeemed>(&r)[0];
    assert_eq!((e.amount_out, e.fee, e.session), (raw, fee, 3));
    assert_eq!(env.balance(&x.spyx) - before, raw);

    // A round trip never gains: the USDq from 100 SPYx buys back fewer.
    let usdq = env.balance(&x.u.usdq);
    assert_ok(&redeem_s(&mut env, &x, 0, usdq, 0));
    assert!(env.balance(&x.spyx) < start, "{} vs {start}", env.balance(&x.spyx));

    // More than the vault holds.
    assert_ok(&env.mint_with_usdc(&x.u, 900_000 * USD, 0));
    assert_err(&redeem_s(&mut env, &x, 0, 800_000 * USD, 0), "VaultInsufficient");
}

fn listing(env: &Env) -> aequa::instructions::ListingArgs {
    let c: Collateral = env.account(&Pdas::collateral(&SPYX));
    aequa::instructions::ListingArgs { feed: c.feed, bands_bps: c.bands_bps, spot_cap: c.spot_cap, hourly_cap: c.hourly_cap, perp: c.perp, perp_asset: c.perp_asset }
}

fn set_listing(env: &mut Env, args: aequa::instructions::ListingArgs) {
    let accounts = aequa::accounts::SetCollateral { config: env.p.config, admin: env.admin.pubkey(), collateral: Pdas::collateral(&SPYX) };
    let admin = clone(&env.admin);
    assert_ok(&env.send(&[ix(accounts, aequa::instruction::SetCollateral { args })], &[&admin]));
}
