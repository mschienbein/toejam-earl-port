//! Based on this windowed sinc interpolation algorithm by Julius O. Smith III:
//!   <https://ccrma.stanford.edu/~jos/resample/resample.html>

mod performance;
mod quality;

use bincode::{Decode, Encode};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::{array, iter};

const LINEAR_INTERPOLATION_BITS: u32 = 20;

pub trait SincKernel {
    fn fir() -> &'static [f32];

    fn oversample_factor() -> u32;
}

#[derive(Debug, Clone, Copy, Default, Encode, Decode)]
pub struct Quality;

impl SincKernel for Quality {
    fn fir() -> &'static [f32] {
        quality::SINC_KERNEL
    }

    fn oversample_factor() -> u32 {
        quality::SINC_OVERSAMPLE_FACTOR
    }
}

#[derive(Debug, Clone, Copy, Default, Encode, Decode)]
pub struct Performance;

impl SincKernel for Performance {
    fn fir() -> &'static [f32] {
        performance::SINC_KERNEL
    }

    fn oversample_factor() -> u32 {
        performance::SINC_OVERSAMPLE_FACTOR
    }
}

#[derive(Debug, Clone, Encode, Decode)]
struct SampleRingBuffer {
    buffer: Vec<f64>,
    idx: usize,
    len: usize,
}

impl SampleRingBuffer {
    // For an N-sample buffer, allows roughly 64*N^2 samples before needing to copy back to the
    // beginning
    const CAPACITY_MULTIPLIER: usize = 64;

    fn new(required_samples: usize) -> Self {
        assert_ne!(required_samples, 0);

        let buffer_len = Self::CAPACITY_MULTIPLIER * required_samples;

        Self { buffer: vec![0.0; buffer_len], idx: 0, len: 0 }
    }

    fn ensure_capacity(&mut self, required_samples: usize) {
        let buffer_len = Self::CAPACITY_MULTIPLIER * required_samples;
        let additional = buffer_len.saturating_sub(self.buffer.len());
        if additional != 0 {
            self.buffer.extend(iter::repeat_n(0.0, additional));
        }
    }

    fn push(&mut self, sample: f64) {
        if self.idx + self.len == self.buffer.len() {
            // TJE native: scalar taps need no SIMD overread padding.
            self.buffer.copy_within(self.idx..self.idx + self.len, 0);
            self.idx = 0;
        }

        self.buffer[self.idx + self.len] = sample;
        self.len += 1;
    }

    fn pop(&mut self) {
        if self.len != 0 {
            self.idx += 1;
            self.len -= 1;
        }
    }

    #[cfg(test)]
    fn as_slice(&self) -> &[f64] {
        &self.buffer[self.idx..self.idx + self.len]
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct SincResampler<const CHANNELS: usize, Kernel: SincKernel> {
    input_counter: f64,
    source_rate: f64,
    target_rate: f64,
    ratio: f64,
    required_samples: usize,
    input: [SampleRingBuffer; CHANNELS],
    output: VecDeque<[f64; CHANNELS]>,
    // Required for this to compile with the generic Kernel type
    _marker: PhantomData<Kernel>,
}

impl<const CHANNELS: usize, Kernel: SincKernel> SincResampler<CHANNELS, Kernel> {
    #[must_use]
    pub fn new(source_rate: f64, target_rate: f64) -> Self {
        let ratio = target_rate / source_rate;
        let required_samples = estimate_required_samples::<Kernel>(ratio);

        let resampler = Self {
            input_counter: 0.0,
            source_rate,
            target_rate,
            ratio,
            required_samples,
            input: array::from_fn(|_| SampleRingBuffer::new(required_samples)),
            output: VecDeque::with_capacity(48000 / 30),
            _marker: PhantomData,
        };

        resampler.log_debug_output();

        resampler
    }

    #[inline]
    pub fn collect(&mut self, samples: [f64; CHANNELS]) {
        for (sample, input) in iter::zip(samples, &mut self.input) {
            input.push(sample);
        }

        while self.input[0].len >= self.required_samples {
            self.generate_output_sample();

            self.input_counter += 1.0 / self.ratio;
            while self.input_counter >= 1.0 {
                self.input_counter -= 1.0;
                for input in &mut self.input {
                    input.pop();
                }
            }
        }
    }

    fn generate_output_sample(&mut self) {
        fn interpolation_idx_float_to_fixed_point(float: f64, oversample_factor: u32) -> u64 {
            (float * f64::from(1 << LINEAR_INTERPOLATION_BITS) * f64::from(oversample_factor))
                .round() as u64
        }

        let fir = Kernel::fir();
        let oversample_factor = Kernel::oversample_factor();

        let n = self.required_samples / 2;

        // Steps are smaller when downsampling (ratio < 1.0) to lower the low-pass filter's cutoff
        let scale = if self.ratio < 1.0 { self.ratio } else { 1.0 };

        let step = {
            let step_float = scale * f64::from(oversample_factor);
            (step_float * f64::from(1 << LINEAR_INTERPOLATION_BITS)).round() as u64
        };

        let input_slices: [_; CHANNELS] = array::from_fn(|ch| self.input[ch].buffer.as_slice());

        // Sum the left wing of the input window / right wing of the windowed sinc
        let interpolation_idx = {
            let idx_float = scale * self.input_counter.fract();
            interpolation_idx_float_to_fixed_point(idx_float, oversample_factor)
        };
        let l_sum =
            sum_wing::<true, _>(fir, interpolation_idx, step, input_slices, n + self.input[0].idx);

        // Sum the right wing of the input window / left wing of the windowed sinc
        let interpolation_idx = {
            let idx_float = scale * (1.0 - self.input_counter.fract());
            interpolation_idx_float_to_fixed_point(idx_float, oversample_factor)
        };
        let r_sum =
            sum_wing::<false, _>(fir, interpolation_idx, step, input_slices, n + self.input[0].idx);

        let volume = scale * f64::from(oversample_factor);
        self.output.push_back(array::from_fn(|ch| volume * (l_sum[ch] + r_sum[ch])));
    }

    #[inline]
    #[must_use]
    pub fn output_buffer_len(&self) -> usize {
        self.output.len()
    }

    #[inline]
    #[must_use]
    pub fn output_buffer_pop_front(&mut self) -> Option<[f64; CHANNELS]> {
        self.output.pop_front()
    }

    pub fn update_source_frequency(&mut self, source_frequency: f64) {
        self.source_rate = source_frequency;
        self.handle_frequency_update();
    }

    pub fn update_output_frequency(&mut self, output_frequency: f64) {
        self.target_rate = output_frequency;
        self.handle_frequency_update();
    }

    fn handle_frequency_update(&mut self) {
        // TODO adjust input counter?
        self.ratio = self.target_rate / self.source_rate;
        self.required_samples = estimate_required_samples::<Kernel>(self.ratio);

        for input in &mut self.input {
            input.ensure_capacity(self.required_samples);
        }

        self.log_debug_output();
    }

    fn log_debug_output(&self) {
        if !log::log_enabled!(log::Level::Debug) {
            return;
        }

        log::debug!("Source frequency: {}", self.source_rate);
        log::debug!("Target frequency: {}", self.target_rate);
        log::debug!("Ratio: {}", self.ratio);
        log::debug!("FIR half-length: {}", Kernel::fir().len());
        log::debug!("Oversampling factor: {}", Kernel::oversample_factor());
        log::debug!("Required input samples: {}", self.required_samples);
    }
}

fn estimate_required_samples<Kernel: SincKernel>(ratio: f64) -> usize {
    let fir = Kernel::fir();
    let oversample_factor = Kernel::oversample_factor();

    let mut step: f64 = oversample_factor.into();
    if ratio < 1.0 {
        step *= ratio;
    }

    let required_half = (fir.len() as f64 / step).ceil() as usize;
    2 * required_half + 1
}

// TJE native (ER-001): all targets use the same scalar tap order and separate
// multiply/add operations. CPU-dispatched FMA and lane reductions change sample
// bits, so they are not alternate implementations of this deterministic policy.
fn sum_wing<const REVERSE: bool, const CHANNELS: usize>(
    fir: &[f32],
    mut interpolation_idx: u64,
    step: u64,
    input: [&[f64]; CHANNELS],
    n: usize,
) -> [f64; CHANNELS] {
    let mut sum = [0.0; CHANNELS];
    for i in 0.. {
        let fir_idx = (interpolation_idx >> LINEAR_INTERPOLATION_BITS) as usize;

        // Check len-1 because last entry is always 0
        if fir_idx >= fir.len() - 1 {
            break;
        }

        // Apply linear interpolation
        // Coefficient diffs are not cached as the algorithm describes because RAM speed is likely
        // going to be the bottleneck here, not calculation throughput
        let linear_factor = (interpolation_idx & ((1 << LINEAR_INTERPOLATION_BITS) - 1)) as f64
            / f64::from(1 << LINEAR_INTERPOLATION_BITS);

        let coefficient: f64 = fir[fir_idx].into();
        let next_coeff: f64 = fir[fir_idx + 1].into();
        let multiplier = coefficient + linear_factor * (next_coeff - coefficient);

        let input_idx = if REVERSE { n - i } else { n + i + 1 };

        for ch in 0..CHANNELS {
            let in_sample = input[ch][input_idx];
            sum[ch] += multiplier * in_sample;
        }

        interpolation_idx += step;
    }

    sum
}

pub type QualitySincResampler<const CHANNELS: usize> = SincResampler<CHANNELS, Quality>;
pub type PerformanceSincResampler<const CHANNELS: usize> = SincResampler<CHANNELS, Performance>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_basic() {
        let mut buffer = SampleRingBuffer::new(10);

        buffer.push(1.0);
        buffer.push(2.0);
        buffer.push(3.0);
        buffer.push(4.0);
        buffer.push(5.0);

        assert_eq!(buffer.as_slice(), &[1.0, 2.0, 3.0, 4.0, 5.0]);

        buffer.pop();
        assert_eq!(buffer.as_slice(), &[2.0, 3.0, 4.0, 5.0]);
        buffer.pop();
        assert_eq!(buffer.as_slice(), &[3.0, 4.0, 5.0]);
        buffer.pop();
        assert_eq!(buffer.as_slice(), &[4.0, 5.0]);
        buffer.pop();
        assert_eq!(buffer.as_slice(), &[5.0]);
        buffer.pop();
        assert_eq!(buffer.as_slice(), &[]);
        buffer.pop();
        assert_eq!(buffer.as_slice(), &[]);
    }

    #[test]
    fn ring_buffer_copy() {
        let mut buffer = SampleRingBuffer::new(10);

        buffer.buffer.fill_with(|| rand::random_range(-1.0..=1.0));

        let len = 15;
        let end_start = buffer.buffer.len() - len;

        buffer.idx = end_start;
        buffer.len = len;

        // Should trigger a copy from end to start
        buffer.push(1.0);

        // Validate that the index moved to start
        assert_eq!(buffer.idx, 0);
        assert_eq!(buffer.len, len + 1);

        // Validate that samples were copied
        let mut expected = buffer.buffer[end_start..end_start + len].to_vec();
        expected.push(1.0);
        assert_eq!(buffer.as_slice(), expected);
    }

    #[test]
    fn scalar_taps_preserve_sequential_reduction_order() {
        let fir = [1.0_f32, 1.0, 1.0, 0.0];
        let forward = [0.0, 1e16, 1.0, -1e16];
        let reverse = [-1e16, 1.0, 1e16];
        let step = 1 << LINEAR_INTERPOLATION_BITS;
        assert_eq!(sum_wing::<false, 1>(&fir, 0, step, [&forward], 0), [0.0]);
        assert_eq!(sum_wing::<true, 1>(&fir, 0, step, [&reverse], 2), [0.0]);
    }

    #[test]
    fn scalar_interpolation_does_not_fuse_multiply_and_add() {
        let delta = 1.0 / f64::from(1 << LINEAR_INTERPOLATION_BITS);
        let next = 1.0 / (1_u64 << 40) as f64;
        let fir = [1.0_f32, next as f32, 0.0];
        let input = [0.0, 1.0];
        let index = (1 << LINEAR_INTERPOLATION_BITS) - 1;
        let step = 3 << LINEAR_INTERPOLATION_BITS;
        let result = sum_wing::<false, 1>(&fir, index, step, [&input], 0)[0];
        assert_eq!(result.to_bits(), (delta + next).to_bits());
    }

    #[test]
    fn scalar_accumulation_does_not_fuse_multiply_and_add() {
        let coefficient = 1.0 + f32::EPSILON;
        let fir = [coefficient, coefficient, 0.0];
        let input = [0.0, -1.0, 1.0 - f64::EPSILON];
        let step = 1 << LINEAR_INTERPOLATION_BITS;
        let result = sum_wing::<false, 1>(&fir, 0, step, [&input], 0)[0];
        assert_eq!(result.to_bits(), (-f64::EPSILON).to_bits());
    }
}
