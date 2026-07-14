# Devnet transaction log

Every link below is a real, confirmed transaction on Solana devnet, produced by running the two deployed
Obligo programs end-to-end. Nothing here is fabricated; each signature resolves on the explorer.

| Program | Address |
|---|---|
| core `obligo` | `3YV8p2MhPQpSZS8fp3D7VoQYPy8GwxV221koWpsptRpN` |
| hook `obligo_hook` | `AtDpNdzKVRxMwK5bTotfmjxQdVU854RopJccgYRP8wQ7` |

The protocol takes its settlement asset as a parameter at genesis. This run uses a self-controlled
6-decimal SPL mint so the demo is reproducible without a faucet; on mainnet this would be Circle USDC.
The protocol assumes nothing about the mint beyond its being an SPL/Token-2022 mint.

## Deployment

- core deploy — [`3xsVM97M…`](https://explorer.solana.com/tx/3xsVM97M9hcj8F1BugWmm8jJsz9KSPCjkKY1CGgkzKQo8NEURwv5f52KZB4Vjwe4vgzbVCv5HEtN2m4pxXfhk8AG?cluster=devnet)
- hook deploy — [`26fp4nic…`](https://explorer.solana.com/tx/26fp4nicqQK8x9zSZ1cAVLZ53DRPVu8eKhYfrR1zgj7nbUodcRQkXVp9Z1B4QxmXPcrSfWjLCxNtmG8Hrb7NprD6?cluster=devnet)

## Lifecycle

**Genesis.** Fixes the settlement mint and the hook program for all time —
[`tHePr4L2…`](https://explorer.solana.com/tx/tHePr4L2gqTScKiKG9RukXY9b9rdoMSG1nV16Rojg7CZ8ia47vD4VMQwjovUHqGP9tJuE8nx4cEvxWq11cVUVrY?cluster=devnet)

**Merchants register** (permissionless), each with its own terms —
[CAFE](https://explorer.solana.com/tx/2p6tDkTsnL3Q3LLdDfPhphdR6w55E4i3a52W7Swd5ctgHQLfXvyfXyH6Graqj2ACYXd7DStFsYqM7jGc1T9A8pUk?cluster=devnet) ·
[SHOP](https://explorer.solana.com/tx/53Ecq4r9cjYUBBW5PwmapQ3F9G75nUrPPw4FAbJNgdKQuwDkJ8v48M68MJRXJzLkv4EGx2Ma7jo8RUfEw6TEoAwB?cluster=devnet) ·
[KIOSK](https://explorer.solana.com/tx/5uiu8XsHedwkPMS29tKDh3hTTjSkRney2MKMxuER5rbYqXSJhwA3UwwuAY4kkgFxr2kANX7gm4Vu8DZmCsCdukWn?cluster=devnet)

**Points mints created** — Token-2022, hooked, hook authority burned —
[CAFE](https://explorer.solana.com/tx/38unZECAx4wW2opUYrihqQNfpbwtFcq7TXphNNeDM9dAvZWdzQrn7dgkEo9ukQxQZ5dmsjShX8uiTjwK3sRjVdii?cluster=devnet) ·
[SHOP](https://explorer.solana.com/tx/JKdVtxEoUwNJzHjXJk3ubpuNbYGQbvKsBmGKk8aQmKFXPCXuJsMBwAqqgJR92dwbFd7ZMDCMrDUi3zTrGyXHLCx?cluster=devnet) ·
[KIOSK](https://explorer.solana.com/tx/5rDndhF1JdvW3cxKEMWVzn1A8DFckUDR2YDY9uf1EbUsoEd82Ds9kd572dT5ifB85beC7kVevW8P72NyTHe65irY?cluster=devnet)

**Collateral deposited** —
[CAFE $50](https://explorer.solana.com/tx/66RAz8cLT6kzEGD3XpoiFraWB6yLkEERkAv5ke8EMzxficqcvVtg6aBaALT1R8yYKUe69BEQ2M5PJP4YYgxJQuR1?cluster=devnet) ·
[SHOP $50](https://explorer.solana.com/tx/46kQqAfty7qCMdDaRPX4RUNH5yK44bwoLhvqW6FBmTkhd3SNtr2x1gbmH4VEy1Z5L3c5meCDr28GEtHHgzHvP6kP?cluster=devnet) ·
[KIOSK $50](https://explorer.solana.com/tx/WtUncL6NsQYASpasiiGUMikCWWsA5AtGHCcmoQd3qmCe6vDuGuCrobdPdkY1GQ8HZEV3Y5KsASZdiNMtmBeMPfi?cluster=devnet)

**Acceptance offers.** Shop honours Café at **110%** — bidding above face to win footfall —
[`3A2ndCp5…`](https://explorer.solana.com/tx/3A2ndCp5pF6FVwkcGJpmEknbUTovgbqqt566TSnQ8UAoXLW8T6iLuwPoWYfqHnahi41WYvtcMYK1HKBBASK8JFXn?cluster=devnet).
Kiosk honours Café at **95%** — discounting the issuer's credit —
[`5HzG5doq…`](https://explorer.solana.com/tx/5HzG5doqMxDTjMhTXF5vhajid28Ua7SMMRmxPvFN6pUtLLg2uLKj2Dq5yYAQJJjhH62hRzSi4zb4KFyuykSXHg4a?cluster=devnet)

**Issuance.** Café mints 500 points ($5.00 face) to a customer; 30% reserve ($1.50) locked —
[`2ZaiTSDB…`](https://explorer.solana.com/tx/2ZaiTSDB2EL2nECWFM8gpL8RZ36Z1CDt4TMPoTZjaijEhkRWAjvakJkTw2NYAHAtwQ7d4gEEdLyQFXAZdQRvUdMZ?cluster=devnet)

### Redemption — a debt is created, no cash moves
[`4vuxzkqq…`](https://explorer.solana.com/tx/4vuxzkqqtD4Ft2zNGN4mAuZUgfyGc1yewSU95E8LkZUQUp5LFHsUGrbWKZ3xuBsT7orGLaemreqpDn1Bav6nwwrv?cluster=devnet)
- 500 Café points redeemed at Shop; goods worth **$5.50** handed over (Shop's 110% bid, eating $0.50 of acquisition cost)
- obligation **Café→Shop $5.00** created
- **USDC moved: $0.00** — both vaults byte-identical
- Café health **3333% → 1000%** — a 30% reserve becomes a 100% debt

### Cycle clearing — a ring of debt extinguished with zero cash
[`UagyNDAt…`](https://explorer.solana.com/tx/UagyNDAtz4FHEStnLfah8GkQJjCWiCmRWp7ju5ZBHNMxAHQXo81qbtBwYDhvCvNsyFnyNTszk9zwqLF46m23mKg?cluster=devnet)
- ring Café→Shop→Kiosk→Café, edges $5 / $7 / $6
- **obligations extinguished: $15.00** ($5 smallest edge × 3)
- **USDC moved: $0.00** — all three vaults byte-identical
- edges after: $0 / $2 / $1

### Bilateral settlement — only the difference moves
[`2WhtBnNk…`](https://explorer.solana.com/tx/2WhtBnNkG6j7sw4JmsGhYB7nuzPLu9mVTt1Qo76Jv4qyk3rFBz6jgtxFWU3tJt4G7ckuS736gNeycG6F3ZGq2w4t?cluster=devnet)
- Café→Shop $8, Shop→Café $3 → $3 cancels, **net $5.00 moved**

### Liquidation — pro-rata, permissionless
An issuer with $3 collateral against $12 of debt is liquidated; both creditors recover exactly 25%.
[creditor Shop → $2.00](https://explorer.solana.com/tx/4G6u4dGJj49ueN12DRDJmnwSQ9uZyZCmeAu5mm45VCognTPpFFcwpzi2812VRNWDRDF7ZJ8RqwUMWkAwFbNpFH5c?cluster=devnet) ·
[creditor Kiosk → $1.00](https://explorer.solana.com/tx/3ZYNWiL2vBhp8qeU6GQRCXHL23UT4ktTc5Zc7Q9GNhjN2A5qNWMKi3agiWQmha8EfVVyrkByAz2F8X7S3pjJmVcz?cluster=devnet)

**Reinstatement.** After topping up, the defaulted merchant returns to Active — the default stays on
record —
[`2NHMgjSC…`](https://explorer.solana.com/tx/2NHMgjSCy2UTYS5rvjp5zQRpEiiJFCCyBUqsufGm8J1P516aZiJsKHyVgEWL3SrKVf87Hef9NPjQqyDpQmamHmVc?cluster=devnet)

### Breakage — lapsed points burned, recognised on-chain
[`4bQBCpbR…`](https://explorer.solana.com/tx/4bQBCpbRWebPMdmcVd3aoSjKnPEbv9FNJpuNxri4D9VVxx1VqZ3q483phJH1EPjQmjSLZd7JKJzPfmaCnFan2wXJ?cluster=devnet)
- 100 lapsed points burned; **breakage face value $1.00** recognised; `total_expired` 0 → 100
