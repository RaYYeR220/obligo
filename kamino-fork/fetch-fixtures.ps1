# Dumps the real mainnet accounts and the KLend program the fork test loads into LiteSVM.
# Run once from PowerShell before `cargo test`:  ./fetch-fixtures.ps1
# Nothing it writes is committed — see .gitignore. The KLend program is BUSL-1.1; it is fetched
# from its own on-chain deployment at test time, never vendored into the repo.
$ErrorActionPreference = "Stop"
$fx = Join-Path $PSScriptRoot "fixtures"
New-Item -ItemType Directory -Force -Path $fx | Out-Null

# The Kamino main-market USDC reserve and everything KLend touches to price and move it. Verified on
# mainnet: reserve.liquidity.mint_pubkey == USDC, reserve.lending_market == the main market.
$accounts = [ordered]@{
  "reserve"          = "D6q6wuQSrifJKZYpR1M8R4YawnLDtDsMmWM1NbBmgJ59" # KLend USDC reserve (main market)
  "lending_market"   = "7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF" # Kamino main lending market
  "liq_supply_vault" = "Bgq7trRgVMeq33yt235zM2onQ4bRDBsY5EWiTetF4qw6" # reserve's USDC supply vault
  "coll_mint"        = "B8V6WVjPxW1UGwVDfxH2d2r8SyT4cqn7dQRK6XneVa7D" # cUSDC collateral mint
  "usdc_mint"        = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" # USDC
  "scope_prices"     = "3t4JZcueEzTbVP6kLxXrL3VpWx45jDer4eqysweBchNH" # Scope oracle prices (chain index 13)
}
foreach ($k in $accounts.Keys) {
  Write-Host "dumping $k ($($accounts[$k]))"
  solana account $accounts[$k] --url mainnet-beta --output json --output-file (Join-Path $fx "$k.json")
}
Write-Host "dumping KLend program bytecode"
solana program dump KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD (Join-Path $fx "klend.so") --url mainnet-beta
Write-Host "done -> $fx"
