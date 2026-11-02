//! Signal-naturalisation DSP pipeline.
//!
//! Three sanitisation levels on top of the byte-level container cleaner:
//!
//! * `safe`           — container hygiene only, pixels untouched (handled by `metadata_cleaner`).
//! * `sensor_natural` — heteroscedastic Poisson-Gaussian sensor noise + triangular sub-LSB dither.
//! * `studio_master`  — six-stage spectral/geometric re-conditioning:
//!     A. optional inverse alpha-blend of a bottom-right semi-transparent overlay,
//!     B. smooth sub-pixel elastic resampling (bicubic, max 1.4 px, σ = 48 px field),
//!     C. 2D FFT spectral peak attenuation + mid-band phase micro-dither on Y and G,
//!     D. band-split high-frequency detail preservation,
//!     E. DCT grid shift (asymmetric crop + 99.7 % Lanczos3 resample),
//!     F. sensor noise emulation.
//!
//! All arithmetic runs on f32 planes in the 0..255 range; alpha images are processed
//! premultiplied so geometric stages cannot bleed colour into transparent pixels.

use image::RgbaImage;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use serde::{Deserialize, Serialize};
use std::f32::consts::PI;
use std::thread;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Parallel helpers (scoped std threads; no extra dependencies)
// ---------------------------------------------------------------------------

fn worker_count() -> usize {
    thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8)
}

/// Evaluates `f(0..n)` on scoped threads and returns the results in index order.
fn parallel_map<T, F>(n: usize, f: F) -> Vec<T>
where
    T: Send,
    F: Fn(usize) -> T + Sync,
{
    if n <= 1 {
        return (0..n).map(&f).collect();
    }
    let f = &f;
    thread::scope(|scope| {
        let handles: Vec<_> = (0..n).map(|i| scope.spawn(move || f(i))).collect();
        handles.into_iter().map(|h| h.join().expect("worker thread panicked")).collect()
    })
}

/// Applies `f` to every plane in parallel.
fn parallel_planes<F>(planes: &[Vec<f32>], f: F) -> Vec<Vec<f32>>
where
    F: Fn(&[f32]) -> Vec<f32> + Sync,
{
    parallel_map(planes.len(), |i| f(&planes[i]))
}

/// Splits `0..rows` into contiguous chunks, one per worker.
fn row_chunks(rows: usize) -> Vec<(usize, usize)> {
    let workers = worker_count().min(rows.max(1));
    let per = (rows + workers - 1) / workers;
    (0..workers)
        .map(|t| (t * per, ((t + 1) * per).min(rows)))
        .filter(|(a, b)| a < b)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SanitizeMode {
    #[default]
    Off,
    Safe,
    SensorNatural,
    StudioMaster,
}

impl SanitizeMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "off" | "none" => Some(SanitizeMode::Off),
            "safe" => Some(SanitizeMode::Safe),
            "sensor_natural" | "sensor" => Some(SanitizeMode::SensorNatural),
            "studio_master" | "studio" => Some(SanitizeMode::StudioMaster),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SanitizeMode::Off => "off",
            SanitizeMode::Safe => "safe",
            SanitizeMode::SensorNatural => "sensor_natural",
            SanitizeMode::StudioMaster => "studio_master",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Webp,
    Original,
    Png,
    Jpeg,
}

impl OutputFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "webp" => Some(OutputFormat::Webp),
            "original" | "same" | "keep" => Some(OutputFormat::Original),
            "png" => Some(OutputFormat::Png),
            "jpeg" | "jpg" => Some(OutputFormat::Jpeg),
            _ => None,
        }
    }
}

/// Clean attribution written into the output after every foreign trace has been removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BrandStamp {
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub copyright: String,
}

impl BrandStamp {
    pub fn is_empty(&self) -> bool {
        self.artist.trim().is_empty() && self.copyright.trim().is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SanitizeConfig {
    #[serde(default)]
    pub mode: SanitizeMode,
    #[serde(default)]
    pub deblend_corner: bool,
    #[serde(default)]
    pub output_format: OutputFormat,
    /// Optional deterministic seed for the stochastic stages (tests / reproducible batches).
    #[serde(default)]
    pub seed: Option<u64>,
    /// Optional corporate copyright/artist stamp embedded after cleaning.
    #[serde(default)]
    pub brand_stamp: Option<BrandStamp>,
    /// Name batch/MCP outputs with an SEO slug derived from the source file name.
    #[serde(default)]
    pub seo_slug: bool,
    /// Optional maximum output width in px: wider images are Lanczos-downscaled (aspect
    /// ratio kept) before any DSP stage; images at or below it are never upscaled.
    #[serde(default)]
    pub max_width: Option<u32>,
}

impl Default for SanitizeConfig {
    fn default() -> Self {
        Self {
            mode: SanitizeMode::Off,
            deblend_corner: false,
            output_format: OutputFormat::Webp,
            seed: None,
            brand_stamp: None,
            seo_slug: false,
            max_width: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineStats {
    pub mode: SanitizeMode,
    pub input_width: u32,
    pub input_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    /// Signal fidelity against the geometry-aligned reference (what the DSP stages changed).
    pub ssim: f64,
    pub psnr: f64,
    /// Fidelity against the source image resampled to the output grid (includes the
    /// intentional elastic micro-shift).
    pub ssim_source: f64,
    pub psnr_source: f64,
    pub stages: Vec<String>,
    pub corner_overlay_detected: bool,
    pub corner_overlay_alpha: f32,
    pub corner_overlay_size: String,
    pub corner_overlay_shape: String,
    pub corner_overlay_margin: u32,
    pub spectral_peaks_attenuated: u32,
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Planar f32 image
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Planes {
    pub w: usize,
    pub h: usize,
    /// RGB planes followed by an optional premultiplied-alpha plane.
    pub ch: Vec<Vec<f32>>,
}

impl Planes {
    pub fn from_rgba(img: &RgbaImage) -> Planes {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let has_alpha = img.pixels().any(|p| p[3] != 255);
        let n = w * h;
        let mut ch = vec![vec![0f32; n]; if has_alpha { 4 } else { 3 }];
        for (i, p) in img.pixels().enumerate() {
            if has_alpha {
                let a = p[3] as f32;
                let k = a / 255.0;
                ch[0][i] = p[0] as f32 * k;
                ch[1][i] = p[1] as f32 * k;
                ch[2][i] = p[2] as f32 * k;
                ch[3][i] = a;
            } else {
                ch[0][i] = p[0] as f32;
                ch[1][i] = p[1] as f32;
                ch[2][i] = p[2] as f32;
            }
        }
        Planes { w, h, ch }
    }

    pub fn has_alpha(&self) -> bool {
        self.ch.len() == 4
    }

    pub fn to_rgba(&self) -> RgbaImage {
        let mut img = RgbaImage::new(self.w as u32, self.h as u32);
        for (i, p) in img.pixels_mut().enumerate() {
            if self.has_alpha() {
                let a = self.ch[3][i].clamp(0.0, 255.0);
                let a_q = a.round();
                // Fully transparent output pixels carry no colour (premultiplied storage).
                let k = if a_q > 0.0 { 255.0 / a } else { 0.0 };
                p[0] = (self.ch[0][i] * k).round().clamp(0.0, 255.0) as u8;
                p[1] = (self.ch[1][i] * k).round().clamp(0.0, 255.0) as u8;
                p[2] = (self.ch[2][i] * k).round().clamp(0.0, 255.0) as u8;
                p[3] = a_q as u8;
            } else {
                p[0] = self.ch[0][i].round().clamp(0.0, 255.0) as u8;
                p[1] = self.ch[1][i].round().clamp(0.0, 255.0) as u8;
                p[2] = self.ch[2][i].round().clamp(0.0, 255.0) as u8;
                p[3] = 255;
            }
        }
        img
    }

    pub fn luma(&self) -> Vec<f32> {
        let n = self.w * self.h;
        let mut y = vec![0f32; n];
        for i in 0..n {
            y[i] = 0.299 * self.ch[0][i] + 0.587 * self.ch[1][i] + 0.114 * self.ch[2][i];
        }
        y
    }

    fn alpha_weight(&self, i: usize) -> f32 {
        if self.has_alpha() {
            (self.ch[3][i] / 255.0).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }
}

// ---------------------------------------------------------------------------
// Random helpers
// ---------------------------------------------------------------------------

struct Gaussian {
    spare: Option<f32>,
}

impl Gaussian {
    fn new() -> Self {
        Self { spare: None }
    }

    /// Standard normal sample via Box-Muller.
    fn sample(&mut self, rng: &mut StdRng) -> f32 {
        if let Some(s) = self.spare.take() {
            return s;
        }
        let u1: f32 = loop {
            let v: f32 = rng.gen();
            if v > 1e-12 {
                break v;
            }
        };
        let u2: f32 = rng.gen();
        let r = (-2.0 * u1.ln()).sqrt();
        let theta = 2.0 * PI * u2;
        self.spare = Some(r * theta.sin());
        r * theta.cos()
    }
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Deterministic uniform value in [-1, 1) derived from (seed, index).
fn hash_uniform(seed: u64, index: u64) -> f32 {
    let v = splitmix64(seed ^ index.wrapping_mul(0xD6E8_FEB8_6659_FD93));
    ((v >> 11) as f64 / (1u64 << 53) as f64) as f32 * 2.0 - 1.0
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    let radius = (3.0 * sigma).ceil().max(1.0) as usize;
    let mut k = Vec::with_capacity(2 * radius + 1);
    let mut sum = 0.0f32;
    for i in 0..=2 * radius {
        let x = i as f32 - radius as f32;
        let v = (-(x * x) / (2.0 * sigma * sigma)).exp();
        k.push(v);
        sum += v;
    }
    for v in &mut k {
        *v /= sum;
    }
    k
}

/// Separable convolution with clamp-to-edge boundary handling.
fn convolve_separable(src: &[f32], w: usize, h: usize, kernel: &[f32]) -> Vec<f32> {
    let r = kernel.len() / 2;
    let mut tmp = vec![0f32; w * h];
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        let out = &mut tmp[y * w..(y + 1) * w];
        for x in 0..w {
            let mut acc = 0.0f32;
            for (k, &kv) in kernel.iter().enumerate() {
                let sx = (x as isize + k as isize - r as isize).clamp(0, w as isize - 1) as usize;
                acc += row[sx] * kv;
            }
            out[x] = acc;
        }
    }
    let mut dst = vec![0f32; w * h];
    let mut col = vec![0f32; h];
    for x in 0..w {
        for y in 0..h {
            col[y] = tmp[y * w + x];
        }
        for y in 0..h {
            let mut acc = 0.0f32;
            for (k, &kv) in kernel.iter().enumerate() {
                let sy = (y as isize + k as isize - r as isize).clamp(0, h as isize - 1) as usize;
                acc += col[sy] * kv;
            }
            dst[y * w + x] = acc;
        }
    }
    dst
}

fn box_blur_line(src: &[f32], dst: &mut [f32], radius: usize) {
    let n = src.len();
    if n == 0 {
        return;
    }
    let clamp = |i: isize| -> usize { i.clamp(0, n as isize - 1) as usize };
    let win = (2 * radius + 1) as f64;
    let mut sum = 0.0f64;
    for k in -(radius as isize)..=(radius as isize) {
        sum += src[clamp(k)] as f64;
    }
    dst[0] = (sum / win) as f32;
    for i in 1..n {
        sum += src[clamp(i as isize + radius as isize)] as f64;
        sum -= src[clamp(i as isize - 1 - radius as isize)] as f64;
        dst[i] = (sum / win) as f32;
    }
}

/// Three successive box blurs per axis ≈ Gaussian with the requested sigma, in O(N).
fn gaussian_approx_box(field: &mut Vec<f32>, w: usize, h: usize, sigma: f32) {
    let width = (4.0 * sigma * sigma + 1.0).sqrt();
    let radius = ((width - 1.0) / 2.0).round().max(1.0) as usize;
    let mut tmp = vec![0f32; w * h];
    let mut line = vec![0f32; h.max(w)];
    for _ in 0..3 {
        for y in 0..h {
            box_blur_line(&field[y * w..(y + 1) * w], &mut tmp[y * w..(y + 1) * w], radius);
        }
        field.copy_from_slice(&tmp);
    }
    let mut col_src = vec![0f32; h];
    for _ in 0..3 {
        for x in 0..w {
            for y in 0..h {
                col_src[y] = field[y * w + x];
            }
            box_blur_line(&col_src, &mut line[..h], radius);
            for y in 0..h {
                field[y * w + x] = line[y];
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Sampling
// ---------------------------------------------------------------------------

#[inline]
fn cubic_weight(t: f32) -> f32 {
    // Keys cubic, a = -0.5 (Catmull-Rom)
    let a = -0.5f32;
    let t = t.abs();
    if t < 1.0 {
        (a + 2.0) * t * t * t - (a + 3.0) * t * t + 1.0
    } else if t < 2.0 {
        a * t * t * t - 5.0 * a * t * t + 8.0 * a * t - 4.0 * a
    } else {
        0.0
    }
}

#[inline]
fn bicubic_sample(plane: &[f32], w: usize, h: usize, x: f32, y: f32) -> f32 {
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;
    let xi = x0 as isize;
    let yi = y0 as isize;
    let mut acc = 0.0f32;
    let mut wsum = 0.0f32;
    for j in -1..=2isize {
        let wy = cubic_weight(fy - j as f32);
        if wy == 0.0 {
            continue;
        }
        let sy = (yi + j).clamp(0, h as isize - 1) as usize;
        let row = &plane[sy * w..(sy + 1) * w];
        for i in -1..=2isize {
            let wx = cubic_weight(fx - i as f32);
            if wx == 0.0 {
                continue;
            }
            let sx = (xi + i).clamp(0, w as isize - 1) as usize;
            let wgt = wx * wy;
            acc += row[sx] * wgt;
            wsum += wgt;
        }
    }
    if wsum.abs() > 1e-6 {
        acc / wsum
    } else {
        acc
    }
}

fn lanczos3(x: f32) -> f32 {
    if x == 0.0 {
        return 1.0;
    }
    if x.abs() >= 3.0 {
        return 0.0;
    }
    let px = PI * x;
    3.0 * px.sin() * (px / 3.0).sin() / (px * px)
}

fn lanczos_weights(src_len: usize, dst_len: usize) -> Vec<(usize, Vec<f32>)> {
    let scale = src_len as f32 / dst_len as f32;
    let filter_scale = scale.max(1.0);
    let support = 3.0 * filter_scale;
    let mut out = Vec::with_capacity(dst_len);
    for i in 0..dst_len {
        let center = (i as f32 + 0.5) * scale - 0.5;
        let start = ((center - support).floor().max(0.0)) as usize;
        let end = ((center + support).ceil().min(src_len as f32 - 1.0)) as usize;
        let mut weights = Vec::with_capacity(end - start + 1);
        let mut sum = 0.0f32;
        for j in start..=end {
            let wgt = lanczos3((j as f32 - center) / filter_scale);
            weights.push(wgt);
            sum += wgt;
        }
        if sum.abs() > 1e-8 {
            for wgt in &mut weights {
                *wgt /= sum;
            }
        }
        out.push((start, weights));
    }
    out
}

fn lanczos_resize_plane(src: &[f32], w: usize, h: usize, nw: usize, nh: usize) -> Vec<f32> {
    let wx = lanczos_weights(w, nw);
    let mut tmp = vec![0f32; nw * h];
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        for (x, (start, weights)) in wx.iter().enumerate() {
            let mut acc = 0.0f32;
            for (k, wgt) in weights.iter().enumerate() {
                acc += row[start + k] * wgt;
            }
            tmp[y * nw + x] = acc;
        }
    }
    let wy = lanczos_weights(h, nh);
    let mut dst = vec![0f32; nw * nh];
    for (y, (start, weights)) in wy.iter().enumerate() {
        for x in 0..nw {
            let mut acc = 0.0f32;
            for (k, wgt) in weights.iter().enumerate() {
                acc += tmp[(start + k) * nw + x] * wgt;
            }
            dst[y * nw + x] = acc;
        }
    }
    dst
}

fn lanczos_resize(p: &Planes, nw: usize, nh: usize) -> Planes {
    Planes {
        w: nw,
        h: nh,
        ch: parallel_planes(&p.ch, |c| lanczos_resize_plane(c, p.w, p.h, nw, nh)),
    }
}

fn crop(p: &Planes, left: usize, top: usize) -> Planes {
    let nw = p.w - left;
    let nh = p.h - top;
    let mut ch = Vec::with_capacity(p.ch.len());
    for c in &p.ch {
        let mut out = vec![0f32; nw * nh];
        for y in 0..nh {
            out[y * nw..(y + 1) * nw].copy_from_slice(&c[(y + top) * p.w + left..(y + top) * p.w + left + nw]);
        }
        ch.push(out);
    }
    Planes { w: nw, h: nh, ch }
}

// ---------------------------------------------------------------------------
// Stage A — bottom-right overlay inverse blend
// ---------------------------------------------------------------------------

pub const CORNER_BOX_CANDIDATES: &[(usize, usize)] = &[(36, 36), (48, 36), (48, 48), (96, 96)];
pub const CORNER_STAR_CANDIDATES: &[(usize, usize)] = &[(36, 36), (48, 48), (96, 96)];
/// Badges may sit up to this many pixels inside the bottom-right corner.
pub const CORNER_MARGIN_MAX: usize = 48;
pub const CORNER_MARGIN_STEP: usize = 4;
/// Background ring (in px) around the template bounding box used for the plane fit and NCC.
pub const CORNER_RING: usize = 8;
pub const CORNER_NCC_MIN: f32 = 0.6;
pub const CORNER_STEP_MIN: f32 = 4.0;
/// A badge may not span more than this fraction of the image width/height.
pub const CORNER_MAX_FRACTION: f32 = 0.5;
/// The candidate template must beat a convex (elliptic) null template by this NCC margin,
/// which rejects soft radial highlights that merely happen to sit in the corner.
pub const CORNER_NULL_MARGIN: f32 = 0.05;
/// Astroid exponent: (|dx/rx|^e + |dy/ry|^e) <= 1 with e < 1 gives a concave four-pointed star.
pub const ASTROID_EXPONENT: f32 = 0.65;
pub const ASTROID_PROFILE_GAMMA: f32 = 0.75;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayShape {
    Box,
    Star,
}

impl OverlayShape {
    pub fn label(self) -> &'static str {
        match self {
            OverlayShape::Box => "box",
            OverlayShape::Star => "star",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CornerOverlay {
    pub shape: OverlayShape,
    pub x0: usize,
    pub y0: usize,
    pub width: usize,
    pub height: usize,
    pub margin: usize,
    /// Peak overlay opacity; the per-pixel opacity is `alpha * template(x, y)`.
    pub alpha: f32,
    /// Normalised cross-correlation with the template.
    pub score: f32,
}

/// Per-pixel alpha template in [0, 1] for a w×h badge.
/// * `Box`: flat 1 with a 2-px linear feather along the edges.
/// * `Star`: concave astroid `(|dx/rx|^0.65 + |dy/ry|^0.65) <= 1` whose opacity rises toward
///   the centre: `(1 − v)^γ`.
pub fn overlay_template(shape: OverlayShape, w: usize, h: usize) -> Vec<f32> {
    let mut t = vec![0f32; w * h];
    let rx = w as f32 / 2.0;
    let ry = h as f32 / 2.0;
    for y in 0..h {
        for x in 0..w {
            t[y * w + x] = match shape {
                OverlayShape::Box => {
                    let ex = (x + 1).min(w - x) as f32;
                    let ey = (y + 1).min(h - y) as f32;
                    (ex.min(ey) / 2.0).min(1.0)
                }
                OverlayShape::Star => {
                    let dx = ((x as f32 + 0.5 - rx) / rx).abs();
                    let dy = ((y as f32 + 0.5 - ry) / ry).abs();
                    let v = dx.powf(ASTROID_EXPONENT) + dy.powf(ASTROID_EXPONENT);
                    (1.0 - v).max(0.0).powf(ASTROID_PROFILE_GAMMA)
                }
            };
        }
    }
    t
}

/// Convex elliptic null template (same bounding box, same centre-bright profile) used to
/// reject soft radial highlights that are neither a box nor a concave star.
fn null_template(w: usize, h: usize) -> Vec<f32> {
    let mut t = vec![0f32; w * h];
    let rx = w as f32 / 2.0;
    let ry = h as f32 / 2.0;
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f32 + 0.5 - rx) / rx;
            let dy = (y as f32 + 0.5 - ry) / ry;
            let v = (dx * dx + dy * dy).sqrt();
            t[y * w + x] = (1.0 - v).max(0.0).powf(ASTROID_PROFILE_GAMMA);
        }
    }
    t
}

/// Least-squares background surface `a + b·x + c·y + d·x² + e·x·y + f·y²` fitted to the
/// samples (falls back to a plane when there are too few samples). Returns the six
/// coefficients; unused ones are zero.
fn fit_background(samples: &[(f32, f32, f32)]) -> [f64; 6] {
    let quadratic = samples.len() >= 60;
    let k = if quadratic { 6 } else { 3 };
    let mut ata = [[0f64; 6]; 6];
    let mut atb = [0f64; 6];
    if samples.len() < 3 {
        let mean = samples.iter().map(|s| s.2 as f64).sum::<f64>() / samples.len().max(1) as f64;
        return [mean, 0.0, 0.0, 0.0, 0.0, 0.0];
    }
    for &(x, y, z) in samples {
        let (x, y, z) = (x as f64, y as f64, z as f64);
        let basis = [1.0, x, y, x * x, x * y, y * y];
        for i in 0..k {
            for j in 0..k {
                ata[i][j] += basis[i] * basis[j];
            }
            atb[i] += basis[i] * z;
        }
    }
    // Gaussian elimination with partial pivoting.
    let mut m = ata;
    let mut v = atb;
    for col in 0..k {
        let mut pivot = col;
        for r in col + 1..k {
            if m[r][col].abs() > m[pivot][col].abs() {
                pivot = r;
            }
        }
        if m[pivot][col].abs() < 1e-12 {
            let mean = samples.iter().map(|s| s.2 as f64).sum::<f64>() / samples.len() as f64;
            return [mean, 0.0, 0.0, 0.0, 0.0, 0.0];
        }
        m.swap(col, pivot);
        v.swap(col, pivot);
        for r in col + 1..k {
            let f = m[r][col] / m[col][col];
            for c in col..k {
                m[r][c] -= f * m[col][c];
            }
            v[r] -= f * v[col];
        }
    }
    let mut out = [0f64; 6];
    for col in (0..k).rev() {
        let mut acc = v[col];
        for c in col + 1..k {
            acc -= m[col][c] * out[c];
        }
        out[col] = acc / m[col][col];
    }
    out
}

#[inline]
fn eval_background(c: &[f64; 6], x: f32, y: f32) -> f32 {
    let (x, y) = (x as f64, y as f64);
    (c[0] + c[1] * x + c[2] * y + c[3] * x * x + c[4] * x * y + c[5] * y * y) as f32
}

/// Detects a semi-transparent bright badge in the bottom-right corner. Every candidate
/// template (flat box 36×36/48×36/48×48/96×96, astroid star 36×36/48×48/96×96) is slid over
/// margins 0..=48 px; for each placement a quadratic background surface is fitted on the
/// ring around the template, and the normalised cross-correlation between the residual
/// luminance and the template is scored. A placement is accepted when NCC ≥ 0.6, the
/// brightness step is positive, the badge spans at most half of the image, and the
/// template beats a convex elliptic null template (soft highlights are not badges). The
/// best NCC wins; its peak opacity is the least-squares inverse-blend estimate.
pub fn detect_corner_overlay(p: &Planes) -> Option<CornerOverlay> {
    let luma = p.luma();
    let (w, h) = (p.w, p.h);
    let mut best: Option<CornerOverlay> = None;
    let candidates = CORNER_BOX_CANDIDATES
        .iter()
        .map(|&(cw, ch)| (OverlayShape::Box, cw, ch))
        .chain(CORNER_STAR_CANDIDATES.iter().map(|&(cw, ch)| (OverlayShape::Star, cw, ch)));

    for (shape, rw, rh) in candidates {
        if rw as f32 > w as f32 * CORNER_MAX_FRACTION || rh as f32 > h as f32 * CORNER_MAX_FRACTION {
            continue;
        }
        let tmpl = overlay_template(shape, rw, rh);
        let null = null_template(rw, rh);
        let tmpl_sum: f64 = tmpl.iter().map(|&v| v as f64).sum();
        let tmpl_sq: f64 = tmpl.iter().map(|&v| (v as f64) * (v as f64)).sum();
        let null_sum: f64 = null.iter().map(|&v| v as f64).sum();
        let null_sq: f64 = null.iter().map(|&v| (v as f64) * (v as f64)).sum();
        if tmpl_sum <= 0.0 || null_sum <= 0.0 {
            continue;
        }
        let mut margin = 0usize;
        while margin <= CORNER_MARGIN_MAX {
            if rw + margin + CORNER_RING > w || rh + margin + CORNER_RING > h {
                break;
            }
            let x0 = w - margin - rw;
            let y0 = h - margin - rh;
            let wx0 = x0 - CORNER_RING;
            let wy0 = y0 - CORNER_RING;
            let wx1 = (x0 + rw + CORNER_RING).min(w);
            let wy1 = (y0 + rh + CORNER_RING).min(h);

            // Background surface from the ring (window minus template bbox).
            let mut ring: Vec<(f32, f32, f32)> = Vec::with_capacity((wx1 - wx0) * (wy1 - wy0));
            for y in wy0..wy1 {
                for x in wx0..wx1 {
                    let in_bbox = x >= x0 && x < x0 + rw && y >= y0 && y < y0 + rh;
                    if !in_bbox {
                        ring.push(((x - wx0) as f32, (y - wy0) as f32, luma[y * w + x]));
                    }
                }
            }
            let coeffs = fit_background(&ring);

            // NCC of the residual luminance against the candidate and the null template, plus
            // the least-squares opacity for the model  res = α · T(x,y) · (255 − bg(x,y)).
            let n = ((wx1 - wx0) * (wy1 - wy0)) as f64;
            let (mut sl, mut sll, mut slt, mut sln) = (0f64, 0f64, 0f64, 0f64);
            let (mut num_alpha, mut den_alpha) = (0f64, 0f64);
            for y in wy0..wy1 {
                for x in wx0..wx1 {
                    let bg = eval_background(&coeffs, (x - wx0) as f32, (y - wy0) as f32);
                    let res = (luma[y * w + x] - bg) as f64;
                    sl += res;
                    sll += res * res;
                    if x >= x0 && x < x0 + rw && y >= y0 && y < y0 + rh {
                        let ti = (y - y0) * rw + (x - x0);
                        let t = tmpl[ti] as f64;
                        slt += res * t;
                        sln += res * null[ti] as f64;
                        let basis = t * (255.0 - bg.clamp(0.0, 254.0)) as f64;
                        num_alpha += res * basis;
                        den_alpha += basis * basis;
                    }
                }
            }
            let var_l = sll - sl * sl / n;
            let ncc_of = |s_lt: f64, t_sum: f64, t_sq: f64| -> f32 {
                let var_t = t_sq - t_sum * t_sum / n;
                let cov = s_lt - sl * t_sum / n;
                if var_l > 1e-9 && var_t > 1e-9 { (cov / (var_l * var_t).sqrt()) as f32 } else { 0.0 }
            };
            let ncc = ncc_of(slt, tmpl_sum, tmpl_sq);
            let ncc_null = ncc_of(sln, null_sum, null_sq);
            let step = (slt / tmpl_sum) as f32;
            if ncc >= CORNER_NCC_MIN && step >= CORNER_STEP_MIN && ncc >= ncc_null + CORNER_NULL_MARGIN {
                let alpha = if den_alpha > 1e-9 { (num_alpha / den_alpha) as f32 } else { 0.0 };
                let cand = CornerOverlay {
                    shape,
                    x0,
                    y0,
                    width: rw,
                    height: rh,
                    margin,
                    alpha: alpha.clamp(0.03, 0.9),
                    score: ncc,
                };
                if best.map(|b| cand.score > b.score).unwrap_or(true) {
                    best = Some(cand);
                }
            }
            margin += CORNER_MARGIN_STEP;
        }
    }
    best
}

/// Per-pixel inverse alpha blend of a white overlay using the template opacity
/// α(x,y) = alpha · template(x,y):  I_orig = (I − α(x,y)·255) / (1 − α(x,y)).
pub fn deblend_corner(p: &mut Planes, ov: &CornerOverlay) {
    let w = p.w;
    let tmpl = overlay_template(ov.shape, ov.width, ov.height);
    for ty in 0..ov.height {
        for tx in 0..ov.width {
            let a = ov.alpha * tmpl[ty * ov.width + tx];
            if a <= 1e-4 {
                continue;
            }
            let i = (ov.y0 + ty) * w + ov.x0 + tx;
            let cover = p.alpha_weight(i);
            let inv = 1.0 / (1.0 - a);
            for c in 0..3 {
                let v = p.ch[c][i];
                p.ch[c][i] = ((v - a * 255.0 * cover) * inv).clamp(0.0, 255.0 * cover.max(1e-3));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Stage B — smooth elastic resampling
// ---------------------------------------------------------------------------

pub const ELASTIC_SIGMA_COARSE: f32 = 48.0;
pub const ELASTIC_SIGMA_FINE: f32 = 16.0;
/// Weight of the fine-scale field in the combined displacement.
pub const ELASTIC_FINE_WEIGHT: f32 = 0.35;
pub const ELASTIC_MAX_PX: f32 = 1.8;

/// Random field smoothed with the given sigma and normalised to unit peak magnitude.
fn smooth_unit_field(w: usize, h: usize, rng: &mut StdRng, sigma: f32) -> (Vec<f32>, Vec<f32>) {
    let n = w * h;
    let mut dx: Vec<f32> = (0..n).map(|_| rng.gen::<f32>() * 2.0 - 1.0).collect();
    let mut dy: Vec<f32> = (0..n).map(|_| rng.gen::<f32>() * 2.0 - 1.0).collect();
    gaussian_approx_box(&mut dx, w, h, sigma);
    gaussian_approx_box(&mut dy, w, h, sigma);
    let max_mag = (0..n).map(|i| (dx[i] * dx[i] + dy[i] * dy[i]).sqrt()).fold(0.0f32, f32::max);
    if max_mag > 1e-9 {
        for i in 0..n {
            dx[i] /= max_mag;
            dy[i] /= max_mag;
        }
    }
    (dx, dy)
}

/// Multi-scale smooth elastic resampling: a σ=48 px coarse field plus a σ=16 px fine field
/// (weighted sum), normalised to a 1.8 px peak displacement, sampled bicubically.
pub fn elastic_resample(p: &Planes, rng: &mut StdRng) -> Planes {
    let (w, h) = (p.w, p.h);
    let n = w * h;
    let (cx, cy) = smooth_unit_field(w, h, rng, ELASTIC_SIGMA_COARSE);
    let (fx, fy) = smooth_unit_field(w, h, rng, ELASTIC_SIGMA_FINE);
    let wc = 1.0 - ELASTIC_FINE_WEIGHT;
    let mut dx: Vec<f32> = (0..n).map(|i| wc * cx[i] + ELASTIC_FINE_WEIGHT * fx[i]).collect();
    let mut dy: Vec<f32> = (0..n).map(|i| wc * cy[i] + ELASTIC_FINE_WEIGHT * fy[i]).collect();
    drop((cx, cy, fx, fy));
    let max_mag = (0..n).map(|i| (dx[i] * dx[i] + dy[i] * dy[i]).sqrt()).fold(0.0f32, f32::max);
    if max_mag > 1e-9 {
        let k = ELASTIC_MAX_PX / max_mag;
        for i in 0..n {
            dx[i] *= k;
            dy[i] *= k;
        }
    }
    let ch = parallel_planes(&p.ch, |plane| {
        let mut out = vec![0f32; n];
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                out[i] = bicubic_sample(plane, w, h, x as f32 + dx[i], y as f32 + dy[i]);
            }
        }
        out
    });
    Planes { w, h, ch }
}

// ---------------------------------------------------------------------------
// Stage C — spectral peak attenuation and phase micro-dither
// ---------------------------------------------------------------------------

pub const PEAK_BAND: (f32, f32) = (0.12, 0.85);
pub const PEAK_RATIO: f32 = 2.2;
pub const PEAK_RESIDUAL: f32 = 0.15;
/// Low–mid carrier ring that receives the phase perturbation.
pub const PHASE_BAND: (f32, f32) = (0.04, 0.38);
/// Peak phase perturbation for the weakest bins.
pub const PHASE_MAX_RAD: f32 = 0.45;
/// Energy weighting knee: Δφ = PHASE_MAX_RAD / (1 + knee · (|F| / median_ring)²), so bins at the
/// ring's median energy get ≈ 0.35 rad while strong texture carriers keep their phase.
pub const PHASE_ENERGY_KNEE: f32 = 0.3;

fn fft2d(buf: &mut Vec<Complex<f32>>, w: usize, h: usize, inverse: bool) {
    let mut planner = FftPlanner::<f32>::new();
    let fft_row = if inverse { planner.plan_fft_inverse(w) } else { planner.plan_fft_forward(w) };
    let fft_col = if inverse { planner.plan_fft_inverse(h) } else { planner.plan_fft_forward(h) };

    // Rows: every chunk holds whole rows, so each worker runs a batch of 1-D FFTs.
    let per_rows = row_chunks(h).first().map(|(a, b)| b - a).unwrap_or(h).max(1);
    thread::scope(|scope| {
        for chunk in buf.chunks_mut(per_rows * w) {
            let fft = fft_row.clone();
            scope.spawn(move || {
                let mut scratch = vec![Complex::new(0.0f32, 0.0); fft.get_inplace_scratch_len()];
                fft.process_with_scratch(chunk, &mut scratch);
            });
        }
    });

    // Columns: transpose so columns become contiguous rows of length h.
    let mut t = vec![Complex::new(0.0f32, 0.0); w * h];
    for y in 0..h {
        for x in 0..w {
            t[x * h + y] = buf[y * w + x];
        }
    }
    let per_cols = row_chunks(w).first().map(|(a, b)| b - a).unwrap_or(w).max(1);
    thread::scope(|scope| {
        for chunk in t.chunks_mut(per_cols * h) {
            let fft = fft_col.clone();
            scope.spawn(move || {
                let mut scratch = vec![Complex::new(0.0f32, 0.0); fft.get_inplace_scratch_len()];
                fft.process_with_scratch(chunk, &mut scratch);
            });
        }
    });
    for x in 0..w {
        for y in 0..h {
            buf[y * w + x] = t[x * h + y];
        }
    }
}

#[inline]
fn norm_radius(ky: usize, kx: usize, w: usize, h: usize) -> f32 {
    let fy = if ky <= h / 2 { ky as f32 } else { ky as f32 - h as f32 };
    let fx = if kx <= w / 2 { kx as f32 } else { kx as f32 - w as f32 };
    let ry = fy / (h as f32 / 2.0);
    let rx = fx / (w as f32 / 2.0);
    (ry * ry + rx * rx).sqrt()
}

#[inline]
fn median_of_25(win: &mut [f32; 25]) -> f32 {
    let (_, m, _) = win.select_nth_unstable_by(12, |a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    *m
}

/// 5×5 median around (ky, kx) on the periodic spectrum; interior bins use direct indexing.
#[inline]
fn median25(mag: &[f32], w: usize, h: usize, ky: usize, kx: usize) -> f32 {
    let mut win = [0f32; 25];
    if ky >= 2 && ky + 2 < h && kx >= 2 && kx + 2 < w {
        let mut n = 0;
        for dy in 0..5 {
            let row = (ky + dy - 2) * w + kx - 2;
            win[n..n + 5].copy_from_slice(&mag[row..row + 5]);
            n += 5;
        }
    } else {
        let mut n = 0;
        for dy in -2..=2isize {
            let sy = (ky as isize + dy).rem_euclid(h as isize) as usize;
            for dx in -2..=2isize {
                let sx = (kx as isize + dx).rem_euclid(w as isize) as usize;
                win[n] = mag[sy * w + sx];
                n += 1;
            }
        }
    }
    median_of_25(&mut win)
}

/// Spectrum of the smooth component of Moisan's periodic-plus-smooth decomposition.
/// Subtracting it from the image spectrum removes the cross-shaped ridge that the
/// non-periodic image borders would otherwise imprint on the FFT.
fn smooth_component_spectrum(plane: &[f32], w: usize, h: usize) -> Vec<Complex<f32>> {
    let mut v = vec![Complex::new(0.0f32, 0.0); w * h];
    // v(0,x) = u(h-1,x) - u(0,x); v(h-1,x) = u(0,x) - u(h-1,x); likewise for columns.
    for x in 0..w {
        let d = plane[(h - 1) * w + x] - plane[x];
        v[x].re += d;
        v[(h - 1) * w + x].re -= d;
    }
    for y in 0..h {
        let d = plane[y * w + w - 1] - plane[y * w];
        v[y * w].re += d;
        v[y * w + w - 1].re -= d;
    }
    fft2d(&mut v, w, h, false);
    for ky in 0..h {
        let cy = (2.0 * PI * ky as f32 / h as f32).cos();
        for kx in 0..w {
            let i = ky * w + kx;
            if i == 0 {
                v[i] = Complex::new(0.0, 0.0);
                continue;
            }
            let cx = (2.0 * PI * kx as f32 / w as f32).cos();
            let denom = 2.0 * cy + 2.0 * cx - 4.0;
            v[i] = if denom.abs() < 1e-9 { Complex::new(0.0, 0.0) } else { v[i] / denom };
        }
    }
    v
}

struct SpectralOut {
    /// Peaks attenuated, phase untouched (only when requested).
    peakless: Option<Vec<f32>>,
    /// Peaks attenuated (if enabled) and phase-dithered (if enabled).
    conditioned: Vec<f32>,
    peaks: u32,
}

fn inverse_real(spec: &[Complex<f32>], smooth: &[Complex<f32>], w: usize, h: usize) -> Vec<f32> {
    let n = w * h;
    let mut buf: Vec<Complex<f32>> = (0..n).map(|i| spec[i] + smooth[i]).collect();
    fft2d(&mut buf, w, h, true);
    let scale = 1.0 / n as f32;
    buf.iter().map(|c| c.re * scale).collect()
}

/// One forward transform, optional peak attenuation, optional phase dither, and up to
/// two inverse transforms (peak-free reference and fully conditioned plane).
fn spectral_core(plane: &[f32], w: usize, h: usize, seed: u64, attenuate: bool, dither: bool, want_peakless: bool) -> SpectralOut {
    let n = w * h;
    let smooth = smooth_component_spectrum(plane, w, h);
    let mut spec: Vec<Complex<f32>> = plane.iter().map(|&v| Complex::new(v, 0.0)).collect();
    fft2d(&mut spec, w, h, false);
    // Work on the periodic component only; the smooth border component is re-added untouched.
    for i in 0..n {
        spec[i] -= smooth[i];
    }

    let mut peaks = 0u32;
    if attenuate {
        let mag: Vec<f32> = spec.iter().map(|c| c.norm()).collect();
        let chunks = row_chunks(h);
        let found: Vec<Vec<(usize, f32)>> = parallel_map(chunks.len(), |t| {
            let (y0, y1) = chunks[t];
            let mut out = Vec::new();
            for ky in y0..y1 {
                for kx in 0..w {
                    let r = norm_radius(ky, kx, w, h);
                    if r > PEAK_BAND.0 && r < PEAK_BAND.1 {
                        let i = ky * w + kx;
                        let m = mag[i];
                        if m <= 1e-6 {
                            continue;
                        }
                        let med = median25(&mag, w, h, ky, kx);
                        if med > 1e-6 && m > PEAK_RATIO * med {
                            let target = med + (m - med) * PEAK_RESIDUAL;
                            out.push((i, target / m));
                        }
                    }
                }
            }
            out
        });
        for list in &found {
            for &(i, scale) in list {
                spec[i] *= scale;
            }
            peaks += list.len() as u32;
        }
    }

    let peakless = if want_peakless { Some(inverse_real(&spec, &smooth, w, h)) } else { None };

    if dither {
        // Ring median magnitude (after attenuation) for the energy weighting.
        let mut ring: Vec<f32> = Vec::new();
        for ky in 0..h {
            for kx in 0..w {
                let r = norm_radius(ky, kx, w, h);
                if r > PHASE_BAND.0 && r < PHASE_BAND.1 {
                    ring.push(spec[ky * w + kx].norm());
                }
            }
        }
        let ring_med = if ring.is_empty() {
            1.0
        } else {
            let mid = ring.len() / 2;
            *ring.select_nth_unstable_by(mid, |a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)).1
        }
        .max(1e-6);
        let per_rows = row_chunks(h).first().map(|(a, b)| b - a).unwrap_or(h).max(1);
        thread::scope(|scope| {
            for (c, chunk) in spec.chunks_mut(per_rows * w).enumerate() {
                scope.spawn(move || {
                    let ky0 = c * per_rows;
                    for (j, bin) in chunk.iter_mut().enumerate() {
                        let ky = ky0 + j / w;
                        let kx = j % w;
                        let r = norm_radius(ky, kx, w, h);
                        if r <= PHASE_BAND.0 || r >= PHASE_BAND.1 {
                            continue;
                        }
                        let i = ky * w + kx;
                        let my = (h - ky) % h;
                        let mx = (w - kx) % w;
                        let mirror = my * w + mx;
                        if mirror == i {
                            continue;
                        }
                        let canonical = i.min(mirror) as u64;
                        let sign = if i < mirror { 1.0 } else { -1.0 };
                        let rel = bin.norm() / ring_med;
                        let weight = 1.0 / (1.0 + PHASE_ENERGY_KNEE * rel * rel);
                        let dphi = sign * hash_uniform(seed, canonical) * PHASE_MAX_RAD * weight;
                        *bin *= Complex::from_polar(1.0, dphi);
                    }
                });
            }
        });
    }

    let conditioned = inverse_real(&spec, &smooth, w, h);
    SpectralOut { peakless, conditioned, peaks }
}

/// Spectral conditioning of one plane on top of Moisan's periodic component:
/// * `attenuate` — narrow periodic peaks (`|F| > 2.2 × local 5×5 median`, 0.12 < r < 0.85)
///   are pulled toward the local median;
/// * `dither` — bins in the low–mid carrier ring (0.04 < r < 0.38) receive an antisymmetric
///   phase perturbation whose amplitude is inversely weighted by the bin's energy.
///
/// Returns the processed plane and the number of attenuated bins.
pub fn spectral_condition(plane: &[f32], w: usize, h: usize, seed: u64, attenuate: bool, dither: bool) -> (Vec<f32>, u32) {
    let out = spectral_core(plane, w, h, seed, attenuate, dither, false);
    (out.conditioned, out.peaks)
}

fn to_ycbcr(p: &Planes) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let n = p.w * p.h;
    let mut y = vec![0f32; n];
    let mut cb = vec![0f32; n];
    let mut cr = vec![0f32; n];
    for i in 0..n {
        let (r, g, b) = (p.ch[0][i], p.ch[1][i], p.ch[2][i]);
        y[i] = 0.299 * r + 0.587 * g + 0.114 * b;
        cb[i] = -0.168_736 * r - 0.331_264 * g + 0.5 * b;
        cr[i] = 0.5 * r - 0.418_688 * g - 0.081_312 * b;
    }
    (y, cb, cr)
}

fn from_ycbcr(template: &Planes, y: &[f32], cb: &[f32], cr: &[f32]) -> Planes {
    let n = template.w * template.h;
    let mut ch = vec![vec![0f32; n]; 3];
    for i in 0..n {
        ch[0][i] = y[i] + 1.402 * cr[i];
        ch[1][i] = y[i] - 0.344_136 * cb[i] - 0.714_136 * cr[i];
        ch[2][i] = y[i] + 1.772 * cb[i];
    }
    if template.has_alpha() {
        ch.push(template.ch[3].clone());
    }
    Planes { w: template.w, h: template.h, ch }
}

/// Stage C on the elastic reference. Returns `(peakless, conditioned, peaks)`:
/// * `peakless` — reference with only the narrow spectral peaks removed on Y and G
///   (no phase change), used by Stage D so detail restoration cannot re-inject the peaks;
/// * `conditioned` — peaks removed plus the energy-weighted phase dither on Y, Cb, Cr and G.
///
/// Y, Cb and Cr are transformed concurrently, then the two G passes run concurrently.
fn spectral_stage(p: &Planes, seed: u64) -> (Planes, Planes, u32) {
    let (w, h) = (p.w, p.h);
    let (y, cb, cr) = to_ycbcr(p);
    let (y_out, cb_out, cr_out) = thread::scope(|scope| {
        let hy = scope.spawn(|| spectral_core(&y, w, h, seed ^ 0x59, true, true, true));
        let hcb = scope.spawn(|| spectral_core(&cb, w, h, seed ^ 0xCB, false, true, false));
        let hcr = scope.spawn(|| spectral_core(&cr, w, h, seed ^ 0xC4, false, true, false));
        (hy.join().unwrap(), hcb.join().unwrap(), hcr.join().unwrap())
    });
    let y_pk = y_out.peakless.expect("peakless Y requested");
    let mut peakless = from_ycbcr(p, &y_pk, &cb, &cr);
    let mut conditioned = from_ycbcr(p, &y_out.conditioned, &cb_out.conditioned, &cr_out.conditioned);
    drop((y, cb, cr, y_pk, cb_out, cr_out));
    let (g_pk, g_c) = thread::scope(|scope| {
        let a = scope.spawn(|| spectral_core(&peakless.ch[1], w, h, seed ^ 0x47, true, false, false));
        let b = scope.spawn(|| spectral_core(&conditioned.ch[1], w, h, seed ^ 0x47, true, true, false));
        (a.join().unwrap(), b.join().unwrap())
    });
    let peaks = y_out.peaks + g_pk.peaks;
    peakless.ch[1] = g_pk.conditioned;
    conditioned.ch[1] = g_c.conditioned;
    (peakless, conditioned, peaks)
}

// ---------------------------------------------------------------------------
// Stage D — band-split detail preservation
// ---------------------------------------------------------------------------

pub const DETAIL_SIGMA: f32 = 1.95;

/// Output = O_ref − GaussianBlur(O_ref − R, σ): low/mid frequencies come from the
/// processed image `r`, sharp detail from the reference `oref`. The caller passes the
/// peak-free reference so narrow periodic spikes are never restored.
pub fn detail_preserve(oref: &Planes, r: &Planes) -> Planes {
    let kernel = gaussian_kernel(DETAIL_SIGMA);
    let n = oref.w * oref.h;
    let mut ch = parallel_map(3, |c| {
        let diff: Vec<f32> = (0..n).map(|i| oref.ch[c][i] - r.ch[c][i]).collect();
        let blurred = convolve_separable(&diff, oref.w, oref.h, &kernel);
        (0..n).map(|i| oref.ch[c][i] - blurred[i]).collect::<Vec<f32>>()
    });
    if oref.has_alpha() {
        ch.push(oref.ch[3].clone());
    }
    Planes { w: oref.w, h: oref.h, ch }
}

// ---------------------------------------------------------------------------
// Stage E — DCT grid shift
// ---------------------------------------------------------------------------

pub const GRID_SCALE: f32 = 0.997;
pub const GRID_CROP_LEFT: usize = 1;
pub const GRID_CROP_TOP: usize = 2;

fn grid_target(w: usize, h: usize) -> (usize, usize) {
    let cw = w - GRID_CROP_LEFT;
    let ch = h - GRID_CROP_TOP;
    let nw = ((cw as f32 * GRID_SCALE).round() as usize).max(1);
    let nh = ((ch as f32 * GRID_SCALE).round() as usize).max(1);
    (nw, nh)
}

pub fn grid_shift(p: &Planes) -> Planes {
    let cropped = crop(p, GRID_CROP_LEFT, GRID_CROP_TOP);
    let (nw, nh) = grid_target(p.w, p.h);
    lanczos_resize(&cropped, nw, nh)
}

// ---------------------------------------------------------------------------
// Stage F / Mode 2 — sensor noise and dither
// ---------------------------------------------------------------------------

/// Heteroscedastic Poisson-Gaussian sensor noise:
/// I' = clip(I + N(0, σ_read² + σ_shot² · I/255), 0, 255)
pub fn sensor_noise(p: &mut Planes, rng: &mut StdRng, sigma_read: f32, sigma_shot: f32) {
    let n = p.w * p.h;
    let mut g = Gaussian::new();
    let read2 = sigma_read * sigma_read;
    let shot2 = sigma_shot * sigma_shot;
    for i in 0..n {
        let cover = p.alpha_weight(i);
        if cover <= 0.0 {
            continue;
        }
        let ceil = 255.0 * cover;
        for c in 0..3 {
            let v = p.ch[c][i];
            let sigma = (read2 + shot2 * (v / cover).clamp(0.0, 255.0) / 255.0).sqrt() * cover;
            p.ch[c][i] = (v + g.sample(rng) * sigma).clamp(0.0, ceil);
        }
    }
}

/// Triangular (TPDF) sub-LSB dither, independent per channel, amplitude ±0.5 LSB.
pub fn tpdf_dither(p: &mut Planes, rng: &mut StdRng) {
    let n = p.w * p.h;
    for i in 0..n {
        let cover = p.alpha_weight(i);
        if cover <= 0.0 {
            continue;
        }
        let ceil = 255.0 * cover;
        for c in 0..3 {
            let d = (rng.gen::<f32>() - rng.gen::<f32>()) * 0.5;
            p.ch[c][i] = (p.ch[c][i] + d).clamp(0.0, ceil);
        }
    }
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

pub fn psnr(a: &Planes, b: &Planes) -> f64 {
    let n = a.w * a.h;
    if n == 0 || a.w != b.w || a.h != b.h {
        return 0.0;
    }
    let mut se = 0.0f64;
    for c in 0..3 {
        for i in 0..n {
            let d = (a.ch[c][i] - b.ch[c][i]) as f64;
            se += d * d;
        }
    }
    let mse = se / (3.0 * n as f64);
    if mse <= 1e-10 {
        99.0
    } else {
        (10.0 * (255.0f64 * 255.0 / mse).log10()).min(99.0)
    }
}

/// Gaussian-window SSIM (σ = 1.5, 11×11) on luminance.
pub fn ssim(a: &Planes, b: &Planes) -> f64 {
    if a.w != b.w || a.h != b.h || a.w == 0 {
        return 0.0;
    }
    let (w, h) = (a.w, a.h);
    let n = w * h;
    let x = a.luma();
    let y = b.luma();
    let kernel = gaussian_kernel(1.5);
    let xx: Vec<f32> = x.iter().map(|v| v * v).collect();
    let yy: Vec<f32> = y.iter().map(|v| v * v).collect();
    let xy: Vec<f32> = (0..n).map(|i| x[i] * y[i]).collect();
    let inputs: [&Vec<f32>; 5] = [&x, &y, &xx, &yy, &xy];
    let mut blurred = parallel_map(5, |i| convolve_separable(inputs[i], w, h, &kernel));
    let e_xy = blurred.pop().unwrap();
    let e_yy = blurred.pop().unwrap();
    let e_xx = blurred.pop().unwrap();
    let mu_y = blurred.pop().unwrap();
    let mu_x = blurred.pop().unwrap();
    let c1 = (0.01f64 * 255.0).powi(2);
    let c2 = (0.03f64 * 255.0).powi(2);
    let mut total = 0.0f64;
    for i in 0..n {
        let mx = mu_x[i] as f64;
        let my = mu_y[i] as f64;
        let vx = (e_xx[i] as f64 - mx * mx).max(0.0);
        let vy = (e_yy[i] as f64 - my * my).max(0.0);
        let cov = e_xy[i] as f64 - mx * my;
        let s = ((2.0 * mx * my + c1) * (2.0 * cov + c2)) / ((mx * mx + my * my + c1) * (vx + vy + c2));
        total += s;
    }
    (total / n as f64).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Pipeline entry point
// ---------------------------------------------------------------------------

pub const SENSOR_READ_SIGMA: f32 = 0.35;
pub const SENSOR_SHOT_SIGMA: f32 = 0.45;
pub const STUDIO_READ_SIGMA: f32 = 0.3;
pub const STUDIO_SHOT_SIGMA: f32 = 0.35;
pub const MIN_STUDIO_DIM: usize = 16;

fn base_stats(mode: SanitizeMode, w: usize, h: usize) -> PipelineStats {
    PipelineStats {
        mode,
        input_width: w as u32,
        input_height: h as u32,
        output_width: w as u32,
        output_height: h as u32,
        ssim: 1.0,
        psnr: 99.0,
        ssim_source: 1.0,
        psnr_source: 99.0,
        stages: Vec::new(),
        corner_overlay_detected: false,
        corner_overlay_alpha: 0.0,
        corner_overlay_size: String::new(),
        corner_overlay_shape: String::new(),
        corner_overlay_margin: 0,
        spectral_peaks_attenuated: 0,
        duration_ms: 0,
    }
}

/// Runs the pixel pipeline for the requested mode. `Off` and `Safe` return the input
/// unchanged (their work happens at the container level).
pub fn run_pipeline(img: &RgbaImage, cfg: &SanitizeConfig) -> Result<(RgbaImage, PipelineStats), String> {
    let start = Instant::now();
    let seed = cfg.seed.unwrap_or_else(rand::random::<u64>);
    let mut rng = StdRng::seed_from_u64(seed);
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w == 0 || h == 0 {
        return Err("Image has zero dimensions".to_string());
    }
    let mut stats = base_stats(cfg.mode, w, h);

    // Stage 0 — optional downscale, shared by every mode so that noise, dither and
    // spectral conditioning operate at the final resolution. Never upscales.
    let downscaled;
    let img = match cfg.max_width {
        Some(mw) if mw > 0 && (mw as usize) < w => {
            let t = Instant::now();
            let nw = mw as usize;
            let nh = ((h as f64 * nw as f64 / w as f64).round() as usize).max(1);
            downscaled = lanczos_resize(&Planes::from_rgba(img), nw, nh).to_rgba();
            stats.output_width = nw as u32;
            stats.output_height = nh as u32;
            stats.stages.push(format!("downscale(lanczos, {}×{} → {}×{}) {}ms", w, h, nw, nh, t.elapsed().as_millis()));
            &downscaled
        }
        _ => img,
    };
    let (w, h) = (img.width() as usize, img.height() as usize);

    match cfg.mode {
        SanitizeMode::Off | SanitizeMode::Safe => {
            stats.duration_ms = start.elapsed().as_millis() as u64;
            Ok((img.clone(), stats))
        }
        SanitizeMode::SensorNatural => {
            let source = Planes::from_rgba(img);
            let mut p = source.clone();
            sensor_noise(&mut p, &mut rng, SENSOR_READ_SIGMA, SENSOR_SHOT_SIGMA);
            stats.stages.push(format!("sensor_noise(read={}, shot={})", SENSOR_READ_SIGMA, SENSOR_SHOT_SIGMA));
            tpdf_dither(&mut p, &mut rng);
            stats.stages.push("tpdf_dither(±0.5 LSB)".to_string());
            let out = p.to_rgba();
            let quantised = Planes::from_rgba(&out);
            stats.ssim = ssim(&source, &quantised);
            stats.psnr = psnr(&source, &quantised);
            stats.ssim_source = stats.ssim;
            stats.psnr_source = stats.psnr;
            stats.duration_ms = start.elapsed().as_millis() as u64;
            Ok((out, stats))
        }
        SanitizeMode::StudioMaster => {
            if w < MIN_STUDIO_DIM || h < MIN_STUDIO_DIM {
                return Err(format!(
                    "studio_master needs at least {}×{} px (got {}×{})",
                    MIN_STUDIO_DIM, MIN_STUDIO_DIM, w, h
                ));
            }
            let mut p0 = Planes::from_rgba(img);

            // A — overlay inverse blend
            if cfg.deblend_corner {
                if let Some(ov) = detect_corner_overlay(&p0) {
                    deblend_corner(&mut p0, &ov);
                    stats.corner_overlay_detected = true;
                    stats.corner_overlay_alpha = ov.alpha;
                    stats.corner_overlay_size = format!("{}x{}", ov.width, ov.height);
                    stats.corner_overlay_shape = ov.shape.label().to_string();
                    stats.corner_overlay_margin = ov.margin as u32;
                    stats.stages.push(format!(
                        "deblend_corner({} {}x{}, margin={}px, α={:.3}, ncc={:.2})",
                        ov.shape.label(),
                        ov.width,
                        ov.height,
                        ov.margin,
                        ov.alpha,
                        ov.score
                    ));
                } else {
                    stats.stages.push("deblend_corner(no overlay detected)".to_string());
                }
            }

            // B — elastic resampling
            let t = Instant::now();
            let oref = elastic_resample(&p0, &mut rng);
            stats.stages.push(format!(
                "elastic_resample(σ={}+{}px, max={}px) {}ms",
                ELASTIC_SIGMA_COARSE, ELASTIC_SIGMA_FINE, ELASTIC_MAX_PX, t.elapsed().as_millis()
            ));

            // C — spectral conditioning: peak-free reference + phase-dithered image
            let t = Instant::now();
            let (o_peakless, r, peaks) = spectral_stage(&oref, seed);
            stats.spectral_peaks_attenuated = peaks;
            stats.stages.push(format!(
                "spectral_condition(peaks={}, phase≤{}rad energy-weighted, ring {}–{}) {}ms",
                peaks, PHASE_MAX_RAD, PHASE_BAND.0, PHASE_BAND.1, t.elapsed().as_millis()
            ));

            // D — detail preservation against the peak-free reference
            let t = Instant::now();
            let p2 = detail_preserve(&o_peakless, &r);
            drop((o_peakless, r));
            stats.stages.push(format!("detail_preserve(σ={}, peak-free reference) {}ms", DETAIL_SIGMA, t.elapsed().as_millis()));

            // E — grid shift
            let t = Instant::now();
            let (mut p3, reference, source_aligned) = thread::scope(|scope| {
                let a = scope.spawn(|| grid_shift(&p2));
                let b = scope.spawn(|| grid_shift(&oref));
                let c = scope.spawn(|| grid_shift(&p0));
                (a.join().unwrap(), b.join().unwrap(), c.join().unwrap())
            });
            drop((p2, oref, p0));
            stats.stages.push(format!("grid_shift(crop {}/{}px, scale {}) {}ms", GRID_CROP_LEFT, GRID_CROP_TOP, GRID_SCALE, t.elapsed().as_millis()));

            // F — sensor noise
            let t = Instant::now();
            sensor_noise(&mut p3, &mut rng, STUDIO_READ_SIGMA, STUDIO_SHOT_SIGMA);
            stats.stages.push(format!("sensor_noise(read={}, shot={}) {}ms", STUDIO_READ_SIGMA, STUDIO_SHOT_SIGMA, t.elapsed().as_millis()));

            let t = Instant::now();
            let out = p3.to_rgba();
            let quantised = Planes::from_rgba(&out);
            stats.output_width = out.width();
            stats.output_height = out.height();
            let ((ssim_ref, psnr_ref), (ssim_src, psnr_src)) = thread::scope(|scope| {
                let a = scope.spawn(|| (ssim(&reference, &quantised), psnr(&reference, &quantised)));
                let b = scope.spawn(|| (ssim(&source_aligned, &quantised), psnr(&source_aligned, &quantised)));
                (a.join().unwrap(), b.join().unwrap())
            });
            stats.ssim = ssim_ref;
            stats.psnr = psnr_ref;
            stats.ssim_source = ssim_src;
            stats.psnr_source = psnr_src;
            stats.stages.push(format!("metrics(ssim+psnr ×2) {}ms", t.elapsed().as_millis()));
            stats.duration_ms = start.elapsed().as_millis() as u64;
            Ok((out, stats))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Natural-looking synthetic image: smooth gradients, soft blobs, mild white noise.
    fn natural_rgba(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            let fx = x as f32 / w as f32;
            let fy = y as f32 / h as f32;
            let blob = |cx: f32, cy: f32, s: f32| -> f32 {
                let d = ((fx - cx).powi(2) + (fy - cy).powi(2)) / (s * s);
                (-d).exp()
            };
            let noise = |k: u64| hash_uniform(77, (x as u64 * 7919 + y as u64 * 104_729) ^ k) * 2.0;
            let r = 90.0 + 90.0 * fx + 50.0 * blob(0.3, 0.4, 0.15) + noise(1);
            let g = 80.0 + 70.0 * fy + 60.0 * blob(0.7, 0.6, 0.2) + noise(2);
            let b = 140.0 - 60.0 * fx * fy + 40.0 * blob(0.5, 0.2, 0.1) + noise(3);
            image::Rgba([r.clamp(0.0, 255.0) as u8, g.clamp(0.0, 255.0) as u8, b.clamp(0.0, 255.0) as u8, 255])
        })
    }

    fn cfg(mode: SanitizeMode, deblend: bool) -> SanitizeConfig {
        SanitizeConfig { mode, deblend_corner: deblend, output_format: OutputFormat::Png, seed: Some(1234), ..Default::default() }
    }

    #[test]
    fn sensor_natural_changes_pixels_subtly() {
        let img = natural_rgba(96, 80);
        let (out, stats) = run_pipeline(&img, &cfg(SanitizeMode::SensorNatural, false)).unwrap();
        assert_eq!((out.width(), out.height()), (96, 80));
        let changed = img.pixels().zip(out.pixels()).filter(|(a, b)| a != b).count();
        assert!(changed > 96 * 80 / 10, "noise must touch a meaningful share of pixels ({changed})");
        assert!(stats.psnr > 45.0, "psnr {}", stats.psnr);
        assert!(stats.ssim > 0.99, "ssim {}", stats.ssim);
        let max_diff = img
            .pixels()
            .zip(out.pixels())
            .map(|(a, b)| (0..3).map(|c| (a[c] as i32 - b[c] as i32).abs()).max().unwrap())
            .max()
            .unwrap();
        assert!(max_diff <= 6, "sensor noise must stay sub-perceptual (max diff {max_diff})");
    }

    #[test]
    fn studio_master_keeps_high_fidelity() {
        let img = natural_rgba(256, 192);
        let (out, stats) = run_pipeline(&img, &cfg(SanitizeMode::StudioMaster, false)).unwrap();
        let (nw, nh) = grid_target(256, 192);
        assert_eq!((out.width() as usize, out.height() as usize), (nw, nh));
        assert!(nw < 256 && nh < 192);
        eprintln!(
            "studio_master fidelity: ssim {:.4} psnr {:.2} dB | vs source: ssim {:.4} psnr {:.2} dB | peaks {}",
            stats.ssim, stats.psnr, stats.ssim_source, stats.psnr_source, stats.spectral_peaks_attenuated
        );
        assert!(stats.ssim >= 0.985, "signal ssim {}", stats.ssim);
        assert!(stats.psnr >= 38.0, "signal psnr {}", stats.psnr);
        assert!(stats.psnr_source > 35.0, "psnr vs source {}", stats.psnr_source);
        assert!(stats.ssim_source > 0.95, "ssim vs source {}", stats.ssim_source);
        assert_eq!(stats.stages.len(), 6, "{:?}", stats.stages);
        assert!(stats.duration_ms < 60_000);
    }

    #[test]
    fn max_width_downscales_before_dsp_and_never_upscales() {
        let img = natural_rgba(200, 100);
        let mut c = cfg(SanitizeMode::Safe, false);
        c.max_width = Some(50);
        let (out, stats) = run_pipeline(&img, &c).unwrap();
        assert_eq!((out.width(), out.height()), (50, 25));
        assert_eq!((stats.output_width, stats.output_height), (50, 25));
        assert_eq!((stats.input_width, stats.input_height), (200, 100));
        assert!(stats.stages[0].starts_with("downscale(lanczos, 200×100 → 50×25)"), "{:?}", stats.stages);
        // The channel means must survive the Lanczos downscale (no aliasing, no drift).
        let mean = |p: &RgbaImage, c: usize| p.pixels().map(|px| px[c] as f64).sum::<f64>() / (p.width() * p.height()) as f64;
        for c in 0..3 {
            assert!((mean(&img, c) - mean(&out, c)).abs() < 1.0, "channel {} mean drift", c);
        }

        c.max_width = Some(400);
        let (same, stats) = run_pipeline(&img, &c).unwrap();
        assert_eq!(same.as_raw(), img.as_raw(), "images at or below max_width are untouched");
        assert!(stats.stages.is_empty());

        let mut s = cfg(SanitizeMode::StudioMaster, false);
        s.max_width = Some(64);
        let (out, stats) = run_pipeline(&natural_rgba(256, 192), &s).unwrap();
        let (nw, nh) = grid_target(64, 48);
        assert_eq!((out.width() as usize, out.height() as usize), (nw, nh), "DSP stages ran on the downscaled grid");
        assert_eq!(stats.stages.len(), 7, "{:?}", stats.stages);
        assert!(stats.stages[0].starts_with("downscale"));
    }

    #[test]
    fn studio_master_is_deterministic_with_seed() {
        let img = natural_rgba(64, 48);
        let (a, _) = run_pipeline(&img, &cfg(SanitizeMode::StudioMaster, false)).unwrap();
        let (b, _) = run_pipeline(&img, &cfg(SanitizeMode::StudioMaster, false)).unwrap();
        assert_eq!(a.as_raw(), b.as_raw());
    }

    #[test]
    fn spectral_stage_attenuates_periodic_grid_peaks() {
        let w = 128;
        let h = 128;
        // Smooth base + strong 8-px periodic grid (upsampling artefact).
        let mut plane: Vec<f32> = grid_plane(w, h);
        let before = plane.clone();
        let peaks;
        (plane, peaks) = spectral_condition(&plane, w, h, 99, true, true);
        assert!(peaks > 0, "grid harmonics must be detected as spectral peaks");
        // Grid energy (difference between grid lines and neighbours) must shrink.
        let grid_contrast = |p: &[f32]| -> f32 {
            let mut acc = 0.0;
            let mut n = 0;
            for y in 8..h - 8 {
                for x in 8..w - 8 {
                    if x % 8 == 0 && y % 8 != 0 {
                        acc += (p[y * w + x] - p[y * w + x + 3]).abs();
                        n += 1;
                    }
                }
            }
            acc / n as f32
        };
        let c0 = grid_contrast(&before);
        let c1 = grid_contrast(&plane);
        assert!(c1 < c0 * 0.6, "grid contrast {c0} -> {c1}");
    }

    /// Smooth base + strong 8-px periodic grid (upsampling artefact).
    fn grid_plane(w: usize, h: usize) -> Vec<f32> {
        (0..w * h)
            .map(|i| {
                let x = (i % w) as f32;
                let y = (i / w) as f32;
                120.0 + 30.0 * (x / 40.0).sin() + 20.0 * (y / 50.0).cos()
                    + if (i % w) % 8 == 0 || (i / w) % 8 == 0 { 25.0 } else { 0.0 }
            })
            .collect()
    }

    /// Period-8 folded column profile contrast of the luminance (phase independent).
    fn folded_grid_contrast(p: &Planes) -> f32 {
        let luma = p.luma();
        let mut sums = [0f64; 8];
        let mut counts = [0u32; 8];
        for y in 0..p.h {
            for x in 0..p.w {
                sums[x % 8] += luma[y * p.w + x] as f64;
                counts[x % 8] += 1;
            }
        }
        let prof: Vec<f64> = (0..8).map(|j| sums[j] / counts[j].max(1) as f64).collect();
        (prof.iter().cloned().fold(f64::MIN, f64::max) - prof.iter().cloned().fold(f64::MAX, f64::min)) as f32
    }

    #[test]
    fn studio_master_suppresses_periodic_grid_end_to_end() {
        let (w, h) = (256usize, 192usize);
        let grid = grid_plane(w, h);
        let img = RgbaImage::from_fn(w as u32, h as u32, |x, y| {
            let v = grid[y as usize * w + x as usize].clamp(0.0, 255.0) as u8;
            image::Rgba([v, v.saturating_sub(10), v.saturating_sub(20), 255])
        });
        let input = Planes::from_rgba(&img);
        let (out, stats) = run_pipeline(&img, &cfg(SanitizeMode::StudioMaster, false)).unwrap();
        let output = Planes::from_rgba(&out);
        let c_in = folded_grid_contrast(&input);
        let c_out = folded_grid_contrast(&output);
        eprintln!("grid contrast end-to-end: {c_in:.2} -> {c_out:.2} (peaks {})", stats.spectral_peaks_attenuated);
        assert!(stats.spectral_peaks_attenuated > 0);
        assert!(c_out < c_in * 0.5, "grid contrast must drop in the final output ({c_in} -> {c_out})");
    }

    #[test]
    fn phase_dither_is_energy_weighted() {
        let (w, h) = (128usize, 128usize);
        // Strong carrier at kx = 16 (r = 0.25, inside the dither ring) + weak white noise.
        let plane: Vec<f32> = (0..w * h)
            .map(|i| {
                let x = (i % w) as f32;
                128.0 + 60.0 * (2.0 * PI * 16.0 * x / w as f32).cos() + hash_uniform(5, i as u64) * 2.0
            })
            .collect();
        let (out, _) = spectral_condition(&plane, w, h, 11, false, true);
        let spectrum = |p: &[f32]| {
            let mut s: Vec<Complex<f32>> = p.iter().map(|&v| Complex::new(v, 0.0)).collect();
            fft2d(&mut s, w, h, false);
            s
        };
        let fa = spectrum(&plane);
        let fb = spectrum(&out);
        let carrier = 16usize;
        let dphi_carrier = (fb[carrier] * fa[carrier].conj()).arg().abs();
        assert!(dphi_carrier < 0.02, "strong carrier phase must be kept (Δφ={dphi_carrier})");
        let mut ring_mags: Vec<f32> = Vec::new();
        let mut ring_bins: Vec<usize> = Vec::new();
        for ky in 0..h {
            for kx in 0..w {
                let r = norm_radius(ky, kx, w, h);
                if r > PHASE_BAND.0 && r < PHASE_BAND.1 {
                    ring_mags.push(fa[ky * w + kx].norm());
                    ring_bins.push(ky * w + kx);
                }
            }
        }
        let mid = ring_mags.len() / 2;
        let med = *ring_mags.clone().select_nth_unstable_by(mid, |a, b| a.partial_cmp(b).unwrap()).1;
        let weak: Vec<f32> = ring_bins
            .iter()
            .filter(|&&i| fa[i].norm() <= med && fa[i].norm() > 1e-3)
            .map(|&i| (fb[i] * fa[i].conj()).arg().abs())
            .collect();
        let mean_weak = weak.iter().sum::<f32>() / weak.len() as f32;
        eprintln!("phase dither: carrier |Δφ| {dphi_carrier:.4} rad, weak-bin mean |Δφ| {mean_weak:.3} rad");
        assert!(
            (0.15..=0.45).contains(&mean_weak),
            "weak bins must be perturbed by ≈0.35–0.45 rad peak (mean |Δφ| = {mean_weak})"
        );
        let a = Planes { w, h, ch: vec![plane.clone(), plane.clone(), plane.clone()] };
        let b = Planes { w, h, ch: vec![out.clone(), out.clone(), out.clone()] };
        assert!(psnr(&a, &b) > 40.0, "texture carrier must survive: psnr {}", psnr(&a, &b));
    }

    fn blend_overlay(img: &mut RgbaImage, ov: &CornerOverlay) {
        let tmpl = overlay_template(ov.shape, ov.width, ov.height);
        for ty in 0..ov.height {
            for tx in 0..ov.width {
                let a = ov.alpha * tmpl[ty * ov.width + tx];
                let p = img.get_pixel_mut((ov.x0 + tx) as u32, (ov.y0 + ty) as u32);
                for c in 0..3 {
                    p[c] = ((1.0 - a) * p[c] as f32 + a * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }

    #[test]
    fn corner_box_overlay_is_detected_and_deblended() {
        let (w, h) = (160u32, 120u32);
        let clean = natural_rgba(w, h);
        let mut overlaid = clean.clone();
        // Hard-edged 48×48 box flush with the corner, α = 0.45.
        let alpha = 0.45f32;
        for y in (h - 48)..h {
            for x in (w - 48)..w {
                let p = overlaid.get_pixel_mut(x, y);
                for c in 0..3 {
                    p[c] = ((1.0 - alpha) * p[c] as f32 + alpha * 255.0).round() as u8;
                }
            }
        }
        let mut planes = Planes::from_rgba(&overlaid);
        let ov = detect_corner_overlay(&planes).expect("box overlay must be detected");
        eprintln!("box overlay: {:?}", ov);
        assert_eq!(ov.shape, OverlayShape::Box);
        assert_eq!((ov.width, ov.height, ov.margin), (48, 48, 0));
        assert!((ov.alpha - alpha).abs() < 0.08, "alpha estimate {}", ov.alpha);
        assert!(ov.score > 0.8, "ncc {}", ov.score);
        let before = psnr(&Planes::from_rgba(&clean), &planes);
        deblend_corner(&mut planes, &ov);
        let after = psnr(&Planes::from_rgba(&clean), &planes);
        assert!(after > before + 6.0, "deblend must restore the corner ({before} -> {after})");
        let fp = detect_corner_overlay(&Planes::from_rgba(&clean));
        assert!(fp.is_none(), "clean gradient image must not trigger: {:?}", fp);
    }

    #[test]
    fn corner_star_overlay_with_margin_is_detected_and_deblended() {
        let (w, h) = (200u32, 160u32);
        let clean = natural_rgba(w, h);
        let mut overlaid = clean.clone();
        let truth = CornerOverlay {
            shape: OverlayShape::Star,
            x0: 200 - 24 - 48,
            y0: 160 - 24 - 48,
            width: 48,
            height: 48,
            margin: 24,
            alpha: 0.5,
            score: 1.0,
        };
        blend_overlay(&mut overlaid, &truth);
        let mut planes = Planes::from_rgba(&overlaid);
        let ov = detect_corner_overlay(&planes).expect("star overlay must be detected");
        eprintln!("star overlay: {:?}", ov);
        assert_eq!(ov.shape, OverlayShape::Star, "shape {:?} ncc {}", ov.shape, ov.score);
        assert_eq!((ov.width, ov.height), (48, 48));
        assert!((ov.x0 as i32 - truth.x0 as i32).abs() <= 4 && (ov.y0 as i32 - truth.y0 as i32).abs() <= 4, "placement {},{} margin {}", ov.x0, ov.y0, ov.margin);
        assert!((ov.alpha - truth.alpha).abs() < 0.1, "alpha estimate {}", ov.alpha);
        let before = psnr(&Planes::from_rgba(&clean), &planes);
        deblend_corner(&mut planes, &ov);
        let after = psnr(&Planes::from_rgba(&clean), &planes);
        assert!(after > before + 6.0, "deblend must restore the corner ({before} -> {after})");
    }

    #[test]
    fn astroid_template_is_concave_and_centre_bright() {
        let t = overlay_template(OverlayShape::Star, 48, 48);
        let at = |x: usize, y: usize| t[y * 48 + x];
        assert!(at(24, 24) > 0.85, "centre opaque (half-pixel offset from the exact centre)");
        assert_eq!(at(12, 12), 0.0, "diagonal mid-point lies outside the concave astroid");
        assert!(at(24, 4) > 0.0 && at(4, 24) > 0.0, "tips reach toward the bbox edges");
        assert!(at(24, 4) < at(24, 12) && at(24, 12) < at(24, 24), "opacity rises toward the centre");
        let b = overlay_template(OverlayShape::Box, 36, 36);
        assert_eq!(b[0], 0.5);
        assert_eq!(b[2 * 36 + 2], 1.0);
    }

    #[test]
    fn alpha_images_keep_transparent_pixels_transparent() {
        let img = RgbaImage::from_fn(64, 64, |x, y| {
            if x < 20 { image::Rgba([0, 0, 0, 0]) } else { image::Rgba([200, 100 + (y as u8 % 50), 50, 255]) }
        });
        let (out, _) = run_pipeline(&img, &cfg(SanitizeMode::StudioMaster, false)).unwrap();
        let fully_transparent = out.pixels().filter(|p| p[3] == 0).count();
        assert!(fully_transparent > 64 * 10, "transparent region preserved ({fully_transparent})");
        for p in out.pixels().filter(|p| p[3] == 0) {
            assert_eq!(p[0] as u32 + p[1] as u32 + p[2] as u32, 0);
        }
    }

    #[test]
    fn metrics_are_sane() {
        let a = Planes::from_rgba(&natural_rgba(32, 32));
        assert!((ssim(&a, &a) - 1.0).abs() < 1e-6);
        assert_eq!(psnr(&a, &a), 99.0);
        let mut b = a.clone();
        for v in &mut b.ch[0] {
            *v = (*v + 20.0).min(255.0);
        }
        assert!(ssim(&a, &b) < 1.0);
        assert!(psnr(&a, &b) < 30.0);
    }
}
