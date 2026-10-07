//! Pyth Pro prices. A signed update travels in the instruction's first argument; an
//! ed25519 instruction earlier in the transaction checks its signature over those bytes, and
//! the Lazer program's `verify_message` confirms the signer is trusted. The program then reads
//! the payload itself: the timestamp, the channel, each feed's price, exponent and session.
//! The Lazer program checks none of those.
//!
//! Layouts: pyth-network/pyth-lazer-public, contracts/solana (signature.rs, lib.rs) and
//! sdk/rust/protocol (message.rs, payload.rs), protocol v0.46.0.

use anchor_lang::{
    prelude::*,
    solana_program::{instruction::Instruction, program::invoke},
};

use crate::errors::AequaError;
use crate::math;
use crate::state::session;

pub const LAZER_PROGRAM: Pubkey = pubkey!("pytd2yyk641x7ak7mkaasSJVXh6YYZnC7wTmtgAyxPt");
pub const LAZER_STORAGE: Pubkey = pubkey!("3rdJbqfnagQ4yx9HXJViD4zc4xpiSqmFsKpPuSCQVyQL");
pub const INSTRUCTIONS_SYSVAR: Pubkey = pubkey!("Sysvar1nstructions1111111111111111111111111");

/// `sha256("global:verify_message")[..8]`.
const VERIFY_MESSAGE: [u8; 8] = [180, 193, 120, 55, 189, 135, 203, 83];
/// The "solana" envelope: magic, signature, signer, payload length, payload.
const ENVELOPE_MAGIC: u32 = 2_182_742_457;
const PAYLOAD_MAGIC: u32 = 2_479_346_549;
const ENVELOPE_PAYLOAD: usize = 102;

/// Channels: 1 real time, 2 every 50 ms, 3 every 200 ms, 4 every second. US equity feeds start
/// at 50 ms; anything up to 200 ms is accepted.
pub const MAX_CHANNEL: u8 = 3;

const PROP_PRICE: u8 = 0;
const PROP_EXPONENT: u8 = 4;
const PROP_MARKET_SESSION: u8 = 9;

/// The accounts a Pyth Pro verification needs, as they appear in an instruction's context.
pub struct PythAccounts<'a, 'info> {
    pub payer: &'a AccountInfo<'info>,
    pub program: &'a AccountInfo<'info>,
    pub storage: &'a AccountInfo<'info>,
    pub treasury: &'a AccountInfo<'info>,
    pub system_program: &'a AccountInfo<'info>,
    pub instructions: &'a AccountInfo<'info>,
}

/// One feed's figures from a verified update.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeedUpdate {
    pub id: u32,
    pub price: i64,
    pub exponent: i16,
    pub session: i16,
    pub has_price: bool,
    pub has_exponent: bool,
    pub has_session: bool,
}

#[derive(Clone, Debug)]
pub struct Update {
    pub timestamp_us: u64,
    pub channel: u8,
    pub feeds: Vec<FeedUpdate>,
}

/// A price ready for the program's math.
#[derive(Clone, Copy, Debug)]
pub struct Price {
    pub price_e8: u64,
    pub session: u8,
}

/// Verifies `message` through the Lazer program and returns the payload it carries.
/// `message` must be the bytes of the instruction's first argument.
pub fn verify(accounts: &PythAccounts, message: &[u8], ed25519_index: u16) -> Result<Update> {
    require_keys_eq!(*accounts.program.key, LAZER_PROGRAM, AequaError::InvalidPythMessage);
    require_keys_eq!(*accounts.storage.key, LAZER_STORAGE, AequaError::InvalidPythMessage);
    require_keys_eq!(*accounts.instructions.key, INSTRUCTIONS_SYSVAR, AequaError::InvalidPythMessage);

    let mut data = Vec::with_capacity(message.len() + 15);
    data.extend_from_slice(&VERIFY_MESSAGE);
    data.extend_from_slice(&(message.len() as u32).to_le_bytes());
    data.extend_from_slice(message);
    data.extend_from_slice(&ed25519_index.to_le_bytes());
    data.push(0);
    let ix = Instruction {
        program_id: LAZER_PROGRAM,
        accounts: vec![
            AccountMeta::new(*accounts.payer.key, true),
            AccountMeta::new_readonly(*accounts.storage.key, false),
            AccountMeta::new(*accounts.treasury.key, false),
            AccountMeta::new_readonly(*accounts.system_program.key, false),
            AccountMeta::new_readonly(*accounts.instructions.key, false),
        ],
        data,
    };
    invoke(
        &ix,
        &[
            accounts.payer.clone(),
            accounts.storage.clone(),
            accounts.treasury.clone(),
            accounts.system_program.clone(),
            accounts.instructions.clone(),
            accounts.program.clone(),
        ],
    )?;
    parse(message)
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).ok_or(AequaError::InvalidPythMessage)?;
        require!(end <= self.b.len(), AequaError::InvalidPythMessage);
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    /// A flag byte, then a u64 when the flag is set.
    fn opt_u64(&mut self) -> Result<Option<u64>> {
        Ok(if self.u8()? != 0 { Some(self.u64()?) } else { None })
    }
}

/// Reads the envelope's payload. Every property id the protocol defines is understood; an
/// unknown one fails, since its length can't be known.
pub fn parse(message: &[u8]) -> Result<Update> {
    let mut r = Reader { b: message, at: 0 };
    require!(r.u32()? == ENVELOPE_MAGIC, AequaError::InvalidPythMessage);
    r.take(64 + 32)?;
    let len = r.u16()? as usize;
    require!(message.len() == ENVELOPE_PAYLOAD + len, AequaError::InvalidPythMessage);

    require!(r.u32()? == PAYLOAD_MAGIC, AequaError::InvalidPythMessage);
    let timestamp_us = r.u64()?;
    let channel = r.u8()?;
    let n = r.u8()? as usize;
    let mut feeds = Vec::with_capacity(n);
    for _ in 0..n {
        let mut f = FeedUpdate { id: r.u32()?, ..Default::default() };
        let props = r.u8()?;
        for _ in 0..props {
            match r.u8()? {
                PROP_PRICE => {
                    f.price = r.i64()?;
                    f.has_price = f.price != 0;
                }
                1 | 2 | 5 | 10 | 11 => {
                    r.i64()?;
                }
                3 => {
                    r.u16()?;
                }
                PROP_EXPONENT => {
                    f.exponent = r.i16()?;
                    f.has_exponent = true;
                }
                6 => {
                    if r.u8()? != 0 {
                        r.i64()?;
                    }
                }
                7 | 8 | 12 => {
                    r.opt_u64()?;
                }
                PROP_MARKET_SESSION => {
                    f.session = r.i16()?;
                    f.has_session = true;
                }
                _ => return err!(AequaError::InvalidPythMessage),
            }
        }
        feeds.push(f);
    }
    require!(r.at == message.len(), AequaError::InvalidPythMessage);
    Ok(Update { timestamp_us, channel, feeds })
}

impl Update {
    /// The update must be recent and on a fast channel.
    pub fn require_fresh(&self, now: i64, freshness_secs: u32) -> Result<()> {
        require!(self.channel >= 1 && self.channel <= MAX_CHANNEL, AequaError::WrongPythChannel);
        let ts = (self.timestamp_us / 1_000_000) as i64;
        require!(now.saturating_sub(ts) <= freshness_secs as i64, AequaError::StalePrice);
        Ok(())
    }

    /// A feed's price in an open session: a session other than closed. `CollateralNotLive`
    /// otherwise.
    pub fn price(&self, feed: u32) -> Result<Price> {
        let f = self.feeds.iter().find(|f| f.id == feed).ok_or(AequaError::CollateralNotLive)?;
        require!(f.has_price && f.has_exponent && f.has_session, AequaError::InvalidPythPrice);
        require!(f.session >= session::REGULAR as i16 && f.session < session::CLOSED as i16, AequaError::CollateralNotLive);
        Ok(Price { price_e8: math::price_e8(f.price, f.exponent)?, session: f.session as u8 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(payload: &[u8]) -> Vec<u8> {
        let mut m = ENVELOPE_MAGIC.to_le_bytes().to_vec();
        m.extend_from_slice(&[7u8; 96]);
        m.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        m.extend_from_slice(payload);
        m
    }

    fn payload(ts: u64, channel: u8, feeds: &[(u32, i64, i16, i16)]) -> Vec<u8> {
        let mut p = PAYLOAD_MAGIC.to_le_bytes().to_vec();
        p.extend_from_slice(&ts.to_le_bytes());
        p.push(channel);
        p.push(feeds.len() as u8);
        for (id, price, expo, session) in feeds {
            p.extend_from_slice(&id.to_le_bytes());
            p.push(4);
            p.push(PROP_PRICE);
            p.extend_from_slice(&price.to_le_bytes());
            p.push(PROP_EXPONENT);
            p.extend_from_slice(&expo.to_le_bytes());
            p.push(5);
            p.extend_from_slice(&123i64.to_le_bytes());
            p.push(PROP_MARKET_SESSION);
            p.extend_from_slice(&session.to_le_bytes());
        }
        p
    }

    #[test]
    fn reads_feeds_and_sessions() {
        let ts = 1_791_212_400_000_000u64;
        let m = envelope(&payload(ts, 3, &[(1398, 66_512_340, -5, 0), (1363, 59_001_000, -5, 3)]));
        let u = parse(&m).unwrap();
        assert_eq!(u.feeds.len(), 2);
        u.require_fresh(1_791_212_405, 10).unwrap();
        assert!(u.require_fresh(1_791_212_411, 10).is_err());
        let p = u.price(1398).unwrap();
        assert_eq!((p.price_e8, p.session), (66_512_340_000, 0));
        assert_eq!(u.price(1363).unwrap().session, 3);
        assert!(u.price(1).is_err());
        let closed = parse(&envelope(&payload(ts, 3, &[(1398, 66_512_340, -5, 4)]))).unwrap();
        assert!(closed.price(1398).is_err(), "closed session");
    }

    #[test]
    fn rejects_malformed() {
        let ts = 1_791_212_400_000_000u64;
        let mut m = envelope(&payload(ts, 3, &[(1398, 1, -5, 0)]));
        m.push(0);
        assert!(parse(&m).is_err(), "trailing byte");
        let mut p = payload(ts, 3, &[(1398, 1, -5, 0)]);
        p[19] = 99; // first property id
        assert!(parse(&envelope(&p)).is_err(), "unknown property");
        let u = parse(&envelope(&payload(ts, 4, &[(1398, 1, -5, 0)]))).unwrap();
        assert!(u.require_fresh(1_791_212_400, 10).is_err(), "one-second channel");
    }
}
