//! Converts the DX7's 49.1 kHz output to the host rate. The analog path
//! between the DAC and the output jack is linear, so its whole impulse
//! response - the sample-and-hold and the reconstruction filter - is built
//! into the interpolation kernel.
//!
//! The filter's poles sit at 11.6 and 16 kHz, close to the host Nyquist: a
//! bilinear-transform IIR at 44.1/48 kHz gets that region badly wrong
//! (about -7 dB at 15 kHz instead of +0.8 dB). Instead, the kernel is the
//! inverse transform of the circuit's complex transfer function:
//!
//!   D(f) = H_analog(f) * hold(f) * anti-image taper
//!
//! evaluated on a fine grid of fractional delays. Magnitude and phase are
//! those of the analog circuit at every host rate: like the hardware, a
//! transient is followed by the filter's 16 kHz ringing, never preceded
//! by it (a linear-phase kernel would ring on both sides).

/// Taps before the kernel's origin (the taper's symmetric part) ...
const PRE: usize = 32;
/// ... and after it, where the filter rings out (-100 dB after ~46 samples).
const POST: usize = 64;
const TAPS: usize = PRE + POST;
const PHASES: usize = 256;
const HIST: usize = 128;

/// The DAC's sample-and-hold takes each of the 16 voice slots in turn, two
/// hold capacitors alternating: a voice stays at the filter input for two
/// slots, an eighth of a sample. (Not a full-sample hold: the droop is
/// 0.02 dB at 15 kHz, not 1.4 dB.)
pub const HOLD: f64 = 2.0 / 16.0;

pub struct Resampler {
    /// Two kernels (S&H only, S&H + analog filter), each `PHASES + 1` rows
    /// of `TAPS` coefficients (extra row for interpolation), oldest sample
    /// first.
    kernels: [Vec<[f32; TAPS]>; 2],
    /// Use the kernel with the analog filter embedded.
    pub filter: bool,
    /// Input history, written twice so any `TAPS` run is contiguous.
    hist: [f32; 2 * HIST],
    w: usize,
    /// Core samples consumed per host sample.
    ratio: f64,
    frac: f64,
}

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let y = x * x / 4.0;
    for k in 1..60 {
        term *= y / (k as f64 * k as f64);
        sum += term;
        if term < 1e-14 * sum {
            break;
        }
    }
    sum
}

fn kaiser(t: f64, beta: f64) -> f64 {
    if t.abs() >= 1.0 { 0.0 } else { bessel_i0(beta * (1.0 - t * t).sqrt()) / bessel_i0(beta) }
}

/// Complex transfer function (re, im) at `f` Hz.
pub type Response = fn(f64) -> (f64, f64);

impl Resampler {
    /// Builds both kernels up front (tens of ms): call off the audio thread.
    /// `analog`: transfer function embedded in the filtered kernel.
    pub fn new(core_sr: f64, host_sr: f64, analog: Response) -> Self {
        let ratio = core_sr / host_sr;
        let kernels = [build(core_sr, host_sr, None), build(core_sr, host_sr, Some(analog))];
        Self { kernels, filter: true, hist: [0.0; 2 * HIST], w: 0, ratio, frac: 0.0 }
    }
}

fn build(core_sr: f64, host_sr: f64, analog: Option<Response>) -> Vec<[f32; TAPS]> {
    use std::f64::consts::PI;
    // Anti-image taper: flat to `pass`, raised cosine to zero at `stop`
    // (both capped by the core and host Nyquist).
    let nyq = 0.5 * core_sr.min(host_sr);
    let stop = nyq.min(24_000.0);
    let pass = (stop * 0.9).min(20_500.0);
    // Frequency grid for the inverse transform: (cycles per core sample,
    // re, im), already weighted by the grid step.
    const NF: usize = 768;
    let df = stop / NF as f64;
    let spectrum: Vec<(f64, f64, f64)> = (0..NF)
        .map(|i| {
            let f = (i as f64 + 0.5) * df;
            let mut d = df / core_sr;
            if f > pass {
                d *= 0.5 * (1.0 + (PI * (f - pass) / (stop - pass)).cos());
            }
            let x = PI * f * HOLD / core_sr;
            d *= x.sin() / x;
            let (re, im) = analog.map_or((1.0, 0.0), |h| h(f));
            (f / core_sr, d * re, d * im)
        })
        .collect();
    // Impulse response `x` core samples after the impulse. The taper's
    // part is symmetric and windowed to `PRE` on both sides; the filter's
    // tail gets a window that is flat while it rings.
    let beta = 7.0;
    let window = |x: f64| -> f64 {
        if x < 0.0 {
            kaiser(x / PRE as f64, beta)
        } else if analog.is_none() {
            kaiser(x / PRE as f64, beta)
        } else {
            let flat = 20.0;
            if x <= flat { 1.0 } else { kaiser((x - flat) / (POST as f64 - flat), beta) }
        }
    };
    let h = |x: f64| -> f64 {
        let w = window(x);
        if w == 0.0 {
            return 0.0;
        }
        let mut acc = 0.0;
        for &(fn_, re, im) in &spectrum {
            let (s, c) = (2.0 * PI * fn_ * x).sin_cos();
            acc += re * c - im * s;
        }
        2.0 * acc * w
    };
    let mut kernel = vec![[0f32; TAPS]; PHASES + 1];
    for (ph, row) in kernel.iter_mut().enumerate() {
        let mu = ph as f64 / PHASES as f64;
        let mut tmp = [0f64; TAPS];
        let mut sum = 0.0;
        for (k, c) in tmp.iter_mut().enumerate() {
            // Tap k reads sample (newest - k).
            *c = h(k as f64 - (PRE as f64 - 1.0) - mu);
            sum += *c;
        }
        // Unity DC gain at every phase; stored oldest sample first.
        for (c, t) in row.iter_mut().rev().zip(tmp) {
            *c = (t / sum) as f32;
        }
    }
    kernel
}

impl Resampler {
    /// Forget buffered input (after all voices were cut).
    pub fn clear(&mut self) {
        self.hist = [0.0; 2 * HIST];
    }

    /// Produce one host-rate sample, pulling core-rate samples from `pull`.
    #[inline]
    pub fn next(&mut self, mut pull: impl FnMut() -> f32) -> f32 {
        self.frac += self.ratio;
        while self.frac >= 1.0 {
            self.frac -= 1.0;
            self.w = (self.w + 1) & (HIST - 1);
            let x = pull();
            self.hist[self.w] = x;
            self.hist[self.w + HIST] = x;
        }
        // Read point lies `1 - frac` samples before the newest one.
        let pos = (1.0 - self.frac) * PHASES as f64;
        let ip = (pos as usize).min(PHASES - 1);
        let t = (pos - ip as f64) as f32;
        let kernel = &self.kernels[usize::from(self.filter)];
        let a = &kernel[ip];
        let b = &kernel[ip + 1];
        // The newest TAPS samples, oldest first.
        let start = self.w + HIST + 1 - TAPS;
        let x: &[f32; TAPS] = self.hist[start..start + TAPS].try_into().unwrap();
        // Eight running sums: lets the compiler use SIMD.
        let mut sa = [0.0f32; 8];
        let mut sb = [0.0f32; 8];
        for ((xc, ac), bc) in x.chunks_exact(8).zip(a.chunks_exact(8)).zip(b.chunks_exact(8)) {
            for i in 0..8 {
                sa[i] += xc[i] * ac[i];
                sb[i] += xc[i] * bc[i];
            }
        }
        let ya: f32 = sa.iter().sum();
        let yb: f32 = sb.iter().sum();
        ya + (yb - ya) * t
    }
}
