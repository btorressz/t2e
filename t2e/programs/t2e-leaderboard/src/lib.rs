use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

declare_id!("GHTyeny1bNPntWknAJwxu2YWJ9GUyRL57PjtGeaapS9h");

#[program]
pub mod t2e_leaderboard {
    use super::*;

    /// Initializes the leaderboard state.
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        let leaderboard = &mut ctx.accounts.leaderboard;
        leaderboard.last_update = Clock::get()?.unix_timestamp;
        leaderboard.traders = Vec::new();
        leaderboard.ranking_scores = Vec::new();
        leaderboard.emergency_pause = false;
        Ok(())
    }

    /// Records a trade by updating the trader's stats.
    ///
    /// - Updates total volume, weighted average execution time, and P&L.
    /// - Prevents rapid-fire trades to mitigate spam (enforcing a 10-second gap).
    pub fn record_trade(
        ctx: Context<RecordTrade>,
        volume: u64,
        execution_time: u64,
        pnl: i64,
    ) -> Result<()> {
        let trader_stats = &mut ctx.accounts.trader_stats;
        let current_time = Clock::get()?.unix_timestamp;
        
        // Prevent trade spam: if a trade was made less than 10 seconds ago, reject.
        if trader_stats.trade_count > 0 && current_time - trader_stats.last_trade < 10 {
            return Err(ErrorCode::TradeSpamDetected.into());
        }

        // Update trading volume.
        trader_stats.total_volume = trader_stats
            .total_volume
            .checked_add(volume)
            .ok_or(ErrorCode::Overflow)?;

        // Update average execution time using a simple weighted average.
        let current_count = trader_stats.trade_count;
        let new_count = current_count.checked_add(1).ok_or(ErrorCode::Overflow)?;
        trader_stats.average_execution_time = ((trader_stats.average_execution_time * current_count)
            .checked_add(execution_time)
            .ok_or(ErrorCode::Overflow)?)
            .checked_div(new_count)
            .ok_or(ErrorCode::Overflow)?;
        trader_stats.trade_count = new_count;

        // Update profit & loss.
        trader_stats.pnl = trader_stats
            .pnl
            .checked_add(pnl)
            .ok_or(ErrorCode::Overflow)?;

        trader_stats.last_trade = current_time;

        Ok(())
    }

    /// Updates the leaderboard ranking based on trader stats.
    ///
    /// Uses a composite score calculation:
    /// - Base score: total_volume / (average_execution_time + 1)
    /// - Bonus: positive P&L and a staking bonus (staked_amount / 1000)
    ///
    /// Enforces a minimum 10-minute interval between updates.
    pub fn update_leaderboard(
        ctx: Context<UpdateLeaderboard>,
        trader_stats_list: Vec<TraderStatsInput>,
    ) -> Result<()> {
        let leaderboard = &mut ctx.accounts.leaderboard;
        let current_time = Clock::get()?.unix_timestamp;

        if current_time - leaderboard.last_update < 600 {
            return Err(ErrorCode::UpdateTooSoon.into());
        }

        // Compute boosted score for each trader.
        let mut ranked_traders: Vec<RankedTrader> = trader_stats_list
            .into_iter()
            .map(|ts| {
                let base_score = ts.total_volume
                    .checked_div(ts.average_execution_time.checked_add(1).unwrap())
                    .unwrap_or(0);
                let pnl_score = if ts.pnl > 0 { ts.pnl as u64 } else { 0 };
                let staking_bonus = ts.staked_amount / 1000; // Bonus per 1000 $T2E staked
                let score = base_score
                    .checked_add(pnl_score)
                    .unwrap_or(0)
                    .checked_add(staking_bonus)
                    .unwrap_or(0);
                RankedTrader {
                    trader: ts.trader,
                    score,
                }
            })
            .collect();

        // Sort traders in descending order based on their boosted score.
        ranked_traders.sort_by(|a, b| b.score.cmp(&a.score));

        // Update leaderboard with ordered traders and their corresponding scores.
        leaderboard.traders = ranked_traders.iter().map(|rt| rt.trader).collect();
        leaderboard.ranking_scores = ranked_traders.iter().map(|rt| rt.score).collect();
        leaderboard.last_update = current_time;
        Ok(())
    }

    /// Distributes $T2E rewards to the top N traders.
    ///
    /// Rewards are scaled proportionally to each trader's ranking score.
    /// A reward halving mechanism reduces the total reward pool over time.
    ///
    /// Expects each top trader’s token account to be provided via `remaining_accounts`.
  pub fn distribute_rewards<'info>(
    ctx: Context<'_, '_, '_, 'info, DistributeRewards<'info>>,
    top_n: u64,
    reward_amount: u64, // Total reward pool amount.
) -> Result<()> {
    let leaderboard = &ctx.accounts.leaderboard;

    // Check for emergency pause.
    if leaderboard.emergency_pause {
        return Err(ErrorCode::EmergencyPaused.into());
    }

    // Determine the halving factor on a monthly basis (halving every 6 months).
    let current_time = Clock::get()?.unix_timestamp;
    let current_epoch = current_time / (30 * 24 * 60 * 60);
    let halving_periods = current_epoch / 6;
    let halving_factor = 2_u64.pow(halving_periods as u32);
    let adjusted_reward = reward_amount.checked_div(halving_factor).unwrap_or(1);

    // Compute total score among the top N traders.
    let top_n_usize = top_n as usize;
    let num_traders = leaderboard.traders.len().min(top_n_usize);
    let mut total_score: u64 = 0;
    for score in leaderboard.ranking_scores.iter().take(num_traders) {
        total_score = total_score.checked_add(*score).ok_or(ErrorCode::Overflow)?;
    }
    if total_score == 0 {
        return Err(ErrorCode::NoValidScores.into());
    }

    // Loop over the top traders and distribute rewards scaled by their score.
    for (i, trader) in leaderboard.traders.iter().take(num_traders).enumerate() {
        let score = leaderboard.ranking_scores.get(i).unwrap();
        let trader_reward = ( (*score as u128)
            .checked_mul(adjusted_reward as u128)
            .ok_or(ErrorCode::Overflow)?
            .checked_div(total_score as u128)
            .ok_or(ErrorCode::Overflow)? ) as u64;

        let trader_token_account = ctx
            .remaining_accounts
            .iter()
            .find(|acc| acc.key == trader)
            .ok_or(ErrorCode::TraderTokenAccountNotFound)?;

        // Create transfer accounts
        let cpi_accounts = Transfer {
            from: ctx.accounts.reward_vault.to_account_info(),
            to: trader_token_account.to_account_info(),
            authority: ctx.accounts.reward_authority.to_account_info(),
        };

        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts),
            trader_reward,
        )?;
    }
    Ok(())
}
    /// Allows traders to stake $T2E tokens.
    ///
    /// Tokens are transferred to a staking vault and the staked amount is updated.
    pub fn stake_tokens(ctx: Context<StakeTokens>, amount: u64) -> Result<()> {
        let trader_stats = &mut ctx.accounts.trader_stats;
        let token_account = &mut ctx.accounts.trader_token_account;

        let cpi_accounts = Transfer {
            from: token_account.to_account_info(),
            to: ctx.accounts.staking_vault.to_account_info(),
            authority: ctx.accounts.trader.to_account_info(),
        };

        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts),
            amount,
        )?;

        trader_stats.staked_amount = trader_stats
            .staked_amount
            .checked_add(amount)
            .ok_or(ErrorCode::Overflow)?;
        Ok(())
    }

    /// Calculates a fee discount based on the staked amount.
    ///
    /// For example, discount scales from 0% to 50% for staking between 0 and 10,000 $T2E.
    pub fn calculate_fee_discount(ctx: Context<CalculateFeeDiscount>) -> Result<()> {
        let trader_stats = &mut ctx.accounts.trader_stats;
        trader_stats.fee_discount = std::cmp::min((trader_stats.staked_amount / 200) as u8, 50);
        Ok(())
    }

    /// Takes a snapshot of the current leaderboard ranking.
    ///
    /// Useful for creating daily, weekly, or monthly leaderboard history.
    pub fn snapshot_leaderboard(ctx: Context<SnapshotLeaderboard>) -> Result<()> {
        let leaderboard_history = &mut ctx.accounts.leaderboard_history;
        let leaderboard = &ctx.accounts.leaderboard;
        leaderboard_history.past_rankings.push((Clock::get()?.unix_timestamp, leaderboard.traders.clone()));
        Ok(())
    }

    /// Allows an admin to pause or unpause reward distribution in emergencies.
    pub fn admin_pause_rewards(ctx: Context<AdminPauseRewards>, paused: bool) -> Result<()> {
        let leaderboard = &mut ctx.accounts.leaderboard;
        leaderboard.emergency_pause = paused;
        Ok(())
    }
}

/// Input structure for trader stats used during leaderboard updates.
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct TraderStatsInput {
    pub trader: Pubkey,
    pub total_volume: u64,
    pub average_execution_time: u64,
    pub pnl: i64,
    pub staked_amount: u64, // Added for leaderboard boost
}

/// Helper struct for ranking a trader.
pub struct RankedTrader {
    pub trader: Pubkey,
    pub score: u64,
}

#[account]
pub struct TraderStats {
    pub trader: Pubkey,
    pub total_volume: u64,
    pub average_execution_time: u64,
    pub trade_count: u64,
    pub pnl: i64,
    pub staked_amount: u64,
    pub fee_discount: u8, // Percentage discount (0-50)
    pub last_trade: i64,
}

#[account]
pub struct Leaderboard {
    pub traders: Vec<Pubkey>,
    pub ranking_scores: Vec<u64>, // Parallel array holding ranking scores.
    pub last_update: i64,
    pub emergency_pause: bool,
}

#[account]
pub struct LeaderboardHistory {
    // Stores snapshots of leaderboard rankings: (timestamp, list of trader Pubkeys)
    pub past_rankings: Vec<(i64, Vec<Pubkey>)>,
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(init, payer = user, space = 8 + 8 + (4 + 32 * 1000) + (4 + 8 * 1000) + 1)]
    pub leaderboard: Account<'info, Leaderboard>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RecordTrade<'info> {
    #[account(mut, has_one = trader)]
    pub trader_stats: Account<'info, TraderStats>,
    pub trader: Signer<'info>,
}

#[derive(Accounts)]
pub struct UpdateLeaderboard<'info> {
    #[account(mut)]
    pub leaderboard: Account<'info, Leaderboard>,
}

#[derive(Accounts)]
pub struct DistributeRewards<'info> {
    #[account(mut)]
    pub leaderboard: Account<'info, Leaderboard>,
    #[account(mut)]
    pub reward_vault: Account<'info, TokenAccount>,
    /// CHECK: Authority for reward vault transfers.
    pub reward_authority: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct StakeTokens<'info> {
    #[account(mut, has_one = trader)]
    pub trader_stats: Account<'info, TraderStats>,
    #[account(mut)]
    pub trader_token_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub staking_vault: Account<'info, TokenAccount>,
    pub trader: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct CalculateFeeDiscount<'info> {
    #[account(mut, has_one = trader)]
    pub trader_stats: Account<'info, TraderStats>,
    pub trader: Signer<'info>,
}

#[derive(Accounts)]
pub struct SnapshotLeaderboard<'info> {
    #[account(mut)]
    pub leaderboard: Account<'info, Leaderboard>,
    #[account(init_if_needed, payer = admin, space = 8 + (4 + 8 + (4 + 32 * 1000)))]
    pub leaderboard_history: Account<'info, LeaderboardHistory>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct AdminPauseRewards<'info> {
    #[account(mut)]
    pub leaderboard: Account<'info, Leaderboard>,
    pub admin: Signer<'info>,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Arithmetic overflow occurred.")]
    Overflow,
    #[msg("Leaderboard update attempted too soon; wait at least 10 minutes.")]
    UpdateTooSoon,
    #[msg("Trader token account not found among provided accounts.")]
    TraderTokenAccountNotFound,
    #[msg("Trade spam detected: please wait before making another trade.")]
    TradeSpamDetected,
    #[msg("Emergency pause is active; operation aborted.")]
    EmergencyPaused,
    #[msg("No valid ranking scores found for reward distribution.")]
    NoValidScores,
}
