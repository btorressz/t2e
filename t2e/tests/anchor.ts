import * as anchor from "@coral-xyz/anchor";
import BN from "bn.js";
import assert from "assert";
import * as web3 from "@solana/web3.js";
import type { T2eLeaderboard } from "../target/types/t2e_leaderboard";

describe("Trade-to-Earn Leaderboard Token ($T2E)", () => {
  // Configure the client to use the local cluster
  anchor.setProvider(anchor.AnchorProvider.env());

  const program = anchor.workspace.T2eLeaderboard as anchor.Program<T2eLeaderboard>;
  
  let leaderboardAccount = new web3.Keypair();
  let traderAccount = new web3.Keypair();
  let rewardVault = new web3.Keypair();
  let stakingVault = new web3.Keypair();
  let traderTokenAccount = new web3.Keypair();

  it("Initializes the leaderboard", async () => {
    const txHash = await program.methods
      .initialize()
      .accounts({
        leaderboard: leaderboardAccount.publicKey,
        user: program.provider.publicKey,
        systemProgram: web3.SystemProgram.programId,
      })
      .signers([leaderboardAccount])
      .rpc();

    console.log(`✅ Leaderboard initialized: ${txHash}`);
    await program.provider.connection.confirmTransaction(txHash);

    const leaderboard = await program.account.leaderboard.fetch(
      leaderboardAccount.publicKey
    );
    assert(leaderboard.traders.length === 0);
    assert(leaderboard.lastUpdate.gt(new BN(0))); // FIXED: BN comparison
  });

  it("Records a trade", async () => {
    const volume = new BN(5000);
    const executionTime = new BN(250);
    const pnl = new BN(200);

    const txHash = await program.methods
      .recordTrade(volume, executionTime, pnl)
      .accounts({
        traderStats: traderAccount.publicKey,
        trader: program.provider.publicKey,
      })
      .signers([traderAccount])
      .rpc();

    console.log(`✅ Trade recorded: ${txHash}`);
    await program.provider.connection.confirmTransaction(txHash);

    const traderStats = await program.account.traderStats.fetch(
      traderAccount.publicKey
    );
    assert(traderStats.totalVolume.eq(volume));
    assert(traderStats.averageExecutionTime.eq(executionTime));
    assert(traderStats.pnl.eq(pnl));
  });

  it("Updates the leaderboard", async () => {
    const traderStatsList = [
      {
        trader: traderAccount.publicKey,
        totalVolume: new BN(5000),
        averageExecutionTime: new BN(250),
        pnl: new BN(200),
        stakedAmount: new BN(0),
      },
    ];

    const txHash = await program.methods
      .updateLeaderboard(traderStatsList)
      .accounts({
        leaderboard: leaderboardAccount.publicKey,
      })
      .rpc();

    console.log(`✅ Leaderboard updated: ${txHash}`);
    await program.provider.connection.confirmTransaction(txHash);

    const leaderboard = await program.account.leaderboard.fetch(
      leaderboardAccount.publicKey
    );
    assert(leaderboard.traders.length > 0);
    assert(leaderboard.traders[0].toBase58() === traderAccount.publicKey.toBase58());
  });

  it("Distributes rewards", async () => {
    const topN = new BN(1);
    const rewardAmount = new BN(100);

    const txHash = await program.methods
      .distributeRewards(topN, rewardAmount)
      .accounts({
        leaderboard: leaderboardAccount.publicKey,
        rewardVault: rewardVault.publicKey,
        rewardAuthority: program.provider.publicKey,
        tokenProgram: web3.PublicKey.default, // FIXED: Removed 'spl' reference
      })
      .rpc();

    console.log(`✅ Rewards distributed: ${txHash}`);
    await program.provider.connection.confirmTransaction(txHash);
  });

  it("Allows traders to stake tokens", async () => {
    const stakeAmount = new BN(1000);

    const txHash = await program.methods
      .stakeTokens(stakeAmount)
      .accounts({
        traderStats: traderAccount.publicKey,
        traderTokenAccount: traderTokenAccount.publicKey,
        stakingVault: stakingVault.publicKey,
        trader: program.provider.publicKey,
        tokenProgram: web3.PublicKey.default, // FIXED: Removed 'spl' reference
      })
      .rpc();

    console.log(`✅ Tokens staked: ${txHash}`);
    await program.provider.connection.confirmTransaction(txHash);

    const traderStats = await program.account.traderStats.fetch(
      traderAccount.publicKey
    );
    assert(traderStats.stakedAmount.eq(stakeAmount));
  });

  it("Calculates fee discounts based on staking", async () => {
    const txHash = await program.methods
      .calculateFeeDiscount()
      .accounts({
        traderStats: traderAccount.publicKey,
        trader: program.provider.publicKey,
      })
      .rpc();

    console.log(`✅ Fee discount calculated: ${txHash}`);
    await program.provider.connection.confirmTransaction(txHash);

    const traderStats = await program.account.traderStats.fetch(
      traderAccount.publicKey
    );
    assert(traderStats.feeDiscount >= 0 && traderStats.feeDiscount <= 50);
  });

  it("Takes a snapshot of the leaderboard history", async () => {
    const leaderboardHistoryAccount = new web3.Keypair();

    const txHash = await program.methods
      .snapshotLeaderboard()
      .accounts({
        leaderboard: leaderboardAccount.publicKey,
        leaderboardHistory: leaderboardHistoryAccount.publicKey,
        admin: program.provider.publicKey,
        systemProgram: web3.SystemProgram.programId,
      })
      .signers([leaderboardHistoryAccount])
      .rpc();

    console.log(`✅ Leaderboard snapshot saved: ${txHash}`);
    await program.provider.connection.confirmTransaction(txHash);
  });

  it("Pauses leaderboard rewards in case of emergency", async () => {
    const txHash = await program.methods
      .adminPauseRewards(true)
      .accounts({
        leaderboard: leaderboardAccount.publicKey,
        admin: program.provider.publicKey,
      })
      .rpc();

    console.log(`✅ Rewards paused: ${txHash}`);
    await program.provider.connection.confirmTransaction(txHash);

    const leaderboard = await program.account.leaderboard.fetch(
      leaderboardAccount.publicKey
    );
    assert(leaderboard.emergencyPause === true);
  });

  it("Resumes leaderboard rewards", async () => {
    const txHash = await program.methods
      .adminPauseRewards(false)
      .accounts({
        leaderboard: leaderboardAccount.publicKey,
        admin: program.provider.publicKey,
      })
      .rpc();

    console.log(`✅ Rewards resumed: ${txHash}`);
    await program.provider.connection.confirmTransaction(txHash);

    const leaderboard = await program.account.leaderboard.fetch(
      leaderboardAccount.publicKey
    );
    assert(leaderboard.emergencyPause === false);
  });
});
