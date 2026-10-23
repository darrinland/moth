# GPU feasibility spike (plan step 0.6)

Run on 2026-10-22 on an Apple M4 (Metal). Windows (DX12) was not measured: it is shelved until a
Windows machine is available. Code: [spikes/gpu-chaos](../spikes/gpu-chaos). Raw output:
[gpu-spike-results.txt](gpu-spike-results.txt). Reproduce with `cargo run --release -p gpu-chaos`.

## What was built

- The chaos game as a wgpu compute shader ([chaos.wgsl](../spikes/gpu-chaos/src/chaos.wgsl)).
  Each thread walks one point with its own PCG random stream, burns in, then plots through the
  camera into an atomic `u32` histogram holding `[count, r, g, b]` per pixel, as the CPU does.
- Every variation comes from the same generated WGSL that the CPU code was generated alongside
  (`variations.def` through `build.rs`, see [PLAN.md](PLAN.md) step 0.7).
- The CPU reference is `moth_core::flame::render::raw_histogram`, a new public function that
  returns the renderer's raw accumulation before filtering and tone mapping.

## Results

### Accuracy

Total variation distance (TV) between normalized histograms, at 200 x 150 with 60 million samples.
The noise floor is the TV between two CPU runs with different seeds: the best a GPU run can be
expected to match.

| flame | noise floor (CPU vs CPU) | GPU vs CPU | mean red, CPU / GPU |
|-------|--------------------------|------------|---------------------|
| three variations | 0.0062 | 0.0062 | 146.4 / 146.4 |
| random (seed 3) | 0.0042 | 0.0042 | 61.8 / 61.8 |

The GPU histogram is as close to the CPU histogram as the CPU is to itself.

### Throughput

Chaos iterations per second, 800 x 600 at quality 200 (96 million samples). CPU is the existing
rayon renderer on all cores, in a release build.

| flame | CPU | GPU | speed-up |
|-------|-----|-----|----------|
| three variations | 58.5 M/s | 2332 M/s | 39.8x |
| random (seed 3) | 71.4 M/s | 2295 M/s | 32.1x |
| one bright pixel (every point on one pixel) | 245.5 M/s | 193.2 M/s | 0.8x |

The CPU figure here is higher than the "about 36M samples/s" recorded in the plan. Both are
measurements on this machine, but the plan's figure came from an earlier run and its conditions
were not recorded. Step 3.3 will set a proper benchmark.

### Precision (WGSL f32 against Rust f64)

4000 random points per variation, with random affine coefficients. Relative error is the largest
component difference divided by max(1, |value|).

- 23 of 26 variations: max relative error below 3e-6.
- Popcorn: max 1.3e-5, p99 1.8e-6.
- Cross: max 1.4e-4, p99 1.1e-6.
- Waves: max 0.54, p99 1.2e-4.

The full table is in the raw output.

## Findings

1. **Speed.** On normal flames the GPU runs 32 to 40 times faster than the CPU.
2. **Contention is the problem.** When every hit lands on one pixel, the GPU drops to 193 M/s,
   below the CPU's 246 M/s. The CPU copes through its per-thread hit cache, which merges
   repeated hits to a pixel before touching shared memory. The GPU has no such cache, so every
   hit is an atomic on one address. This is the worst case. Dense areas of real flames are much
   less concentrated, so the real penalty is smaller, but it has not been measured yet.
3. **Precision is fine for most variations, with two exceptions.** Waves divides by
   `c*c + 1e-10`, so a transform with `c` near zero sends `sin` an argument around 1e10, which
   f32 cannot reduce accurately. Cross has a similar near-zero division. Popcorn's error is
   small. These are rare, but they are real, and they need a rule that both renderers share.
4. **Accuracy holds in aggregate.** At image level the GPU matches the CPU to within run-to-run
   noise, despite the per-point errors above.

## Decision

**Go for the GPU renderer** (steps 3.4 to 3.6), with two conditions:

- **Contention.** Before 3.4 is accepted, the hot-pixel case needs a fix. Options to test next:
  a per-thread register cache like the CPU's `HitCache`, or merging hits within a workgroup
  before the global atomics. The integer atomics stay: they are exact and do not depend on
  order. Acceptance: the one-bright-pixel case must not be slower than the CPU.
- **Singular variations.** Waves and Cross need a decision about how the near-zero division is
  handled in both renderers. This is an owner decision because it changes output for those
  rare cases: clamp the denominator (for example at 1e-6) in the shared definition, or accept
  the error.

## Not covered

- Windows and DX12, Intel and AMD GPUs, and the Metal behaviour of other Apple chips.
- Sustained load and heat, and sizes beyond 800 x 600 (step 3.6).
- Full images for each variation. The precision table checks each variation's maths point by
  point, which is what the image depends on.
