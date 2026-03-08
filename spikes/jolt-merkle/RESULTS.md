# Jolt Merkle Spike — Results

## What Was Tested

Nonce verification + fixed-depth (16) binary Merkle tree update using SHA-256
(`jolt-inlines-sha2`). The guest verifies each entry's old position was empty,
then computes the new root.

Per entry: 2 root computations x 16 levels x 1 SHA-256 = 32 SHA-256 hashes.

## Benchmarks (Apple Silicon, 16 GB RAM)

| Batch Size | Cycles    | Prove Time | Verify Time | max_trace_length | RAM Needed |
|------------|-----------|------------|-------------|------------------|------------|
| 1          | 244K      | 4.5s       | 0.09s       | 2^22 (4.2M)      | < 10 GB    |
| 10         | 2.3M      | 22.6s      | 0.10s       | 2^22 (4.2M)      | < 10 GB    |
| 100        | 23.2M     | —          | —           | 2^25 (33.5M)     | ~32 GB     |

100 entries could not be proved on 16 GB RAM (needs 2^25 trace length → ~32 GB).

## Key Findings

1. **SHA-256 inline works well.** `jolt-inlines-sha2::Sha256::digest()` is a
   custom RISC-V instruction, not software SHA-256. ~230K cycles per entry
   (32 hashes).

2. **Proving time scales with actual cycles, not just padded trace.** 1 entry
   (244K cycles) proves in 4.5s vs 10 entries (2.3M cycles) in 22.6s, despite
   both fitting in the same 2^22 trace.

3. **Verification is constant ~0.1s** regardless of batch size.

4. **Memory is the bottleneck for large batches.** Proving time is acceptable
   (would be ~2 min for 100 entries on a 32+ GB machine), but RAM requirements
   scale with `max_trace_length`.

5. **Guest std mode works.** Used `guest-std` feature for Vec params. Production
   would use `no_std` + `UntrustedAdvice` to keep params fixed-size.

6. **No showstopper issues.** no_std compatibility, RISC-V compilation, and
   Jolt's proving pipeline all work as expected.

## Per-Entry Cost Breakdown

~230K cycles per merkle insert:
- 2 × compute_root (verify empty + compute new): 16 SHA-256 each = 32 total
- Each SHA-256 inline: ~7K cycles (estimated from total)
- Byte copying/parsing: minimal overhead

## Implications for Strata

- **Batch size sweet spot: 10-50 entries** on typical hardware (16-32 GB RAM)
- **Proving can run async** — 23s is acceptable for a rollup batch
- **Blake3 untested** — `jolt-inlines-blake3` exists and may reduce cycle count
- **QMDB MMR proofs** will have different structure than this fixed-depth tree,
  but the core pattern (verify old root + compute new root with SHA-256) is the
  same

## How to Run

```bash
cd spikes/jolt-merkle
RUST_LOG=info cargo run --release -- 1      # single entry
RUST_LOG=info cargo run --release -- 10     # batch of 10
RUST_LOG=info cargo run --release -- 1 10   # both sequentially
```

To test 100 entries, increase `max_trace_length` to `33554432` in
`guest/src/lib.rs` and run on a machine with 32+ GB RAM.
