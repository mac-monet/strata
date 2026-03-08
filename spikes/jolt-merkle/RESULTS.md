# Jolt Merkle Spike — Results

## What Was Tested

Nonce verification + fixed-depth (16) binary Merkle tree update, comparing
SHA-256 (`jolt-inlines-sha2`) vs Blake3 (`jolt-inlines-blake3`).

Per entry: 2 root computations x 16 levels x 1 hash = 32 hashes.

## Benchmarks (Apple Silicon, 16 GB RAM)

### SHA-256

| Batch | Cycles | Prove  | Verify | Trace Length | RAM    |
|-------|--------|--------|--------|--------------|--------|
| 1     | 244K   | 4.5s   | 0.09s  | 2^22 (4.2M)  | < 10 GB |
| 10    | 2.3M   | 22.6s  | 0.10s  | 2^22 (4.2M)  | < 10 GB |
| 100   | 23.2M  | —      | —      | 2^25 (33.5M) | ~32 GB  |

### Blake3

| Batch | Cycles | Prove  | Verify | Trace Length | RAM    |
|-------|--------|--------|--------|--------------|--------|
| 1     | 65K    | 2.9s   | 0.08s  | 2^22 (4.2M)  | < 10 GB |
| 10    | 538K   | 8.6s   | 0.10s  | 2^22 (4.2M)  | < 10 GB |
| 100   | 5.4M   | 48.4s  | 0.11s  | 2^23 (8.4M)  | < 10 GB |

### Head-to-Head

| Metric              | SHA-256  | Blake3   | Blake3 advantage |
|---------------------|----------|----------|------------------|
| Cycles per entry    | ~230K    | ~54K     | **4.3x fewer**   |
| 10-entry prove time | 22.6s    | 8.6s     | **2.6x faster**  |
| 100-entry feasible  | No (32GB)| Yes (10GB)| **Blake3 wins**  |

## Key Findings

1. **Blake3 is dramatically cheaper to prove.** 4.3x fewer cycles per entry
   because the Blake3 inline instruction is simpler than SHA-256's compression
   function.

2. **Blake3 enables 100-entry batches on 16 GB RAM.** SHA-256 cannot — it needs
   2^25 trace length (~32 GB). This is the single biggest practical difference.

3. **Verification is constant ~0.1s** regardless of hash function or batch size.

4. **No showstopper issues** with either hash function. Both inlines work
   correctly in the Jolt RISC-V guest.

5. **Guest std mode works.** Used `guest-std` for Vec params. Production would
   use `no_std` + `UntrustedAdvice`.

## Implications for Strata

- **Strong recommendation: use Blake3** for the merkle tree hash function.
  4.3x cycle reduction directly translates to cheaper proving and lower RAM.
- **Batch size sweet spot with Blake3:** 50-100 entries on 16 GB, 200+ on 32 GB.
- **Commonware supports both** SHA-256 and Blake3, so this is a config choice.
- **QMDB MMR proofs** will have different structure but same core pattern.

## How to Run

```bash
cd spikes/jolt-merkle

# SHA-256
RUST_LOG=info cargo run --release -- 1 10

# Blake3
RUST_LOG=info cargo run --release -- --blake3 1 10 100

# Both (side-by-side comparison)
RUST_LOG=info cargo run --release -- --both 10
```
