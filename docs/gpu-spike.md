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
| one bright pixel (every point on one pixel), before the hit cache | 245.5 M/s | 193.2 M/s | 0.8x |
| one bright pixel, with the hit cache | 243 M/s | 7,850 M/s | 32x |

The CPU figure here is higher than the "about 36M samples/s" recorded in the plan. Both are
measurements on this machine, but the plan's figure came from an earlier run and its conditions
were not recorded. Step 3.3 will set a proper benchmark.

Re-run after the clamp, with other processes using most of the CPU (`dotnet`, about 780%):
CPU 24.0 / 27.2 / 97.2 M/s and GPU 2119 / 2259 / 194 M/s for the three flames in the same order.
The GPU figures agree with the table above. The CPU figures do not, because of that load, so they
should not be compared with the table. Re-measure the CPU on a quiet machine in step 3.3.

### Precision (WGSL f32 against Rust f64)

4000 random points per variation, with random affine coefficients. Relative error is the largest
component difference divided by max(1, |value|).

- 23 of 26 variations: max relative error below 3e-6.
- Popcorn: max 1.3e-5, p99 1.8e-6.
- Cross: max 1.3e-5, p99 1.0e-6 (after the clamp; was max 1.4e-4).
- Waves: max 0.019, p99 1.1e-4 (after the clamp; was max 0.54).

The full table is in the raw output. The clamp was measured on 2026-10-22 after the decision below.

## Findings

1. **Speed.** On normal flames the GPU runs 32 to 40 times faster than the CPU.
2. **Contention was the problem, and a per-thread hit cache fixes the worst case.** Before the
   cache, when every hit landed on one pixel, the GPU dropped to 193 M/s, below the CPU's
   246 M/s. Each thread now keeps its last pixel and its pending count and colour in registers,
   and flushes to the shared atomics only when the walk moves to another pixel. With the cache
   the same case runs at about 7,850 M/s, roughly 32 times the CPU. Normal flames are unchanged
   (about 2,300 to 2,500 M/s). The histograms are identical to the uncached run: the hit counts
   and mean colours match exactly, since integer sums commute. Measured on the quiet machine,
   three runs each, with the cache and without. The best case for the cache is one pixel;
   flames where hits spread over a few pixels fall between the two and are not measured yet.
3. **Precision is fine for most variations, with two exceptions.** Waves divides by
   `c*c + 1e-10`, so a transform with `c` near zero sends `sin` a large argument, which f32
   cannot reduce accurately. Cross has a similar near-zero division. Both denominators are now
   floored at 1e-6 in the shared definition. That cut the worst error by about 30x for Waves
   and 10x for Cross. Waves still has a p99 of 1e-4, because `sin` of an argument near 1e6 is
   inaccurate in f32 too. Popcorn's error is small.
4. **Accuracy holds in aggregate.** At image level the GPU matches the CPU to within run-to-run
   noise, despite the per-point errors above.

## Decision

**Go for the GPU renderer** (steps 3.4 to 3.6). Both conditions below are now met
for the cases measured:

- **Contention.** Met (2026-10-22). The per-thread hit cache makes the one-bright-pixel case
  about 32 times faster than the CPU, against the criterion of not slower. The integer atomics
  stay: they are exact and do not depend on order. Still open for step 3.4: measure flames
  whose hits spread over a few pixels, where the cache is least effective.
- **Singular variations.** Decided 2026-10-22: Waves and Cross denominators are clamped at 1e-6
  in the shared definition, so both renderers agree. This changes output for flames with a
  near-zero `c` or `x*x - y*y`. Waves' p99 error is still 1e-4; accepted for now.

## Not covered

- Windows and DX12, Intel and AMD GPUs, and the Metal behaviour of other Apple chips.
- Sustained load and heat, and sizes beyond 800 x 600 (step 3.6).
- Full images for each variation. The precision table checks each variation's maths point by
  point, which is what the image depends on.
