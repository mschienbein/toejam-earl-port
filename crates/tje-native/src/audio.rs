use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_queue::ArrayQueue;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};

#[derive(Default)]
struct Counters {
    callbacks: AtomicU64,
    frames: AtomicU64,
    nonzero_frames: AtomicU64,
    underrun_frames: AtomicU64,
    overflows: AtomicU64,
    peak_bits: AtomicU32,
    failed: AtomicBool,
}

#[derive(Debug)]
pub struct AudioStats {
    pub callbacks: u64,
    pub frames: u64,
    pub nonzero_frames: u64,
    pub underrun_frames: u64,
    pub overflows: u64,
    pub peak: f32,
    pub queued: usize,
}

pub struct Audio {
    _stream: cpal::Stream,
    queue: Arc<ArrayQueue<[f32; 2]>>,
    counters: Arc<Counters>,
    volume: Arc<AtomicU32>,
    paused: Arc<AtomicBool>,
    pub sample_rate: u32,
}

impl Audio {
    pub fn new(volume: f32) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("no audio output device")?;
        let supported = device
            .default_output_config()
            .context("default audio output format")?;
        let mut config: cpal::StreamConfig = supported.clone().into();
        // Request a short callback where supported. WASAPI uses its default
        // shared-mode buffer; ALSA's supported range is clamped to 256 frames.
        if let cpal::SupportedBufferSize::Range { min, max } = *supported.buffer_size() {
            config.buffer_size = cpal::BufferSize::Fixed(256_u32.clamp(min, max));
        }
        let sample_rate = config.sample_rate.0;
        let queue = Arc::new(ArrayQueue::new(sample_rate as usize / 8));
        let counters = Arc::new(Counters::default());
        let volume = Arc::new(AtomicU32::new(volume.to_bits()));
        let paused = Arc::new(AtomicBool::new(false));
        let output = Output {
            queue: queue.clone(),
            counters: counters.clone(),
            volume: volume.clone(),
            paused: paused.clone(),
            prebuffer: sample_rate as usize / 25,
            ready: false,
            fade: 0.0,
            last: [0.0; 2],
        };
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, output),
            cpal::SampleFormat::F64 => build::<f64>(&device, &config, output),
            cpal::SampleFormat::I8 => build::<i8>(&device, &config, output),
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, output),
            cpal::SampleFormat::I32 => build::<i32>(&device, &config, output),
            cpal::SampleFormat::I64 => build::<i64>(&device, &config, output),
            cpal::SampleFormat::U8 => build::<u8>(&device, &config, output),
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, output),
            cpal::SampleFormat::U32 => build::<u32>(&device, &config, output),
            cpal::SampleFormat::U64 => build::<u64>(&device, &config, output),
            other => bail!("unsupported audio sample format: {other:?}"),
        }?;
        stream.play().context("start audio output")?;
        println!(
            "AUDIO host={:?} device={} rate={sample_rate} channels={} buffer={:?} prebuffer_ms=40 capacity_ms=125 format={:?}",
            host.id(),
            device.name().unwrap_or_default(),
            config.channels,
            config.buffer_size,
            supported.sample_format()
        );
        Ok(Self {
            _stream: stream,
            queue,
            counters,
            volume,
            paused,
            sample_rate,
        })
    }

    pub fn push(&self, samples: &[[f64; 2]]) {
        for &[left, right] in samples {
            let sample = [left as f32, right as f32];
            // Bounded latency: on a producer overrun discard the oldest sample,
            // not the newest sound. Neither side locks or allocates.
            if self.queue.push(sample).is_err() {
                self.queue.pop();
                let _ = self.queue.push(sample);
                self.counters.overflows.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume.store(volume.to_bits(), Ordering::Relaxed);
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
        if paused {
            self.flush();
        }
    }

    pub fn flush(&self) {
        while self.queue.pop().is_some() {}
    }

    pub fn check(&self) -> Result<()> {
        if self.counters.failed.load(Ordering::Relaxed) {
            bail!("audio device stream failed");
        }
        Ok(())
    }

    pub fn stats(&self) -> AudioStats {
        AudioStats {
            callbacks: self.counters.callbacks.load(Ordering::Relaxed),
            frames: self.counters.frames.load(Ordering::Relaxed),
            nonzero_frames: self.counters.nonzero_frames.load(Ordering::Relaxed),
            underrun_frames: self.counters.underrun_frames.load(Ordering::Relaxed),
            overflows: self.counters.overflows.load(Ordering::Relaxed),
            peak: f32::from_bits(self.counters.peak_bits.load(Ordering::Relaxed)),
            queued: self.queue.len(),
        }
    }
}

struct Output {
    queue: Arc<ArrayQueue<[f32; 2]>>,
    counters: Arc<Counters>,
    volume: Arc<AtomicU32>,
    paused: Arc<AtomicBool>,
    prebuffer: usize,
    ready: bool,
    fade: f32,
    last: [f32; 2],
}

impl Output {
    fn frame(&mut self, volume: f32) -> [f32; 2] {
        let paused = self.paused.load(Ordering::Acquire);
        if paused {
            self.ready = false;
        }
        if !paused && !self.ready && self.queue.len() >= self.prebuffer {
            self.ready = true;
        }
        if self.ready {
            if let Some(sample) = self.queue.pop() {
                self.fade = (self.fade + 1.0 / 64.0).min(1.0);
                self.last = [sample[0] * volume, sample[1] * volume];
                return [self.last[0] * self.fade, self.last[1] * self.fade];
            }
            self.ready = false;
            self.counters
                .underrun_frames
                .fetch_add(1, Ordering::Relaxed);
        } else if !paused {
            self.counters
                .underrun_frames
                .fetch_add(1, Ordering::Relaxed);
        }
        // Fade the previous sample to silence on underrun/pause. Resume only
        // after the 40 ms prebuffer refills; never repeat a stale audio chunk.
        self.fade = (self.fade - 1.0 / 64.0).max(0.0);
        [self.last[0] * self.fade, self.last[1] * self.fade]
    }
}

fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut output: Output,
) -> Result<cpal::Stream> {
    let channels = usize::from(config.channels);
    let errors = output.counters.clone();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let volume = f32::from_bits(output.volume.load(Ordering::Relaxed));
                let mut nonzero = 0;
                let mut peak = 0.0_f32;
                for frame in data.chunks_mut(channels) {
                    let sample = output.frame(volume);
                    peak = peak.max(sample[0].abs()).max(sample[1].abs());
                    if sample[0].abs().max(sample[1].abs()) > 0.00001 {
                        nonzero += 1;
                    }
                    if channels == 1 {
                        frame[0] = T::from_sample((sample[0] + sample[1]) * 0.5);
                    } else {
                        frame[0] = T::from_sample(sample[0].clamp(-1.0, 1.0));
                        frame[1] = T::from_sample(sample[1].clamp(-1.0, 1.0));
                        for channel in &mut frame[2..] {
                            *channel = T::from_sample(0.0);
                        }
                    }
                }
                output.counters.callbacks.fetch_add(1, Ordering::Relaxed);
                output
                    .counters
                    .frames
                    .fetch_add((data.len() / channels) as u64, Ordering::Relaxed);
                output
                    .counters
                    .nonzero_frames
                    .fetch_add(nonzero, Ordering::Relaxed);
                output
                    .counters
                    .peak_bits
                    .fetch_max(peak.to_bits(), Ordering::Relaxed);
            },
            move |error| {
                eprintln!("audio stream: {error}");
                errors.failed.store(true, Ordering::Relaxed);
            },
            None,
        )
        .context("open audio output stream")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn underrun_fades_and_waits_for_prebuffer() {
        let queue = Arc::new(ArrayQueue::new(16));
        let mut output = Output {
            queue: queue.clone(),
            counters: Arc::new(Counters::default()),
            volume: Arc::new(AtomicU32::new(1.0_f32.to_bits())),
            paused: Arc::new(AtomicBool::new(false)),
            prebuffer: 2,
            ready: false,
            fade: 0.0,
            last: [0.0; 2],
        };
        queue.push([0.5; 2]).unwrap();
        assert_eq!(output.frame(1.0), [0.0; 2]);
        queue.push([0.5; 2]).unwrap();
        assert!(output.frame(1.0)[0] > 0.0);
        output.frame(1.0);
        for _ in 0..64 {
            output.frame(1.0);
        }
        assert_eq!(output.frame(1.0), [0.0; 2]);
        queue.push([0.5; 2]).unwrap();
        assert_eq!(output.frame(1.0), [0.0; 2]);
        queue.push([0.5; 2]).unwrap();
        assert!(output.frame(1.0)[0] > 0.0);
    }
}
