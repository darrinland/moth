//! Throwaway GPU feasibility spike for plan step 0.6. Not part of the app.
//!
//! The chaos game runs as a wgpu compute shader and accumulates into an atomic u32
//! histogram. It is checked against moth-core's CPU renderer, on this machine, in three ways:
//!
//! 1. Accuracy: the GPU histogram against the CPU one. The yardstick is the difference
//!    between two CPU runs with different seeds (the noise floor).
//! 2. Throughput: chaos iterations per second on each renderer, for three flames.
//! 3. Precision: every variation's WGSL against its Rust code, point by point, f32 against f64.

use std::borrow::Cow;
use std::sync::mpsc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use moth_core::RenderSpec;
use moth_core::flame::random::{self, RandomSettings};
use moth_core::flame::render::raw_histogram;
use moth_core::flame::variations::{Variation, WGSL as VARIATIONS_WGSL};
use moth_core::flame::{Affine, Camera, Genome, Palette, Xform};
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use wgpu::util::DeviceExt;

const KERNEL: &str = include_str!("chaos.wgsl");
/// Matches `BURN_IN` in moth-core's `flame/render.rs`.
const BURN_IN: u32 = 20;
/// Concurrent walkers. Each one runs `iters` iterations.
const THREADS: u32 = 1 << 16;
const NO_FINAL: u32 = u32::MAX;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct XfGpu {
    cum: f32,
    color: f32,
    speed: f32,
    var_off: u32,
    aff0: [f32; 4],
    aff1: [f32; 4],
    post0: [f32; 4],
    post1: [f32; 4],
    var_n: u32,
    has_post: u32,
    pad: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParamsGpu {
    n_xf: u32,
    final_idx: u32,
    iters: u32,
    burn_in: u32,
    seed: u32,
    w: u32,
    h: u32,
    pad: u32,
    total_weight: f32,
    cx: f32,
    cy: f32,
    ppu: f32,
    cos_r: f32,
    sin_r: f32,
    half_w: f32,
    half_h: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ProbeGpu {
    xy: [f32; 2],
    id: u32,
    coin: f32,
    aff0: [f32; 4],
    aff1: [f32; 4],
}

/// A flame flattened into the buffers the kernel reads.
struct Flat {
    xfs: Vec<XfGpu>,
    vars: Vec<[f32; 4]>,
    palette: Vec<[u32; 4]>,
    params: ParamsGpu,
}

fn affine_rows(a: &Affine) -> ([f32; 4], [f32; 4]) {
    (
        [a.a as f32, a.b as f32, a.c as f32, a.d as f32],
        [a.e as f32, a.f as f32, 0.0, 0.0],
    )
}

fn xf_gpu(x: &Xform, cum: f64, vars: &mut Vec<[f32; 4]>) -> XfGpu {
    let var_off = vars.len() as u32;
    for &(v, w) in &x.variations {
        vars.push([v as u32 as f32, w as f32, 0.0, 0.0]);
    }
    let (aff0, aff1) = affine_rows(&x.affine);
    let (post0, post1) = affine_rows(&x.post);
    XfGpu {
        cum: cum as f32,
        color: x.color as f32,
        speed: x.color_speed as f32,
        var_off,
        aff0,
        aff1,
        post0,
        post1,
        var_n: x.variations.len() as u32,
        has_post: u32::from(!x.post.is_identity()),
        pad: [0; 2],
    }
}

fn flatten(g: &Genome, spec: &RenderSpec) -> Flat {
    let mut vars = Vec::new();
    let mut xfs = Vec::new();
    let mut total = 0.0;
    for x in g.xforms.iter().filter(|x| x.weight > 0.0) {
        total += x.weight;
        xfs.push(xf_gpu(x, total, &mut vars));
    }
    let n_xf = xfs.len() as u32;
    let mut final_idx = NO_FINAL;
    if let Some(f) = &g.final_xform {
        final_idx = xfs.len() as u32;
        xfs.push(xf_gpu(f, 0.0, &mut vars));
    }
    let (sin, cos) = g.camera.rotate.to_radians().sin_cos();
    let ppu = g.camera.scale * spec.width as f64 / g.camera.size.0.max(1) as f64;
    let samples = spec.quality as f64 * (spec.width as f64 * spec.height as f64);
    let iters = (samples / THREADS as f64).ceil() as u32;
    let params = ParamsGpu {
        n_xf,
        final_idx,
        iters,
        burn_in: BURN_IN,
        seed: spec.seed as u32,
        w: spec.width,
        h: spec.height,
        pad: 0,
        total_weight: total as f32,
        cx: g.camera.center.0 as f32,
        cy: g.camera.center.1 as f32,
        ppu: ppu as f32,
        cos_r: cos as f32,
        sin_r: sin as f32,
        half_w: spec.width as f32 / 2.0,
        half_h: spec.height as f32 / 2.0,
    };
    let palette = (0..256)
        .map(|i| {
            let c = g.palette.sample(i as f64 / 255.0);
            [
                (c[0] * 255.0) as u32,
                (c[1] * 255.0) as u32,
                (c[2] * 255.0) as u32,
                0,
            ]
        })
        .collect();
    Flat {
        xfs,
        vars,
        palette,
        params,
    }
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    chaos: wgpu::ComputePipeline,
    probe: wgpu::ComputePipeline,
    name: String,
    backend: String,
}

fn gpu_init() -> Gpu {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .expect("no GPU adapter");
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("no GPU device");
    let source = format!("{VARIATIONS_WGSL}\n{KERNEL}");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("chaos"),
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(source)),
    });
    let pipeline = |entry: &str| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: None,
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    let chaos = pipeline("chaos");
    let probe = pipeline("probe");
    Gpu {
        device,
        queue,
        chaos,
        probe,
        name: info.name,
        backend: format!("{:?}", info.backend),
    }
}

fn storage<T: Pod>(gpu: &Gpu, label: &str, data: &[T]) -> wgpu::Buffer {
    gpu.device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(data),
            usage: wgpu::BufferUsages::STORAGE,
        })
}

fn read_back(gpu: &Gpu, src: &wgpu::Buffer, bytes: u64) -> Vec<u8> {
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("staging"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_buffer_to_buffer(src, 0, &staging, 0, bytes);
    gpu.queue.submit([enc.finish()]);
    let slice = staging.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");
    rx.recv().unwrap().expect("map");
    let data = slice.get_mapped_range().expect("mapped range").to_vec();
    staging.unmap();
    data
}

/// Runs the chaos kernel once. Returns the raw histogram (`[count, r, g, b]` per pixel)
/// and the time the GPU took to run the kernel.
fn run_chaos(gpu: &Gpu, g: &Genome, spec: &RenderSpec) -> (Vec<u32>, f64) {
    let flat = flatten(g, spec);
    let xfs = storage(gpu, "xfs", &flat.xfs);
    let vars = storage(gpu, "vars", &flat.vars);
    let palette = storage(gpu, "palette", &flat.palette);
    let params = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("params"),
            contents: bytemuck::bytes_of(&flat.params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let pixels = spec.width as u64 * spec.height as u64;
    let bytes = pixels * 16;
    let hist = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hist"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let layout = gpu.chaos.get_bind_group_layout(0);
    let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("chaos"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: xfs.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: vars.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: palette.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: hist.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: params.as_entire_binding(),
            },
        ],
    });

    let started = Instant::now();
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&gpu.chaos);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(THREADS / 256, 1, 1);
    }
    gpu.queue.submit([enc.finish()]);
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");
    let secs = started.elapsed().as_secs_f64();

    let raw = read_back(gpu, &hist, bytes);
    (bytemuck::cast_slice(&raw).to_vec(), secs)
}

/// Evaluates the WGSL `variation_apply` at each probe.
fn run_probes(gpu: &Gpu, probes: &[ProbeGpu]) -> Vec<[f32; 2]> {
    let input = storage(gpu, "probes", probes);
    let bytes = (probes.len() * 8) as u64;
    let out = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("probe_out"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    // Group 0 is unused by the probe entry point but must still be bound.
    let empty = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("empty"),
        layout: &gpu.probe.get_bind_group_layout(0),
        entries: &[],
    });
    let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("probe"),
        layout: &gpu.probe.get_bind_group_layout(1),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: out.as_entire_binding(),
            },
        ],
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&gpu.probe);
        pass.set_bind_group(0, &empty, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.dispatch_workgroups(probes.len().div_ceil(64) as u32, 1, 1);
    }
    gpu.queue.submit([enc.finish()]);
    let raw = read_back(gpu, &out, bytes);
    bytemuck::cast_slice::<u8, [f32; 2]>(&raw).to_vec()
}

/// Total variation distance between two count histograms, each normalized to sum to 1.
fn total_variation(a: impl Iterator<Item = f64>, b: impl Iterator<Item = f64>) -> f64 {
    let pairs: Vec<(f64, f64)> = a.zip(b).collect();
    let (sa, sb): (f64, f64) = pairs
        .iter()
        .fold((0.0, 0.0), |(x, y), (p, q)| (x + p, y + q));
    0.5 * pairs
        .iter()
        .map(|(p, q)| (p / sa - q / sb).abs())
        .sum::<f64>()
}

fn counts_cpu(raw: &[f32]) -> impl Iterator<Item = f64> + '_ {
    raw.as_chunks::<4>().0.iter().map(|px| px[0] as f64)
}

fn counts_gpu(raw: &[u32]) -> impl Iterator<Item = f64> + '_ {
    raw.as_chunks::<4>().0.iter().map(|px| px[0] as f64)
}

/// Mean palette red per hit, a cheap check that colour arrives in the same place.
fn mean_red(raw: impl Iterator<Item = (f64, f64)>) -> f64 {
    let (r, n) = raw.fold((0.0, 0.0), |(r, n), (red, count)| (r + red, n + count));
    r / n.max(1.0)
}

fn simple_flame() -> Genome {
    let mut g = Genome::blank();
    g.name = "three variations".into();
    let xform = |weight, color, a, b, c, d, e, f, variations: Vec<(Variation, f64)>| Xform {
        weight,
        color,
        color_speed: 0.5,
        affine: Affine { a, b, c, d, e, f },
        post: Affine::IDENTITY,
        variations,
    };
    g.xforms = vec![
        xform(
            1.0,
            0.0,
            0.7,
            0.3,
            -0.4,
            -0.3,
            0.7,
            0.2,
            vec![(Variation::Swirl, 1.0)],
        ),
        xform(
            1.0,
            0.5,
            0.5,
            -0.4,
            0.3,
            0.4,
            0.5,
            -0.2,
            vec![(Variation::Julia, 1.0)],
        ),
        xform(
            0.6,
            1.0,
            0.6,
            0.0,
            0.0,
            0.0,
            0.6,
            0.4,
            vec![(Variation::Spherical, 0.5), (Variation::Heart, 0.5)],
        ),
    ];
    g.palette = Palette::random_seeded(7);
    g.camera = Camera {
        center: (0.0, 0.0),
        scale: 90.0,
        rotate: 0.0,
        size: (800, 600),
    };
    g
}

/// Every point lands on one pixel: the worst case for atomic contention.
fn point_flame() -> Genome {
    let mut g = simple_flame();
    g.name = "one bright pixel".into();
    g.xforms = vec![Xform {
        weight: 1.0,
        color: 0.5,
        color_speed: 0.5,
        affine: Affine {
            a: 0.0,
            b: 0.0,
            c: 0.0,
            d: 0.0,
            e: 0.0,
            f: 0.0,
        },
        post: Affine::IDENTITY,
        variations: vec![(Variation::Linear, 1.0)],
    }];
    g
}

fn random_flame() -> Genome {
    let mut g = random::randomize(&RandomSettings::default(), 3);
    random::auto_frame(&mut g, 3);
    g.name = "random (seed 3)".into();
    g
}

/// The seed whose first `random_bool(0.5)` is `want`, so Julia's coin can be pinned on the CPU.
fn seed_for_coin(want: bool) -> u64 {
    (0..10_000u64)
        .find(|&s| ChaCha8Rng::seed_from_u64(s).random_bool(0.5) == want)
        .expect("a seed for each coin side")
}

/// Compares every variation's WGSL with its Rust `apply` at random points.
fn precision(gpu: &Gpu) {
    const PER_VARIATION: usize = 4000;
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let mut probes = Vec::new();
    let mut expect = Vec::new();
    for (vi, &v) in Variation::ALL.iter().enumerate() {
        for _ in 0..PER_VARIATION {
            let xy = [
                rng.random_range(-1.5f32..1.5),
                rng.random_range(-1.5f32..1.5),
            ];
            let mut k = || rng.random_range(-1.0f32..1.0);
            let aff0 = [k(), k(), k(), k()];
            let aff1 = [k(), k(), 0.0, 0.0];
            let coin = rng.random_bool(0.5);
            probes.push(ProbeGpu {
                xy,
                id: vi as u32,
                coin: f32::from(u8::from(coin)),
                aff0,
                aff1,
            });
            let aff = Affine {
                a: aff0[0] as f64,
                b: aff0[1] as f64,
                c: aff0[2] as f64,
                d: aff0[3] as f64,
                e: aff1[0] as f64,
                f: aff1[1] as f64,
            };
            // Julia's coin is the first draw from the rng, so pick a seed that gives it.
            let mut cpu_rng = ChaCha8Rng::seed_from_u64(seed_for_coin(coin));
            let cpu = v.apply(xy[0] as f64, xy[1] as f64, &aff, &mut cpu_rng);
            expect.push(cpu);
        }
    }
    let got = run_probes(gpu, &probes);

    println!("\nPrecision: WGSL (f32) against Rust (f64), {PER_VARIATION} random points each");
    println!(
        "{:<14} {:>12} {:>12} {:>9}",
        "variation", "max rel err", "p99 rel err", "nonfinite"
    );
    for (vi, &v) in Variation::ALL.iter().enumerate() {
        let mut errs = Vec::new();
        let mut nonfinite = 0;
        for i in vi * PER_VARIATION..(vi + 1) * PER_VARIATION {
            let (cx, cy) = expect[i];
            if !(cx.is_finite() && cy.is_finite()) {
                nonfinite += 1;
                continue;
            }
            let (gx, gy) = (got[i][0] as f64, got[i][1] as f64);
            let e = ((gx - cx).abs().max((gy - cy).abs())) / cx.abs().max(cy.abs()).max(1.0);
            errs.push(e);
        }
        errs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let max = errs.last().copied().unwrap_or(f64::NAN);
        let p99 = errs
            .get(((errs.len() as f64) * 0.99) as usize)
            .copied()
            .unwrap_or(f64::NAN);
        println!(
            "{:<14} {:>12.3e} {:>12.3e} {:>9}",
            v.name(),
            max,
            p99,
            nonfinite
        );
    }
}

fn accuracy_and_speed(gpu: &Gpu) {
    println!("\nAccuracy: GPU histogram against CPU (total variation distance; lower is closer)");
    println!("noise floor = CPU against a second CPU run with another seed");
    let acc = RenderSpec {
        width: 200,
        height: 150,
        quality: 2000.0,
        transparent: false,
        seed: 1,
        oversample: 1,
        denoise: 0.0,
    };
    let mut acc_flames = vec![
        ("three variations", simple_flame()),
        ("random", random_flame()),
    ];
    for (name, g) in &mut acc_flames {
        let cpu1 = raw_histogram(g, &acc);
        let cpu2 = raw_histogram(
            g,
            &RenderSpec {
                seed: 2,
                ..acc.clone()
            },
        );
        let (gpu_raw, _) = run_chaos(gpu, g, &acc);
        let floor = total_variation(counts_cpu(&cpu1), counts_cpu(&cpu2));
        let gpu_vs_cpu = total_variation(counts_cpu(&cpu1), counts_gpu(&gpu_raw));
        let hits_cpu: f64 = counts_cpu(&cpu1).sum();
        let hits_gpu: f64 = counts_gpu(&gpu_raw).sum();
        let red_cpu = mean_red(
            cpu1.as_chunks::<4>()
                .0
                .iter()
                .map(|px| (px[1] as f64, px[0] as f64)),
        );
        let red_gpu = mean_red(
            gpu_raw
                .as_chunks::<4>()
                .0
                .iter()
                .map(|px| (px[1] as f64, px[0] as f64)),
        );
        println!(
            "{name:<18} noise floor {floor:.4}   GPU vs CPU {gpu_vs_cpu:.4}   hits CPU {hits_cpu:.0} GPU {hits_gpu:.0}   mean red CPU {red_cpu:.1} GPU {red_gpu:.1}"
        );
    }

    println!("\nThroughput: chaos iterations per second (800x600, quality 200, 96M samples)");
    let perf = RenderSpec {
        width: 800,
        height: 600,
        quality: 200.0,
        transparent: false,
        seed: 1,
        oversample: 1,
        denoise: 0.0,
    };
    let samples = perf.quality as f64 * (perf.width * perf.height) as f64;
    for (name, g) in [
        ("three variations", simple_flame()),
        ("random", random_flame()),
        ("one bright pixel", point_flame()),
    ] {
        let started = Instant::now();
        let _ = raw_histogram(&g, &perf);
        let cpu_secs = started.elapsed().as_secs_f64();
        let _ = run_chaos(gpu, &g, &perf); // warm the pipeline
        let (_, gpu_secs) = run_chaos(gpu, &g, &perf);
        println!(
            "{name:<18} CPU {:>8.1} M/s   GPU {:>8.1} M/s   speed-up {:.1}x",
            samples / cpu_secs / 1e6,
            samples / gpu_secs / 1e6,
            cpu_secs / gpu_secs
        );
    }
}

fn main() {
    let gpu = gpu_init();
    println!("GPU: {} ({})", gpu.name, gpu.backend);
    precision(&gpu);
    accuracy_and_speed(&gpu);
}
