//! The Realms voter-weight plugin: votes come from active stAEQUA positions. The
//! records follow spl-governance-addin-api's layouts; Anchor's discriminators for these names
//! are the ones the addin API checks. A record is valid for the slot it was updated in, so the
//! update runs in the same transaction as the vote or proposal.

use anchor_lang::prelude::*;

use crate::constants::{seeds::*, AEQUA_WINDOW_SECS};
use crate::errors::AequaError;
use crate::math;
use crate::state::{AequaPool, AequaStake, Config, StakeState};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub enum VoterWeightAction {
    CastVote,
    CommentProposal,
    CreateGovernance,
    CreateProposal,
    SignOffProposal,
}

#[account]
#[derive(InitSpace)]
pub struct VoterWeightRecord {
    pub realm: Pubkey,
    pub governing_token_mint: Pubkey,
    pub governing_token_owner: Pubkey,
    pub voter_weight: u64,
    pub voter_weight_expiry: Option<u64>,
    pub weight_action: Option<VoterWeightAction>,
    pub weight_action_target: Option<Pubkey>,
    pub reserved: [u8; 8],
}

#[account]
#[derive(InitSpace)]
pub struct MaxVoterWeightRecord {
    pub realm: Pubkey,
    pub governing_token_mint: Pubkey,
    pub max_voter_weight: u64,
    pub max_voter_weight_expiry: Option<u64>,
    pub reserved: [u8; 8],
}

#[derive(Accounts)]
#[instruction(realm: Pubkey)]
pub struct CreateVoterWeightRecord<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config)]
    pub config: Box<Account<'info, Config>>,
    /// CHECK: the voter the record speaks for.
    pub voter: UncheckedAccount<'info>,
    #[account(
        init,
        payer = payer,
        space = 8 + VoterWeightRecord::INIT_SPACE,
        seeds = [VOTER_WEIGHT, realm.as_ref(), config.aequa.mint.as_ref(), voter.key().as_ref()],
        bump,
    )]
    pub record: Box<Account<'info, VoterWeightRecord>>,
    pub system_program: Program<'info, System>,
}

impl CreateVoterWeightRecord<'_> {
    pub fn run(&mut self, realm: Pubkey) -> Result<()> {
        require!(self.config.aequa.is_set(), AequaError::AequaNotSet);
        let r = &mut self.record;
        r.realm = realm;
        r.governing_token_mint = self.config.aequa.mint;
        r.governing_token_owner = self.voter.key();
        r.voter_weight = 0;
        r.voter_weight_expiry = Some(0);
        Ok(())
    }
}

/// Sets the voter's weight to their active stAEQUA, in AEQUA, valid for this slot. Cooling
/// positions have no votes.
#[derive(Accounts)]
pub struct UpdateVoterWeightRecord<'info> {
    #[account(seeds = [CONFIG], bump = config.bumps.config)]
    pub config: Box<Account<'info, Config>>,
    #[account(seeds = [AEQUA_POOL], bump = aequa_pool.bump)]
    pub aequa_pool: Box<Account<'info, AequaPool>>,
    #[account(mut, constraint = record.governing_token_mint == config.aequa.mint @ AequaError::Unauthorized)]
    pub record: Box<Account<'info, VoterWeightRecord>>,
    #[account(seeds = [AEQUA_STAKE, record.governing_token_owner.as_ref()], bump = aequa_stake.bump)]
    pub aequa_stake: Option<Box<Account<'info, AequaStake>>>,
}

impl UpdateVoterWeightRecord<'_> {
    pub fn run(&mut self, action: Option<VoterWeightAction>, target: Option<Pubkey>) -> Result<()> {
        let clock = Clock::get()?;
        let now = clock.unix_timestamp;
        let cooldown = self.config.params.aequa_cooldown_secs as i64;
        let pool = &self.aequa_pool;
        let weight = match self.aequa_stake.as_ref() {
            Some(s) if s.state(now, cooldown, AEQUA_WINDOW_SECS) == StakeState::Active => {
                math::assets_for(s.shares, pool.assets(now), pool.total_shares)
            }
            _ => 0,
        };
        let r = &mut self.record;
        r.voter_weight = weight;
        r.voter_weight_expiry = Some(clock.slot);
        r.weight_action = action;
        r.weight_action_target = target;
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(realm: Pubkey)]
pub struct CreateMaxVoterWeightRecord<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config)]
    pub config: Box<Account<'info, Config>>,
    #[account(
        init,
        payer = payer,
        space = 8 + MaxVoterWeightRecord::INIT_SPACE,
        seeds = [MAX_VOTER_WEIGHT, realm.as_ref(), config.aequa.mint.as_ref()],
        bump,
    )]
    pub record: Box<Account<'info, MaxVoterWeightRecord>>,
    pub system_program: Program<'info, System>,
}

impl CreateMaxVoterWeightRecord<'_> {
    pub fn run(&mut self, realm: Pubkey) -> Result<()> {
        require!(self.config.aequa.is_set(), AequaError::AequaNotSet);
        let r = &mut self.record;
        r.realm = realm;
        r.governing_token_mint = self.config.aequa.mint;
        r.max_voter_weight = 0;
        r.max_voter_weight_expiry = Some(0);
        Ok(())
    }
}

/// The most votes there can be: AEQUA in the stake pool. Positions cooling down are counted,
/// so the quorum is measured against the whole pool.
#[derive(Accounts)]
pub struct UpdateMaxVoterWeightRecord<'info> {
    #[account(seeds = [CONFIG], bump = config.bumps.config)]
    pub config: Box<Account<'info, Config>>,
    #[account(seeds = [AEQUA_POOL], bump = aequa_pool.bump)]
    pub aequa_pool: Box<Account<'info, AequaPool>>,
    #[account(mut, constraint = record.governing_token_mint == config.aequa.mint @ AequaError::Unauthorized)]
    pub record: Box<Account<'info, MaxVoterWeightRecord>>,
}

impl UpdateMaxVoterWeightRecord<'_> {
    pub fn run(&mut self) -> Result<()> {
        let clock = Clock::get()?;
        self.record.max_voter_weight = self.aequa_pool.assets(clock.unix_timestamp);
        self.record.max_voter_weight_expiry = Some(clock.slot);
        Ok(())
    }
}
