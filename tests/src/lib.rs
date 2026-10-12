//! A LiteSVM world for the aequa program: mainnet's token programs and mints from
//! tests/fixtures (scripts/fixtures.sh), the program as built by `cargo build-sbf`, and the
//! protocol initialized the way scripts/deploy initializes it on mainnet.

use std::{fs, path::PathBuf};

use anchor_lang::{AccountDeserialize, AnchorSerialize, Discriminator, InstructionData, ToAccountMetas};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use litesvm::{
    types::{FailedTransactionMetadata, TransactionMetadata},
    LiteSVM,
};
use solana_account::Account;
use solana_address::Address as Pubkey;
use solana_clock::Clock;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_message::{v0, VersionedMessage};
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;

use aequa::{constants::seeds, state::*};

pub const TOKEN: Pubkey = Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const TOKEN_2022: Pubkey = Pubkey::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
pub const ATA: Pubkey = Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
pub const SYSTEM: Pubkey = Pubkey::from_str_const("11111111111111111111111111111111");
pub const LOADER_V3: Pubkey = Pubkey::from_str_const("BPFLoaderUpgradeab1e11111111111111111111111");
pub const PYTH_LAZER: Pubkey = Pubkey::from_str_const("pytd2yyk641x7ak7mkaasSJVXh6YYZnC7wTmtgAyxPt");
pub const PYTH_STORAGE: Pubkey = Pubkey::from_str_const("3rdJbqfnagQ4yx9HXJViD4zc4xpiSqmFsKpPuSCQVyQL");
pub const PYTH_TREASURY: Pubkey = Pubkey::from_str_const("Gx4MBPb1vqZLJajZmsKLg8fGw9ErhoKsR8LeKcCKFyak");
pub const TOKEN_METADATA: Pubkey = Pubkey::from_str_const("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");
pub const USDC: Pubkey = Pubkey::from_str_const("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
pub const SPYX: Pubkey = Pubkey::from_str_const("XsoCS1TfEyfFhfvj8EtZ528L3CaKBDBRqRapnBbDF2W");
pub const QQQX: Pubkey = Pubkey::from_str_const("Xs8S1uUs1zvS2p7iwtsG3b6fkhpvmwz4GYU3gWAmWHZ");

pub const USD: u64 = 1_000_000;
pub use aequa::instructions::BridgeArgs;
pub const DAY: i64 = 86_400;
/// Thursday 2026-10-08 15:00 UTC: 11:00 in New York, the regular session.
pub const START: i64 = 1_791_471_600;

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name)
}

/// An account saved by `solana account --output json-compact`.
pub fn load_account(name: &str) -> Account {
    let v: serde_json::Value = serde_json::from_slice(&fs::read(fixture(name)).unwrap()).unwrap();
    let a = &v["account"];
    Account {
        lamports: a["lamports"].as_u64().unwrap(),
        data: B64.decode(a["data"][0].as_str().unwrap()).unwrap(),
        owner: a["owner"].as_str().unwrap().parse().unwrap(),
        executable: a["executable"].as_bool().unwrap(),
        rent_epoch: 0,
    }
}

pub fn pda(seeds: &[&[u8]]) -> Pubkey {
    Pubkey::find_program_address(seeds, &aequa::ID).0
}

pub fn ata(owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[owner.as_ref(), token_program.as_ref(), mint.as_ref()], &ATA).0
}

/// Every address the program derives.
pub struct Pdas {
    pub config: Pubkey,
    pub reserve: Pubkey,
    pub usdq_vault: Pubkey,
    pub aequa_pool: Pubkey,
    pub usdq_mint: Pubkey,
    pub susdq_mint: Pubkey,
    pub buffer: Pubkey,
    pub landing_authority: Pubkey,
    pub landing: Pubkey,
    pub susdq_assets: Pubkey,
    pub susdq_silo: Pubkey,
    pub reserve_fund: Pubkey,
    pub protocol: Pubkey,
    pub swap_authority: Pubkey,
    pub transit_usdc: Pubkey,
}

impl Pdas {
    pub fn new() -> Self {
        Self {
            config: pda(&[seeds::CONFIG]),
            reserve: pda(&[seeds::RESERVE]),
            usdq_vault: pda(&[seeds::SUSDQ_VAULT]),
            aequa_pool: pda(&[seeds::AEQUA_POOL]),
            usdq_mint: pda(&[seeds::USDQ_MINT]),
            susdq_mint: pda(&[seeds::SUSDQ_MINT]),
            buffer: pda(&[seeds::BUFFER]),
            landing_authority: pda(&[seeds::LANDING]),
            landing: ata(&pda(&[seeds::LANDING]), &USDC, &TOKEN),
            susdq_assets: pda(&[seeds::SUSDQ_ASSETS]),
            susdq_silo: pda(&[seeds::SUSDQ_SILO]),
            reserve_fund: pda(&[seeds::RESERVE_FUND]),
            protocol: pda(&[seeds::PROTOCOL]),
            swap_authority: pda(&[seeds::SWAP_AUTHORITY]),
            transit_usdc: ata(&pda(&[seeds::SWAP_AUTHORITY]), &USDC, &TOKEN),
        }
    }

    /// The swap authority's transit account for a mint.
    pub fn transit(mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
        ata(&pda(&[seeds::SWAP_AUTHORITY]), mint, token_program)
    }

    pub fn collateral(mint: &Pubkey) -> Pubkey {
        pda(&[seeds::COLLATERAL, mint.as_ref()])
    }

    pub fn collateral_vault(mint: &Pubkey) -> Pubkey {
        pda(&[seeds::COLLATERAL_VAULT, mint.as_ref()])
    }

    pub fn cooldown(owner: &Pubkey) -> Pubkey {
        pda(&[seeds::SUSDQ_COOLDOWN, owner.as_ref()])
    }

    pub fn aequa_stake(owner: &Pubkey) -> Pubkey {
        pda(&[seeds::AEQUA_STAKE, owner.as_ref()])
    }
}

impl Default for Pdas {
    fn default() -> Self {
        Self::new()
    }
}

/// AEQUA's mint, the pool it is priced in, and the program's AEQUA accounts.
pub struct AequaWorld {
    pub mint: Pubkey,
    pub quote_mint: Pubkey,
    pub pool: Pubkey,
    pub pool_aequa_vault: Pubkey,
    pub pool_quote_vault: Pubkey,
    pub pool_vault: Pubkey,
    pub slash_sale: Pubkey,
    pub twap: Pubkey,
}

/// A wallet with its token accounts.
pub struct User {
    pub kp: Keypair,
    pub usdc: Pubkey,
    pub usdq: Pubkey,
    pub susdq: Pubkey,
}

impl User {
    pub fn key(&self) -> Pubkey {
        self.kp.pubkey()
    }
}

pub type TxResult = Result<TransactionMetadata, FailedTransactionMetadata>;

pub struct Env {
    pub svm: LiteSVM,
    pub payer: Keypair,
    pub admin: Keypair,
    pub guardian: Keypair,
    pub keeper: Keypair,
    pub reporter: Keypair,
    /// Holds the mint authority of the fixture mints (USDC, SPYx, QQQx) in this world.
    pub issuer: Keypair,
    /// A Pyth Pro signer the Lazer storage trusts in this world.
    pub pyth: ed25519_dalek::SigningKey,
    pub p: Pdas,
}

impl Env {
    /// The program deployed and initialized, its vaults created and every path unpaused.
    pub fn new() -> Self {
        let mut env = Self::bare();
        env.initialize(Params::launch());
        env.create_vaults();
        env.admin_ix(aequa::instruction::Unpause { paths: path::ALL }).unwrap();
        env
    }

    /// The program deployed, nothing initialized.
    pub fn bare() -> Self {
        let mut svm = LiteSVM::new().with_precompiles().with_log_bytes_limit(Some(100_000));
        for (id, name) in [(TOKEN, "spl_token.so"), (TOKEN_2022, "spl_token_2022.so"), (ATA, "spl_ata.so"), (PYTH_LAZER, "pyth_lazer.so"), (TOKEN_METADATA, "mpl_token_metadata.so")] {
            svm.add_program_from_file(id, fixture(name)).unwrap();
        }
        let so = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/deploy/aequa.so");
        svm.add_program_from_file(aequa::ID, so).unwrap();

        let payer = Keypair::new();
        let issuer = Keypair::new();
        svm.airdrop(&payer.pubkey(), 1_000_000_000_000).unwrap();
        // The deployer holds the upgrade authority, as on mainnet.
        let programdata = Pubkey::find_program_address(&[aequa::ID.as_ref()], &LOADER_V3).0;
        let mut pd = svm.get_account(&programdata).unwrap();
        pd.data[12] = 1;
        pd.data[13..45].copy_from_slice(payer.pubkey().as_ref());
        svm.set_account(programdata, pd).unwrap();

        let pyth = ed25519_dalek::SigningKey::from_bytes(&[42u8; 32]);
        for (addr, name) in [(USDC, "usdc_mint.json"), (SPYX, "spyx_mint.json"), (QQQX, "qqqx_mint.json"), (PYTH_STORAGE, "pyth_lazer_storage.json"), (PYTH_TREASURY, "pyth_lazer_treasury.json")] {
            let mut acc = load_account(name);
            if [USDC, SPYX, QQQX].contains(&addr) {
                // COption<Pubkey> mint authority at the head of the mint.
                acc.data[0..4].copy_from_slice(&1u32.to_le_bytes());
                acc.data[4..36].copy_from_slice(issuer.pubkey().as_ref());
            }
            if addr == PYTH_STORAGE {
                // A second trusted signer this world holds, next to the production one.
                acc.data[80] = 2;
                acc.data[121..153].copy_from_slice(pyth.verifying_key().as_bytes());
                acc.data[153..161].copy_from_slice(&i64::MAX.to_le_bytes());
            }
            svm.set_account(addr, acc).unwrap();
        }

        let mut env = Self {
            svm,
            payer,
            admin: Keypair::new(),
            guardian: Keypair::new(),
            keeper: Keypair::new(),
            reporter: Keypair::new(),
            issuer,
            pyth,
            p: Pdas::new(),
        };
        for k in [env.admin.pubkey(), env.guardian.pubkey(), env.keeper.pubkey(), env.reporter.pubkey(), env.issuer.pubkey()] {
            env.svm.airdrop(&k, 100_000_000_000).unwrap();
        }
        env.set_time(START);
        env
    }

    pub fn initialize(&mut self, params: Params) {
        let r = self.try_initialize(params, None);
        assert!(r.is_ok(), "initialize: {}", logs_of(&r));
    }

    pub fn try_initialize(&mut self, params: Params, signer: Option<&Keypair>) -> TxResult {
        let payer = signer.unwrap_or(&self.payer);
        let programdata = Pubkey::find_program_address(&[aequa::ID.as_ref()], &LOADER_V3).0;
        let accounts = aequa::accounts::Initialize {
            payer: payer.pubkey(),
            program: aequa::ID,
            program_data: programdata,
            config: self.p.config,
            reserve: self.p.reserve,
            usdq_vault: self.p.usdq_vault,
            aequa_pool: self.p.aequa_pool,
            usdq_mint: self.p.usdq_mint,
            susdq_mint: self.p.susdq_mint,
            usdc_mint: USDC,
            token_program: TOKEN,
            system_program: SYSTEM,
        };
        let args = aequa::instructions::InitializeArgs {
            admin: self.admin.pubkey(),
            guardian: self.guardian.pubkey(),
            keeper: self.keeper.pubkey(),
            reporter: self.reporter.pubkey(),
            params,
        };
        let ix = ix(accounts, aequa::instruction::Initialize { args });
        let kp = Keypair::try_from(payer.to_bytes().as_slice()).unwrap();
        self.send(&[ix], &[&kp])
    }

    /// The vaults, as the setup creates them: the landing and USDC transit accounts first
    /// (associated accounts anyone could create), then the program's own accounts.
    pub fn create_vaults(&mut self) {
        let r = self.try_create_vaults();
        assert!(r.is_ok(), "create_vaults: {}", logs_of(&r));
    }

    pub fn try_create_vaults(&mut self) -> TxResult {
        let accounts = aequa::accounts::CreateVaults {
            payer: self.payer.pubkey(),
            config: self.p.config,
            admin: self.admin.pubkey(),
            usdq_mint: self.p.usdq_mint,
            usdc_mint: USDC,
            buffer: self.p.buffer,
            landing_authority: self.p.landing_authority,
            landing: self.p.landing,
            swap_authority: self.p.swap_authority,
            transit_usdc: self.p.transit_usdc,
            reserve_fund: self.p.reserve_fund,
            protocol: self.p.protocol,
            token_program: TOKEN,
            system_program: SYSTEM,
        };
        let susdq = aequa::accounts::CreateSusdqVaults {
            payer: self.payer.pubkey(),
            config: self.p.config,
            admin: self.admin.pubkey(),
            usdq_mint: self.p.usdq_mint,
            susdq_assets: self.p.susdq_assets,
            susdq_silo: self.p.susdq_silo,
            token_program: TOKEN,
            system_program: SYSTEM,
        };
        let ixs = [
            self.ata_ix(&self.p.landing_authority, &USDC, &TOKEN),
            self.ata_ix(&self.p.swap_authority, &USDC, &TOKEN),
            ix(accounts, aequa::instruction::CreateVaults {}),
            ix(susdq, aequa::instruction::CreateSusdqVaults {}),
        ];
        let (payer, admin) = (clone(&self.payer), clone(&self.admin));
        self.send(&ixs, &[&payer, &admin])
    }

    /// The ATA program's CreateIdempotent, paid by the payer.
    pub fn ata_ix(&self, owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Instruction {
        Instruction {
            program_id: ATA,
            accounts: vec![
                AccountMeta::new(self.payer.pubkey(), true),
                AccountMeta::new(ata(owner, mint, token_program), false),
                AccountMeta::new_readonly(*owner, false),
                AccountMeta::new_readonly(*mint, false),
                AccountMeta::new_readonly(SYSTEM, false),
                AccountMeta::new_readonly(*token_program, false),
            ],
            data: vec![1],
        }
    }

    /// An admin-only instruction over `AdminOnly`.
    pub fn admin_ix<T: InstructionData>(&mut self, data: T) -> TxResult {
        let accounts = aequa::accounts::AdminOnly { config: self.p.config, admin: self.admin.pubkey() };
        let admin = clone(&self.admin);
        self.send(&[ix(accounts, data)], &[&admin])
    }

    pub fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> TxResult {
        let payer = signers[0].pubkey();
        self.svm.expire_blockhash();
        let msg = v0::Message::try_compile(&payer, ixs, &[], self.svm.latest_blockhash()).unwrap();
        let tx = VersionedTransaction::try_new(VersionedMessage::V0(msg), signers).unwrap();
        self.svm.send_transaction(tx)
    }

    pub fn now(&self) -> i64 {
        self.svm.get_sysvar::<Clock>().unix_timestamp
    }

    pub fn set_time(&mut self, t: i64) {
        let mut c = self.svm.get_sysvar::<Clock>();
        c.unix_timestamp = t;
        c.slot += 1;
        self.svm.set_sysvar(&c);
    }

    pub fn warp(&mut self, secs: i64) {
        let t = self.now() + secs;
        self.set_time(t);
    }

    pub fn account<T: AccountDeserialize>(&self, addr: &Pubkey) -> T {
        let a = self.svm.get_account(addr).unwrap_or_else(|| panic!("no account {addr}"));
        T::try_deserialize(&mut a.data.as_slice()).unwrap()
    }

    pub fn exists(&self, addr: &Pubkey) -> bool {
        self.svm.get_account(addr).map(|a| a.lamports > 0).unwrap_or(false)
    }

    pub fn config(&self) -> Config {
        self.account(&self.p.config)
    }

    pub fn reserve(&self) -> Reserve {
        self.account(&self.p.reserve)
    }

    pub fn usdq_vault(&self) -> UsdqVault {
        self.account(&self.p.usdq_vault)
    }

    /// A token account's amount (SPL Token or Token-2022: the amount sits at 64..72).
    pub fn balance(&self, token_account: &Pubkey) -> u64 {
        match self.svm.get_account(token_account) {
            Some(a) if a.data.len() >= 72 => u64::from_le_bytes(a.data[64..72].try_into().unwrap()),
            _ => 0,
        }
    }

    /// A mint's supply.
    pub fn supply(&self, mint: &Pubkey) -> u64 {
        let a = self.svm.get_account(mint).unwrap();
        u64::from_le_bytes(a.data[36..44].try_into().unwrap())
    }

    /// Creates the associated token account if needed.
    pub fn create_ata(&mut self, owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
        let addr = ata(owner, mint, token_program);
        let ix = self.ata_ix(owner, mint, token_program);
        let payer = clone(&self.payer);
        let r = self.send(&[ix], &[&payer]);
        assert!(r.is_ok(), "create ata: {}", logs_of(&r));
        addr
    }

    /// Mints fixture tokens (USDC, SPYx, QQQx) as their issuer.
    pub fn mint_to(&mut self, mint: &Pubkey, to: &Pubkey, amount: u64) {
        let token_program = self.svm.get_account(mint).unwrap().owner;
        let mut data = vec![7u8];
        data.extend_from_slice(&amount.to_le_bytes());
        let ix = Instruction {
            program_id: token_program,
            accounts: vec![AccountMeta::new(*mint, false), AccountMeta::new(*to, false), AccountMeta::new_readonly(self.issuer.pubkey(), true)],
            data,
        };
        let issuer = clone(&self.issuer);
        let r = self.send(&[ix], &[&issuer]);
        assert!(r.is_ok(), "mint_to: {}", logs_of(&r));
    }

    /// A funded wallet with its USDC, USDq and sUSDq accounts, holding `usdc` dollars.
    pub fn user(&mut self, usdc: u64) -> User {
        let kp = Keypair::new();
        self.svm.airdrop(&kp.pubkey(), 10_000_000_000).unwrap();
        let k = kp.pubkey();
        let u = User {
            usdc: self.create_ata(&k, &USDC, &TOKEN),
            usdq: self.create_ata(&k, &self.p.usdq_mint.clone(), &TOKEN),
            susdq: self.create_ata(&k, &self.p.susdq_mint.clone(), &TOKEN),
            kp,
        };
        if usdc > 0 {
            self.mint_to(&USDC, &u.usdc, usdc);
        }
        u
    }

    pub fn mint_with_usdc(&mut self, u: &User, amount: u64, min_out: u64) -> TxResult {
        let accounts = aequa::accounts::MintWithUsdc {
            user: u.key(),
            config: self.p.config,
            usdq_mint: self.p.usdq_mint,
            usdc_mint: USDC,
            user_usdc: u.usdc,
            user_usdq: u.usdq,
            buffer: self.p.buffer,
            reserve_fund: self.p.reserve_fund,
            protocol: self.p.protocol,
            aequa_stake: self.stake_if_any(&u.key()),
            aequa_pool: Some(self.p.aequa_pool),
            token_program: TOKEN,
        };
        let ix = ix(accounts, aequa::instruction::MintWithUsdc { amount, min_out });
        self.send(&[ix], &[&u.kp])
    }

    pub fn redeem_for_usdc(&mut self, u: &User, amount: u64, min_out: u64) -> TxResult {
        let accounts = aequa::accounts::RedeemForUsdc {
            user: u.key(),
            config: self.p.config,
            usdq_mint: self.p.usdq_mint,
            usdc_mint: USDC,
            user_usdq: u.usdq,
            user_usdc: u.usdc,
            buffer: self.p.buffer,
            reserve_fund: self.p.reserve_fund,
            protocol: self.p.protocol,
            aequa_stake: self.stake_if_any(&u.key()),
            aequa_pool: Some(self.p.aequa_pool),
            token_program: TOKEN,
        };
        let ix = ix(accounts, aequa::instruction::RedeemForUsdc { amount, min_out });
        self.send(&[ix], &[&u.kp])
    }

    pub fn stake_usdq(&mut self, u: &User, amount: u64) -> TxResult {
        let accounts = aequa::accounts::StakeUsdq {
            user: u.key(),
            config: self.p.config,
            usdq_vault: self.p.usdq_vault,
            usdq_mint: self.p.usdq_mint,
            susdq_mint: self.p.susdq_mint,
            user_usdq: u.usdq,
            user_susdq: u.susdq,
            susdq_assets: self.p.susdq_assets,
            token_program: TOKEN,
        };
        let ix = ix(accounts, aequa::instruction::StakeUsdq { amount, min_shares: 0 });
        self.send(&[ix], &[&u.kp])
    }

    pub fn start_usdq_cooldown(&mut self, u: &User, shares: u64) -> TxResult {
        let ix = self.start_usdq_cooldown_ix(u, shares);
        self.send(&[ix], &[&u.kp])
    }

    pub fn start_usdq_cooldown_ix(&self, u: &User, shares: u64) -> Instruction {
        let accounts = aequa::accounts::StartUsdqCooldown {
            user: u.key(),
            config: self.p.config,
            usdq_vault: self.p.usdq_vault,
            usdq_mint: self.p.usdq_mint,
            susdq_mint: self.p.susdq_mint,
            user_susdq: u.susdq,
            susdq_assets: self.p.susdq_assets,
            susdq_silo: self.p.susdq_silo,
            cooldown: Pdas::cooldown(&u.key()),
            token_program: TOKEN,
            system_program: SYSTEM,
        };
        ix(accounts, aequa::instruction::StartUsdqCooldown { shares })
    }

    pub fn claim_usdq(&mut self, u: &User) -> TxResult {
        let ix = self.claim_usdq_ix(u);
        self.send(&[ix], &[&u.kp])
    }

    pub fn claim_usdq_ix(&self, u: &User) -> Instruction {
        let accounts = aequa::accounts::ClaimUsdq {
            user: u.key(),
            config: self.p.config,
            usdq_mint: self.p.usdq_mint,
            user_usdq: u.usdq,
            susdq_silo: self.p.susdq_silo,
            cooldown: Pdas::cooldown(&u.key()),
            token_program: TOKEN,
        };
        ix(accounts, aequa::instruction::ClaimUsdq {})
    }

    pub fn report(&mut self, args: aequa::instructions::ReportArgs) -> TxResult {
        let accounts = aequa::accounts::ReportHedge { config: self.p.config, reporter: self.reporter.pubkey(), reserve: self.p.reserve };
        let reporter = clone(&self.reporter);
        self.send(&[ix(accounts, aequa::instruction::ReportHedge { args })], &[&reporter])
    }

    pub fn land_usdc(&mut self, caller: &Keypair) -> TxResult {
        let accounts = aequa::accounts::LandUsdc {
            config: self.p.config,
            reserve: self.p.reserve,
            usdc_mint: USDC,
            landing_authority: self.p.landing_authority,
            landing: self.p.landing,
            buffer: self.p.buffer,
            token_program: TOKEN,
        };
        self.send(&[ix(accounts, aequa::instruction::LandUsdc {})], &[caller])
    }

    /// A plain SPL mint at `addr` whose authority is the issuer (stands in for AEQUA and
    /// the pool's quote asset).
    pub fn create_mint(&mut self, addr: Pubkey, decimals: u8) {
        let mut data = vec![0u8; 82];
        data[0..4].copy_from_slice(&1u32.to_le_bytes());
        data[4..36].copy_from_slice(self.issuer.pubkey().as_ref());
        data[44] = decimals;
        data[45] = 1;
        let lamports = self.svm.minimum_balance_for_rent_exemption(82);
        self.svm.set_account(addr, Account { lamports, data, owner: TOKEN, executable: false, rent_epoch: 0 }).unwrap();
    }

    /// AEQUA set up against a constant-product pool holding `aequa` and `quote` base units.
    pub fn setup_aequa(&mut self, aequa_mint: Pubkey, quote_mint: Pubkey, pool_aequa: u64, pool_quote: u64) -> AequaWorld {
        let pool = Keypair::new().pubkey();
        let pool_aequa_vault = self.create_ata(&pool, &aequa_mint, &TOKEN);
        let pool_quote_vault = self.create_ata(&pool, &quote_mint, &TOKEN);
        self.mint_to(&aequa_mint, &pool_aequa_vault, pool_aequa);
        self.mint_to(&quote_mint, &pool_quote_vault, pool_quote);
        let w = AequaWorld {
            mint: aequa_mint,
            quote_mint,
            pool,
            pool_aequa_vault,
            pool_quote_vault,
            pool_vault: pda(&[seeds::AEQUA_POOL_VAULT]),
            slash_sale: pda(&[seeds::SLASH_SALE]),
            twap: pda(&[seeds::TWAP]),
        };
        let accounts = aequa::accounts::SetAequa {
            payer: self.payer.pubkey(),
            config: self.p.config,
            admin: self.admin.pubkey(),
            mint: w.mint,
            quote_mint: w.quote_mint,
            pool_aequa_vault: w.pool_aequa_vault,
            pool_quote_vault: w.pool_quote_vault,
            aequa_pool_vault: w.pool_vault,
            slash_sale: w.slash_sale,
            twap: w.twap,
            swap_authority: self.p.swap_authority,
            transit_aequa: Pdas::transit(&aequa_mint, &TOKEN),
            token_program: TOKEN,
            quote_token_program: TOKEN,
            associated_token_program: ATA,
            system_program: SYSTEM,
        };
        let args = aequa::instructions::AequaArgs { pool: w.pool, pool_aequa_vault: w.pool_aequa_vault, pool_quote_vault: w.pool_quote_vault, quote_feed: 0 };
        let (payer, admin) = (clone(&self.payer), clone(&self.admin));
        let r = self.send(&[ix(accounts, aequa::instruction::SetAequa { args })], &[&payer, &admin]);
        assert!(r.is_ok(), "set_aequa: {}", logs_of(&r));
        w
    }

    pub fn stake_aequa(&mut self, w: &AequaWorld, u: &Keypair, from: &Pubkey, amount: u64) -> TxResult {
        self.stake_aequa_min(w, u, from, amount, 0)
    }

    pub fn stake_aequa_min(&mut self, w: &AequaWorld, u: &Keypair, from: &Pubkey, amount: u64, min_shares: u64) -> TxResult {
        let accounts = aequa::accounts::StakeAequa {
            user: u.pubkey(),
            config: self.p.config,
            aequa_pool: self.p.aequa_pool,
            aequa_stake: Pdas::aequa_stake(&u.pubkey()),
            mint: w.mint,
            user_aequa: *from,
            aequa_pool_vault: w.pool_vault,
            token_program: TOKEN,
            system_program: SYSTEM,
        };
        self.send(&[ix(accounts, aequa::instruction::StakeAequa { amount, min_shares })], &[u])
    }

    pub fn start_aequa_unstake(&mut self, u: &Keypair) -> TxResult {
        let accounts = aequa::accounts::StartAequaUnstake { user: u.pubkey(), config: self.p.config, aequa_stake: Pdas::aequa_stake(&u.pubkey()) };
        self.send(&[ix(accounts, aequa::instruction::StartAequaUnstake {})], &[u])
    }

    pub fn withdraw_aequa(&mut self, w: &AequaWorld, u: &Keypair, to: &Pubkey) -> TxResult {
        let accounts = aequa::accounts::WithdrawAequa {
            user: u.pubkey(),
            config: self.p.config,
            aequa_pool: self.p.aequa_pool,
            aequa_stake: Pdas::aequa_stake(&u.pubkey()),
            mint: w.mint,
            user_aequa: *to,
            aequa_pool_vault: w.pool_vault,
            token_program: TOKEN,
        };
        self.send(&[ix(accounts, aequa::instruction::WithdrawAequa {})], &[u])
    }

    /// The keeper observes the pool.
    pub fn observe(&mut self, w: &AequaWorld) -> TxResult {
        let keeper = clone(&self.keeper);
        self.observe_as(w, &keeper)
    }

    pub fn observe_as(&mut self, w: &AequaWorld, signer: &Keypair) -> TxResult {
        let accounts = aequa::accounts::ObservePool {
            config: self.p.config,
            keeper: signer.pubkey(),
            twap: w.twap,
            pool_aequa_vault: w.pool_aequa_vault,
            pool_quote_vault: w.pool_quote_vault,
        };
        self.send(&[ix(accounts, aequa::instruction::ObservePool {})], &[signer])
    }

    /// Observations every three minutes for the next 36 minutes: a full TWAP window.
    pub fn observe_for_twap(&mut self, w: &AequaWorld) {
        for _ in 0..12 {
            self.warp(180);
            let r = self.observe(w);
            assert!(r.is_ok(), "observe: {}", logs_of(&r));
        }
    }

    fn stake_if_any(&self, owner: &Pubkey) -> Option<Pubkey> {
        let s = Pdas::aequa_stake(owner);
        self.exists(&s).then_some(s)
    }
}

impl Default for Env {
    fn default() -> Self {
        Self::new()
    }
}

pub fn ix(accounts: impl ToAccountMetas, data: impl InstructionData) -> Instruction {
    Instruction { program_id: aequa::ID, accounts: accounts.to_account_metas(None), data: data.data() }
}

pub fn clone(kp: &Keypair) -> Keypair {
    Keypair::try_from(kp.to_bytes().as_slice()).unwrap()
}

pub fn logs_of(r: &TxResult) -> String {
    match r {
        Ok(m) => m.logs.join("\n"),
        Err(e) => format!("{:?}\n{}", e.err, e.meta.logs.join("\n")),
    }
}

/// The program's error name in a failed transaction, from its log line.
pub fn error_name(r: &TxResult) -> Option<String> {
    let Err(e) = r else { return None };
    e.meta.logs.iter().find_map(|l| {
        let i = l.find("Error Code: ")?;
        let rest = &l[i + 12..];
        Some(rest.split('.').next().unwrap().to_string())
    })
}

#[track_caller]
pub fn assert_err(r: &TxResult, name: &str) {
    let got = error_name(r);
    assert_eq!(got.as_deref(), Some(name), "expected {name}, logs:\n{}", logs_of(r));
}

#[track_caller]
pub fn assert_ok(r: &TxResult) {
    assert!(r.is_ok(), "transaction failed:\n{}", logs_of(r));
}

/// Events the program emitted in a transaction, decoded as `E`.
pub fn events<E: AnchorSerialize + anchor_lang::AnchorDeserialize + Discriminator>(r: &TxResult) -> Vec<E> {
    let logs = match r {
        Ok(m) => &m.logs,
        Err(e) => &e.meta.logs,
    };
    logs.iter()
        .filter_map(|l| l.strip_prefix("Program data: "))
        .filter_map(|b| B64.decode(b).ok())
        .filter(|d| d.starts_with(E::DISCRIMINATOR))
        .filter_map(|d| E::deserialize(&mut &d[E::DISCRIMINATOR.len()..]).ok())
        .collect()
}

/// One feed in a Pyth Pro update: id, price mantissa, exponent, session and the feed's own
/// update time in microseconds.
#[derive(Clone, Copy, Debug)]
pub struct Feed {
    pub id: u32,
    pub price: i64,
    pub exponent: i16,
    pub session: i16,
    pub updated_us: u64,
}

pub const SPY: u32 = 1398;
pub const QQQ: u32 = 1363;

impl Feed {
    /// A price in dollars at Pyth's equity exponent (−5), updated at `t` seconds.
    pub fn at(id: u32, dollars: f64, session: i16, t: i64) -> Self {
        Self { id, price: (dollars * 1e5).round() as i64, exponent: -5, session, updated_us: t as u64 * 1_000_000 }
    }
}

/// A Pyth Pro "solana" envelope signed by `key`, carrying the properties the relay asks for:
/// price, exponent, confidence, feedUpdateTimestamp and marketSession.
pub fn pyth_envelope(key: &ed25519_dalek::SigningKey, timestamp: i64, channel: u8, feeds: &[Feed]) -> Vec<u8> {
    use ed25519_dalek::Signer as _;
    let mut p = 2_479_346_549u32.to_le_bytes().to_vec();
    p.extend_from_slice(&(timestamp as u64 * 1_000_000).to_le_bytes());
    p.push(channel);
    p.push(feeds.len() as u8);
    for f in feeds {
        p.extend_from_slice(&f.id.to_le_bytes());
        p.push(5);
        p.push(0);
        p.extend_from_slice(&f.price.to_le_bytes());
        p.push(4);
        p.extend_from_slice(&f.exponent.to_le_bytes());
        p.push(5);
        p.extend_from_slice(&1_000i64.to_le_bytes());
        p.push(12);
        p.push(1);
        p.extend_from_slice(&f.updated_us.to_le_bytes());
        p.push(9);
        p.extend_from_slice(&f.session.to_le_bytes());
    }
    let sig = key.sign(&p);
    let mut m = 2_182_742_457u32.to_le_bytes().to_vec();
    m.extend_from_slice(&sig.to_bytes());
    m.extend_from_slice(key.verifying_key().as_bytes());
    m.extend_from_slice(&(p.len() as u16).to_le_bytes());
    m.extend_from_slice(&p);
    m
}

pub const ED25519: Pubkey = Pubkey::from_str_const("Ed25519SigVerify111111111111111111111111111");
pub const INSTRUCTIONS: Pubkey = Pubkey::from_str_const("Sysvar1nstructions1111111111111111111111111");

/// The ed25519 instruction for an envelope at offset 12 of instruction `index`.
pub fn ed25519_ix(envelope: &[u8], index: u16) -> Instruction {
    let len = u16::from_le_bytes([envelope[100], envelope[101]]);
    let mut d = vec![1u8, 0];
    for v in [12 + 4, index, 12 + 68, index, 12 + 102, len, index] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    Instruction { program_id: ED25519, accounts: vec![], data: d }
}

pub fn pyth_accounts() -> aequa::accounts::PythPro {
    aequa::accounts::PythPro {
        pyth_program: PYTH_LAZER,
        pyth_storage: PYTH_STORAGE,
        pyth_treasury: PYTH_TREASURY,
        instructions: INSTRUCTIONS,
        system_program: SYSTEM,
    }
}

static HL_CLOCK: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);

impl Env {
    /// Lists SPYx (SPY's feed) or QQQx (QQQ's) with the launch bands and capacity.
    pub fn list(&mut self, mint: Pubkey, spot_cap: u64) {
        let accounts = aequa::accounts::AddCollateral {
            payer: self.payer.pubkey(),
            config: self.p.config,
            admin: self.admin.pubkey(),
            mint,
            collateral: Pdas::collateral(&mint),
            vault: Pdas::collateral_vault(&mint),
            swap_authority: self.p.swap_authority,
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
        let (payer, admin) = (clone(&self.payer), clone(&self.admin));
        let transit = self.ata_ix(&self.p.swap_authority, &mint, &TOKEN_2022);
        let r = self.send(&[transit, ix(accounts, aequa::instruction::AddCollateral { args })], &[&payer, &admin]);
        assert!(r.is_ok(), "list: {}", logs_of(&r));
    }

    /// A report landing now, both perps in session unless `market_live` says otherwise.
    pub fn report_now(&mut self, account_value: i64, credited: u64, withdrawn: u64, market_live: u8) -> TxResult {
        let hl_time = self.now() * 1_000 + HL_CLOCK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.report(aequa::instructions::ReportArgs {
            account_value,
            short_notional: [0; 8],
            market_live,
            credited_total: credited,
            withdrawn_total: withdrawn,
            hl_time,
        })
    }

    /// The remaining accounts `distribute` reads: each listed collateral, its mint and vault.
    pub fn collateral_accounts(&self) -> Vec<AccountMeta> {
        let c = self.config();
        (0..c.collateral_count as usize)
            .flat_map(|i| {
                let mint = c.collaterals[i];
                [
                    AccountMeta::new_readonly(Pdas::collateral(&mint), false),
                    AccountMeta::new_readonly(mint, false),
                    AccountMeta::new_readonly(Pdas::collateral_vault(&mint), false),
                ]
            })
            .collect()
    }

    /// An envelope with SPY and QQQ in `session`, fresh now.
    pub fn both_feeds(&self, session: i16) -> Vec<u8> {
        let t = self.now();
        pyth_envelope(&self.pyth, t, 3, &[Feed::at(SPY, 665.1234, session, t), Feed::at(QQQ, 598.42, session, t)])
    }

    pub fn distribute(&mut self, caller: &Keypair, envelope: Vec<u8>) -> TxResult {
        let accounts = aequa::accounts::Distribute {
            caller: caller.pubkey(),
            config: self.p.config,
            reserve: self.p.reserve,
            usdq_vault: self.p.usdq_vault,
            usdq_mint: self.p.usdq_mint,
            susdq_mint: self.p.susdq_mint,
            susdq_assets: self.p.susdq_assets,
            reserve_fund: self.p.reserve_fund,
            protocol: self.p.protocol,
            buffer: self.p.buffer,
            token_program: TOKEN,
            pyth: pyth_accounts(),
        };
        let ed = ed25519_ix(&envelope, 1);
        let mut program_ix = ix(accounts, aequa::instruction::Distribute { message: envelope, ed25519_index: 0 });
        program_ix.accounts.extend(self.collateral_accounts());
        self.send(&[ed, program_ix], &[caller])
    }
}

pub const CCTP_MT: Pubkey = Pubkey::from_str_const("CCTPV2Sm4AdWt5296sk4P66VBZ7bEhcARwFaaS9YPbeC");
pub const CCTP_TMM: Pubkey = Pubkey::from_str_const("CCTPV2vPZJS2u2BBsUoscuikbYjnpFmbFsvVuJdgUMQe");
/// Circle's CctpForwarder on HyperEVM, left-padded to 32 bytes.
pub const CCTP_FORWARDER: [u8; 32] = {
    let mut b = [0u8; 32];
    let a: [u8; 20] = [0xb2, 0x1d, 0x28, 0x1d, 0xed, 0xb1, 0x7a, 0xe5, 0xb5, 0x01, 0xf6, 0xaa, 0x82, 0x56, 0xfe, 0x38, 0xc4, 0xe4, 0x57, 0x57];
    let mut i = 0;
    while i < 20 {
        b[12 + i] = a[i];
        i += 1;
    }
    b
};

/// CCTP's accounts for a burn from the buffer towards HyperEVM.
pub struct Cctp {
    pub sender_authority: Pubkey,
    pub denylist: Pubkey,
    pub message_transmitter: Pubkey,
    pub token_messenger: Pubkey,
    pub remote_token_messenger: Pubkey,
    pub token_minter: Pubkey,
    pub local_token: Pubkey,
    pub event_authority: Pubkey,
}

impl Cctp {
    pub fn for_owner(owner: &Pubkey) -> Self {
        let tmm = |seeds: &[&[u8]]| Pubkey::find_program_address(seeds, &CCTP_TMM).0;
        Self {
            sender_authority: tmm(&[b"sender_authority"]),
            denylist: tmm(&[b"denylist_account", owner.as_ref()]),
            message_transmitter: Pubkey::find_program_address(&[b"message_transmitter"], &CCTP_MT).0,
            token_messenger: tmm(&[b"token_messenger"]),
            remote_token_messenger: tmm(&[b"remote_token_messenger", b"19"]),
            token_minter: tmm(&[b"token_minter"]),
            local_token: tmm(&[b"local_token", USDC.as_ref()]),
            event_authority: tmm(&[b"__event_authority"]),
        }
    }
}

/// The 56-byte hook that sends a burn on to a HyperCore account's perp balance.
pub fn hypercore_hook(recipient: [u8; 20]) -> [u8; 56] {
    let mut h = [0u8; 56];
    h[..12].copy_from_slice(b"cctp-forward");
    h[28..32].copy_from_slice(&24u32.to_be_bytes());
    h[32..52].copy_from_slice(&recipient);
    h
}

impl Env {
    /// Circle's CCTP V2 programs and their mainnet state.
    pub fn load_cctp(&mut self) {
        self.svm.add_program_from_file(CCTP_MT, fixture("cctp_message_transmitter_v2.so")).unwrap();
        self.svm.add_program_from_file(CCTP_TMM, fixture("cctp_token_messenger_minter_v2.so")).unwrap();
        for (name, addr) in [
            ("cctp_message_transmitter.json", "W1k5ijkaSTo5iA5zChNpfzcy796fLhkBxfmJuR8W8HU"),
            ("cctp_token_messenger.json", "AawthJCGRmggpfv9MMWV6Jmo9cue4gL9wUZgRBShg58W"),
            ("cctp_remote_token_messenger_19.json", "3WG7G3vPxz4a6bdBb9Xic2YL4QM84ru1YcVYskcweVHY"),
            ("cctp_token_minter.json", "E1bQJ8eMMn3zmeSewW3HQ8zmJr7KR75JonbwAtWx2bux"),
            ("cctp_local_token_usdc.json", "CRBBbuLCyrkQy4dCTHxqstSmDQv4ajBeUVb9qUdMVaP1"),
        ] {
            self.svm.set_account(addr.parse().unwrap(), load_account(name)).unwrap();
        }
    }

    /// The route to the hedge account `recipient` on HyperCore.
    pub fn set_route(&mut self, recipient: [u8; 20]) -> TxResult {
        self.admin_ix(aequa::instruction::SetBridgeRoute { args: BridgeArgs { recipient, max_fee_bps: 1, max_fee_flat: 1_000_000 } })
    }

    pub fn bridge_out(&mut self, keeper: &Keypair, amount: u64, max_fee: u64, fast: bool) -> (TxResult, Pubkey) {
        let event = Keypair::new();
        let c = Cctp::for_owner(&self.p.buffer);
        let accounts = aequa::accounts::BridgeOut {
            keeper: keeper.pubkey(),
            config: self.p.config,
            reserve: self.p.reserve,
            buffer: self.p.buffer,
            usdc_mint: USDC,
            message_sent_event_data: event.pubkey(),
            sender_authority: c.sender_authority,
            denylist: c.denylist,
            message_transmitter: c.message_transmitter,
            token_messenger: c.token_messenger,
            remote_token_messenger: c.remote_token_messenger,
            token_minter: c.token_minter,
            local_token: c.local_token,
            cctp_event_authority: c.event_authority,
            message_transmitter_program: CCTP_MT,
            token_messenger_minter_program: CCTP_TMM,
            token_program: TOKEN,
            system_program: SYSTEM,
        };
        let ix = ix(accounts, aequa::instruction::BridgeOut { amount, max_fee, fast });
        (self.send(&[ix], &[keeper, &event]), event.pubkey())
    }
}

/// A Jupiter route captured from Jupiter's API (`tests/fixtures/jupiter_*.json`).
pub struct Route {
    pub accounts: Vec<AccountMeta>,
    pub data: Vec<u8>,
    pub in_amount: u64,
    pub out_amount: u64,
    pub block_time: i64,
}

pub const COMPUTE_BUDGET: Pubkey = Pubkey::from_str_const("ComputeBudget111111111111111111111111111111");
pub const JUP_MINT: Pubkey = Pubkey::from_str_const("JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN");

pub fn compute_budget(units: u32) -> Instruction {
    Instruction { program_id: COMPUTE_BUDGET, accounts: vec![], data: [vec![2u8], units.to_le_bytes().to_vec()].concat() }
}

/// A price from the snapshot's Jupiter price read: `spyx` (SPY's own price) or `jup`.
pub fn snapshot_price(which: &str) -> f64 {
    let v: serde_json::Value = serde_json::from_slice(&fs::read(fixture("jupiter_price.json")).unwrap()).unwrap();
    match which {
        "spyx" => v["spyx"]["stockData"]["price"].as_f64().unwrap(),
        other => v[other]["usdPrice"].as_f64().unwrap(),
    }
}

impl Env {
    /// Loads a captured route: Jupiter and the AMM programs, and every account it reads except
    /// the mints this world already holds (whose issuer key can mint).
    pub fn load_route(&mut self, name: &str) -> Route {
        let v: serde_json::Value = serde_json::from_slice(&fs::read(fixture(&format!("jupiter_{name}.json"))).unwrap()).unwrap();
        for p in v["programs"].as_array().unwrap() {
            let id: Pubkey = p.as_str().unwrap().parse().unwrap();
            if id == TOKEN || id == TOKEN_2022 {
                continue;
            }
            self.svm.add_program_from_file(id, fixture(&format!("prog_{id}.so"))).unwrap();
        }
        for (key, a) in v["accounts"].as_object().unwrap() {
            let addr: Pubkey = key.parse().unwrap();
            if self.svm.get_account(&addr).is_some_and(|a| a.lamports > 0) && [USDC, SPYX, QQQX, JUP_MINT].contains(&addr) {
                continue;
            }
            let mut acc = Account {
                lamports: a["lamports"].as_u64().unwrap(),
                data: B64.decode(a["data"].as_str().unwrap()).unwrap(),
                owner: a["owner"].as_str().unwrap().parse().unwrap(),
                executable: false,
                rent_epoch: 0,
            };
            if addr == JUP_MINT {
                // This world's issuer mints the stand-in for AEQUA.
                acc.data[0..4].copy_from_slice(&1u32.to_le_bytes());
                acc.data[4..36].copy_from_slice(self.issuer.pubkey().as_ref());
            }
            self.svm.set_account(addr, acc).unwrap();
        }
        let ix = &v["swapInstruction"];
        Route {
            accounts: ix["accounts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| AccountMeta { pubkey: a["pubkey"].as_str().unwrap().parse().unwrap(), is_signer: false, is_writable: a["isWritable"].as_bool().unwrap() })
                .collect(),
            data: B64.decode(ix["data"].as_str().unwrap()).unwrap(),
            in_amount: v["quote"]["inAmount"].as_str().unwrap().parse().unwrap(),
            out_amount: v["quote"]["outAmount"].as_str().unwrap().parse().unwrap(),
            block_time: v["blockTime"].as_i64().unwrap(),
        }
    }

    /// Rewrites a program account in place.
    pub fn edit<T: AccountDeserialize + anchor_lang::AccountSerialize>(&mut self, addr: &Pubkey, f: impl FnOnce(&mut T)) {
        let mut acc = self.svm.get_account(addr).unwrap();
        let mut v = T::try_deserialize(&mut acc.data.as_slice()).unwrap();
        f(&mut v);
        let mut data = Vec::new();
        v.try_serialize(&mut data).unwrap();
        acc.data[..data.len()].copy_from_slice(&data);
        self.svm.set_account(*addr, acc).unwrap();
    }
}
