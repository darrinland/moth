// Chaos-game kernel for the GPU spike (plan step 0.6). It is prepended to the generated
// `variation_apply`, so every variation is the same code the CPU renderer's `apply` came from.
// The loop mirrors moth-core's `flame/render.rs`: pick a transform by weight, apply its
// affine, then its variations, then its post-affine; blend the palette; plot through the
// camera into an atomic histogram of [count, r, g, b] per pixel.

struct Xf {
    cum: f32,
    color: f32,
    speed: f32,
    var_off: u32,
    aff0: vec4<f32>, // a, b, c, d
    aff1: vec4<f32>, // e, f, unused, unused
    post0: vec4<f32>,
    post1: vec4<f32>,
    var_n: u32,
    has_post: u32,
    pad0: u32,
    pad1: u32,
}

struct Params {
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

@group(0) @binding(0) var<storage, read> xfs: array<Xf>;
@group(0) @binding(1) var<storage, read> vars: array<vec4<f32>>; // (variation id, weight)
@group(0) @binding(2) var<storage, read> palette: array<vec4<u32>>; // 256 entries, 0..255 each
@group(0) @binding(3) var<storage, read_write> hist: array<atomic<u32>>;
@group(0) @binding(4) var<uniform> p: Params;

const NO_FINAL: u32 = 0xffffffffu;
const ESCAPE: f32 = 1e9;

fn pcg(state: ptr<function, u32>) -> u32 {
    let s = *state * 747796405u + 2891336453u;
    *state = s;
    let word = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (word >> 22u) ^ word;
}

// A uniform value in [0, 1) from 24 random bits.
fn unit(state: ptr<function, u32>) -> f32 {
    return f32(pcg(state) >> 8u) * (1.0 / 16777216.0);
}

// Applies transform `i` (affine, variations, post-affine) to (x, y).
fn apply_xf(i: u32, x: f32, y: f32, state: ptr<function, u32>) -> vec2<f32> {
    let xf = xfs[i];
    let a = xf.aff0.x;
    let b = xf.aff0.y;
    let c = xf.aff0.z;
    let d = xf.aff0.w;
    let e = xf.aff1.x;
    let f = xf.aff1.y;
    let px = a * x + b * y + c;
    let py = d * x + e * y + f;
    var ox = 0.0;
    var oy = 0.0;
    for (var k = 0u; k < xf.var_n; k = k + 1u) {
        let v = vars[xf.var_off + k];
        let coin = f32(pcg(state) & 1u);
        let r = variation_apply(u32(v.x), px, py, a, b, c, d, e, f, coin);
        ox = ox + v.y * r.x;
        oy = oy + v.y * r.y;
    }
    if (xf.has_post != 0u) {
        let pa = xf.post0.x;
        let pb = xf.post0.y;
        let pc = xf.post0.z;
        let pd = xf.post0.w;
        let pe = xf.post1.x;
        let pf = xf.post1.y;
        return vec2<f32>(pa * ox + pb * oy + pc, pd * ox + pe * oy + pf);
    }
    return vec2<f32>(ox, oy);
}

@compute @workgroup_size(256)
fn chaos(@builtin(global_invocation_id) gid: vec3<u32>) {
    var st = (gid.x * 0x9E3779B9u) ^ (p.seed * 0x85EBCA6Bu) ^ 0x2545F491u;
    _ = pcg(&st);
    var x = unit(&st) * 2.0 - 1.0;
    var y = unit(&st) * 2.0 - 1.0;
    var c = unit(&st);
    let total = p.iters + p.burn_in;
    for (var it = 0u; it < total; it = it + 1u) {
        let pick = unit(&st) * p.total_weight;
        var idx = p.n_xf - 1u;
        for (var i = 0u; i < p.n_xf; i = i + 1u) {
            if (pick < xfs[i].cum) {
                idx = i;
                break;
            }
        }
        let q = apply_xf(idx, x, y, &st);
        // Escaped or undefined: start the walker again, as the CPU renderer does.
        if (!(abs(q.x) <= ESCAPE && abs(q.y) <= ESCAPE)) {
            x = unit(&st) * 2.0 - 1.0;
            y = unit(&st) * 2.0 - 1.0;
            c = unit(&st);
            continue;
        }
        x = q.x;
        y = q.y;
        let xf = xfs[idx];
        c = c + (xf.color - c) * xf.speed;
        if (it < p.burn_in) {
            continue;
        }
        var fx = x;
        var fy = y;
        var fc = c;
        if (p.final_idx != NO_FINAL) {
            let fq = apply_xf(p.final_idx, x, y, &st);
            let fin = xfs[p.final_idx];
            fc = c + (fin.color - c) * fin.speed;
            fx = fq.x;
            fy = fq.y;
            if (!(abs(fx) < 3.0e38 && abs(fy) < 3.0e38)) {
                continue;
            }
        }
        let dx = fx - p.cx;
        let dy = fy - p.cy;
        let sx = (dx * p.cos_r - dy * p.sin_r) * p.ppu + p.half_w;
        let sy = (dx * p.sin_r + dy * p.cos_r) * p.ppu + p.half_h;
        if (sx >= 0.0 && sy >= 0.0 && sx < f32(p.w) && sy < f32(p.h)) {
            let pix = (u32(sy) * p.w + u32(sx)) * 4u;
            let rgb = palette[u32(clamp(fc, 0.0, 1.0) * 255.0)];
            atomicAdd(&hist[pix], 1u);
            atomicAdd(&hist[pix + 1u], rgb.x);
            atomicAdd(&hist[pix + 2u], rgb.y);
            atomicAdd(&hist[pix + 3u], rgb.z);
        }
    }
}

// Per-variation check: evaluates `variation_apply` at many probe points so the WGSL can be
// compared with the Rust `apply` point by point.
struct Probe {
    xy: vec2<f32>,
    id: u32,
    coin: f32,
    aff0: vec4<f32>,
    aff1: vec4<f32>,
}

@group(1) @binding(0) var<storage, read> probes: array<Probe>;
@group(1) @binding(1) var<storage, read_write> probe_out: array<vec2<f32>>;

@compute @workgroup_size(64)
fn probe(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&probes)) {
        return;
    }
    let pr = probes[i];
    let a = pr.aff0;
    let b = pr.aff1;
    probe_out[i] = variation_apply(pr.id, pr.xy.x, pr.xy.y, a.x, a.y, a.z, a.w, b.x, b.y, pr.coin);
}
