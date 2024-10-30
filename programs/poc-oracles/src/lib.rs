use std::ops::Mul;

use anchor_lang::{prelude::*, solana_program::native_token::LAMPORTS_PER_SOL};
use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;
use rust_decimal::prelude::*;

use switchboard_on_demand::on_demand::accounts::pull_feed::PullFeedAccountData;
declare_id!("BqeULWKoq51Ts7uoiqk1Sgc3PXjiHrMTvpfzPRh5aeLr");
pub const MAXIMUM_AGE: u64 = 30; // 30 seconds

#[program]
pub mod oracle_priority {

    use super::*;

    pub fn initialize(ctx: Context<Initialize>, vault_type_name: String) -> Result<()> {
        let oracle_info = &mut ctx.accounts.oracle_info;

        // Initialize with default settings
        oracle_info.vault_type = ctx.accounts.vault_type.key();
        oracle_info.oracle_pyth = [0u8; 32];
        oracle_info.oracle_switchboard = Pubkey::default();
        oracle_info.priority_pyth = -1; // Disabled by default
        oracle_info.priority_switchboard = -1; // Disabled by default
        oracle_info.vault_type_name = vault_type_name;
        oracle_info.recent_price = 0;
        oracle_info.last_update = 0;

        Ok(())
    }

    pub fn update_priority(
        ctx: Context<UpdatePriority>,
        pyth_priority: i8,
        switchboard_priority: i8,
    ) -> Result<()> {
        let oracle_info = &mut ctx.accounts.oracle_info;

        // Validate priorities
        require!(
            check_oracle_priorities(pyth_priority, switchboard_priority),
            OracleError::InvalidPriorities
        );

        oracle_info.priority_pyth = pyth_priority;
        oracle_info.priority_switchboard = switchboard_priority;

        Ok(())
    }

    pub fn update_oracles(
        ctx: Context<UpdateOracles>,
        pyth_oracle: [u8; 32],
        switchboard_oracle: Pubkey,
    ) -> Result<()> {
        let oracle_info = &mut ctx.accounts.oracle_info;

        oracle_info.oracle_pyth = pyth_oracle;
        oracle_info.oracle_switchboard = switchboard_oracle;

        Ok(())
    }

    pub fn get_price(ctx: Context<GetPrice>) -> Result<()> {
        let oracle_info = &mut ctx.accounts.oracle_info;
        let clock = Clock::get()?;

        let mut prices: [Option<Decimal>; 2] = [None, None];

        // Get Pyth price if enabled
        if oracle_info.priority_pyth >= 0 {
            if let Ok(p) = load_pyth(&ctx.accounts.pyth_price_info, oracle_info.oracle_pyth) {
                msg!(
                    "Assigning pyth price to index {}",
                    oracle_info.priority_pyth
                );
                prices[oracle_info.priority_pyth as usize] = Some(p)
            }
        }

        // Get Switchboard price if enabled
        if oracle_info.priority_switchboard >= 0 {
            if let Ok(p) = load_switchboard(&ctx.accounts.switchboard_feed_info) {
                msg!(
                    "Assigning switchboard price to index {}",
                    oracle_info.priority_switchboard
                );
                prices[oracle_info.priority_switchboard as usize] = Some(p)
            }
        }
        // Use first available price based on priority
        if let Some(price) = prices.iter().flatten().next() {
            oracle_info.recent_price = price.to_account();
            oracle_info.last_update = clock.unix_timestamp as u64;
            Ok(())
        } else {
            Err(OracleError::NoPriceAvailable.into())
        }
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        init,
        payer = payer,
        space = 8 + 32 + 32 + 32 + 2 + 32 + 8 + 8 + 8,
        seeds = [vault_type.key().as_ref(), b"Oracle"],
        bump
    )]
    pub oracle_info: Account<'info, OracleInfo>,

    /// CHECK: This is just used as a reference for PDA
    pub vault_type: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdatePriority<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [oracle_info.vault_type.as_ref(), b"Oracle"],
        bump
    )]
    pub oracle_info: Account<'info, OracleInfo>,
}

#[derive(Accounts)]
pub struct UpdateOracles<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [oracle_info.vault_type.as_ref(), b"Oracle"],
        bump
    )]
    pub oracle_info: Account<'info, OracleInfo>,
}

#[derive(Accounts)]
pub struct GetPrice<'info> {
    #[account(
        mut,
        seeds = [oracle_info.vault_type.as_ref(), b"Oracle"],
        bump
    )]
    pub oracle_info: Account<'info, OracleInfo>,

    pub pyth_price_info: Account<'info, PriceUpdateV2>,

    /// CHECK: Validated in program logic
    pub switchboard_feed_info: AccountInfo<'info>,
}

#[account]
pub struct OracleInfo {
    pub vault_type: Pubkey,
    pub oracle_pyth: [u8; 32],
    pub oracle_switchboard: Pubkey,
    pub priority_pyth: i8,
    pub priority_switchboard: i8,
    pub vault_type_name: String,
    pub recent_price: u128,
    pub last_update: u64,
}

#[error_code]
pub enum OracleError {
    #[msg("Invalid oracle priorities configuration")]
    InvalidPriorities,
    #[msg("No price available from configured oracles")]
    NoPriceAvailable,
}

fn check_oracle_priorities(pyth: i8, switchboard: i8) -> bool {
    // At least one oracle must be enabled
    if pyth < 0 && switchboard < 0 {
        return false;
    }

    // Validate priority ranges
    if (pyth >= 0 && pyth > 2) || (switchboard >= 0 && switchboard > 2) {
        return false;
    }

    // Check for duplicate priorities
    if pyth >= 0 && switchboard >= 0 && pyth == switchboard {
        return false;
    }

    true
}

fn load_switchboard<'a>(oracle_switchboard: &AccountInfo<'a>) -> Result<Decimal> {
    let feed_account = oracle_switchboard.data.borrow();
    let feed = PullFeedAccountData::parse(feed_account).unwrap();

    msg!("Switchboard unpack start");

    let price = feed.value().unwrap();

    Ok(price)
}

fn load_pyth<'a>(oracle_pyth: &Account<'a, PriceUpdateV2>, feed_id: [u8; 32]) -> Result<Decimal> {
    let current_timestamp = Clock::get()?;
    let price = oracle_pyth.get_price_no_older_than(&current_timestamp, MAXIMUM_AGE, &feed_id)?;
    msg!("Pyth price was: {:?}", price);
    let p = Decimal::from_i128_with_scale(price.price as i128, price.exponent.mul(-1) as u32);
    msg!("Pyth decimal was: {:?}", p);
    Ok(p)
}

// A wad is a decimal number with 18 digits of precision
const WAD: u128 = 1_000_000_000_000_000_000_u128;

pub trait NeptuneTraits {
    fn from_account(a: u128) -> Decimal;
    fn from_lamport_offset(a: u64) -> Decimal;
    fn to_account(&self) -> u128;
    fn lamports_per_sol() -> Self;
}

impl NeptuneTraits for Decimal {
    fn from_account(a: u128) -> Decimal {
        let seb = Decimal::from_u128(WAD).unwrap();
        Decimal::from_u128(a).unwrap() / seb
    }
    fn from_lamport_offset(a: u64) -> Decimal {
        let lamports_per_sol = Decimal::from_u64(LAMPORTS_PER_SOL).unwrap();
        Decimal::from_u64(a).unwrap() / lamports_per_sol
    }

    fn to_account(&self) -> u128 {
        let seb = Decimal::from_u128(WAD).unwrap();
        (seb * self).to_u128().unwrap()
    }

    fn lamports_per_sol() -> Self {
        Decimal::from_u64(1_000_000_000).unwrap()
    }
}
