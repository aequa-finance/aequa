//! The bridge, the Solana side: `bridge_out` burns buffer USDC through Circle's CCTP V2
//! programs (mainnet binaries and state) towards HyperEVM, with the CctpForwarder hook that
//! credits the hedge account on HyperCore; `land_usdc` brings USDC back into the buffer.

use aequa::events::BridgedOut;
use aequa_tests::*;
use solana_keypair::Keypair;
use solana_signer::Signer;

const HEDGE: [u8; 20] = [0x11; 20];

fn world() -> Env {
    let mut env = Env::new();
    env.load_cctp();
    env.list(SPYX, 15_000_000 * USD);
    let u = env.user(1_000_000 * USD);
    assert_ok(&env.mint_with_usdc(&u, 1_000_000 * USD, 0));
    env
}

/// The CCTP message CCTP wrote into the event account: header and burn body (big-endian).
struct Sent {
    destination_domain: u32,
    destination_caller: [u8; 32],
    min_finality: u32,
    mint_recipient: [u8; 32],
    amount: u128,
    sender: [u8; 32],
    max_fee: u128,
    hook: Vec<u8>,
}

fn read_sent(env: &Env, event: &solana_address::Address) -> Sent {
    let acc = env.svm.get_account(event).expect("event account");
    assert_eq!(acc.owner, CCTP_MT);
    // discriminator 8 | rent_payer 32 | created_at 8 | message: u32 len + bytes
    let len = u32::from_le_bytes(acc.data[48..52].try_into().unwrap()) as usize;
    let m = &acc.data[52..52 + len];
    let u32be = |o: usize| u32::from_be_bytes(m[o..o + 4].try_into().unwrap());
    let b32 = |o: usize| -> [u8; 32] { m[o..o + 32].try_into().unwrap() };
    let u256 = |o: usize| u128::from_be_bytes(m[o + 16..o + 32].try_into().unwrap());
    let body = 148;
    Sent {
        destination_domain: u32be(8),
        destination_caller: b32(108),
        min_finality: u32be(140),
        mint_recipient: b32(body + 36),
        amount: u256(body + 68),
        sender: b32(body + 100),
        max_fee: u256(body + 132),
        hook: m[body + 228..].to_vec(),
    }
}

#[test]
fn burns_to_the_hedge_account_through_the_forwarder() {
    let mut env = world();
    let keeper = clone(&env.keeper);
    // No route yet: the CCTP programs can't match an unset route.
    let (r, _) = env.bridge_out(&keeper, 100 * USD, 0, true);
    assert_err(&r, "ConstraintAddress");
    assert_ok(&env.set_route(HEDGE));

    let supply = env.supply(&USDC);
    let buffer = env.balance(&env.p.buffer);
    // 1 bps plus Circle's forwarding fee, within the route's cap of 1 bps + $1.
    let (amount, fee) = (100_000 * USD, 10_250_000);
    let (r, event) = env.bridge_out(&keeper, amount, fee, true);
    assert_ok(&r);
    assert_eq!(env.balance(&env.p.buffer), buffer - amount);
    assert_eq!(env.supply(&USDC), supply - amount, "CCTP burned it");
    assert_eq!(env.reserve().bridged_out_total, amount - fee, "what will reach HyperCore");
    let e = &events::<BridgedOut>(&r)[0];
    assert_eq!((e.amount, e.message), (amount, event));

    let m = read_sent(&env, &event);
    assert_eq!(m.destination_domain, 19);
    assert_eq!(m.mint_recipient, CCTP_FORWARDER);
    assert_eq!(m.destination_caller, CCTP_FORWARDER);
    assert_eq!((m.amount, m.max_fee, m.min_finality), (amount as u128, fee as u128, 1_000));
    assert_eq!(m.sender, env.p.buffer.to_bytes(), "the message names the buffer as its sender");
    assert_eq!(m.hook, hypercore_hook(HEDGE).to_vec());

    // Standard finality.
    let (r, event) = env.bridge_out(&keeper, 50_000 * USD, 300_000, false);
    assert_ok(&r);
    assert_eq!(read_sent(&env, &event).min_finality, 2_000);
}

#[test]
fn only_the_keeper_within_the_fee_cap_and_the_buffer() {
    let mut env = world();
    assert_ok(&env.set_route(HEDGE));
    let keeper = clone(&env.keeper);
    // Over 1 bps + $1.
    let (r, _) = env.bridge_out(&keeper, 100_000 * USD, 11_000_001, true);
    assert_err(&r, "ParamOutOfRange");
    let (r, _) = env.bridge_out(&keeper, 2_000_000 * USD, 0, true);
    assert_err(&r, "BufferInsufficient");
    let stranger = Keypair::new();
    env.svm.airdrop(&stranger.pubkey(), 10_000_000_000).unwrap();
    let (r, _) = env.bridge_out(&stranger, 100 * USD, 0, true);
    assert_err(&r, "Unauthorized");
    // The guardian's keeper pause stops it.
    let accounts = aequa::accounts::Pause { config: env.p.config, signer: env.guardian.pubkey(), collateral: None };
    let guardian = clone(&env.guardian);
    assert_ok(&env.send(&[ix(accounts, aequa::instruction::Pause { paths: aequa::state::path::KEEPER })], &[&guardian]));
    let (r, _) = env.bridge_out(&keeper, 100 * USD, 0, true);
    assert_err(&r, "Paused");
}

#[test]
fn route_is_fixed_but_the_account() {
    let mut env = world();
    let args = |max_fee_bps, recipient| BridgeArgs { recipient, max_fee_bps, max_fee_flat: 0 };
    assert_err(&env.admin_ix(aequa::instruction::SetBridgeRoute { args: args(11, HEDGE) }), "ParamOutOfRange");
    assert_err(&env.admin_ix(aequa::instruction::SetBridgeRoute { args: args(1, [0; 20]) }), "ParamOutOfRange");
    assert_ok(&env.admin_ix(aequa::instruction::SetBridgeRoute { args: args(1, HEDGE) }));
    // CCTP V2, HyperEVM, Circle's forwarder and its hook come from the program.
    let route = env.config().bridge;
    assert_eq!((route.token_messenger_minter, route.message_transmitter, route.destination_domain), (CCTP_TMM, CCTP_MT, 19));
    assert_eq!((route.mint_recipient, route.destination_caller), (CCTP_FORWARDER, CCTP_FORWARDER));
    assert_eq!(&route.hook_data[..route.hook_len as usize], &hypercore_hook(HEDGE)[..]);
}
