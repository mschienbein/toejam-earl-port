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

#[derive(Clone, Copy, Debug, Default)]
pub struct AudioStats {
    pub callbacks: u64,
    pub frames: u64,
    pub nonzero_frames: u64,
    pub underrun_frames: u64,
    pub overflows: u64,
    pub peak: f32,
    pub queued: usize,
}

struct StreamAudio {
    _stream: cpal::Stream,
    queue: Arc<ArrayQueue<[f32; 2]>>,
    counters: Arc<Counters>,
    volume: Arc<AtomicU32>,
    paused: Arc<AtomicBool>,
    pub sample_rate: u32,
}

impl StreamAudio {
    fn new(device: &cpal::Device, volume: f32) -> Result<Self> {
        let mut supported = device
            .default_output_config()
            .context("default audio output format")?;
        if !(8_000..=192_000).contains(&supported.sample_rate().0) {
            supported = device
                .supported_output_configs()?
                .find_map(|range| {
                    let low = range.min_sample_rate().0.max(8_000);
                    let high = range.max_sample_rate().0.min(192_000);
                    (low <= high).then(|| {
                        range.with_sample_rate(cpal::SampleRate(48_000_u32.clamp(low, high)))
                    })
                })
                .context("audio device has no supported 8..192 kHz PCM format")?;
        }
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
            cpal::SampleFormat::F32 => build::<f32>(device, &config, output),
            cpal::SampleFormat::F64 => build::<f64>(device, &config, output),
            cpal::SampleFormat::I8 => build::<i8>(device, &config, output),
            cpal::SampleFormat::I16 => build::<i16>(device, &config, output),
            cpal::SampleFormat::I32 => build::<i32>(device, &config, output),
            cpal::SampleFormat::I64 => build::<i64>(device, &config, output),
            cpal::SampleFormat::U8 => build::<u8>(device, &config, output),
            cpal::SampleFormat::U16 => build::<u16>(device, &config, output),
            cpal::SampleFormat::U32 => build::<u32>(device, &config, output),
            cpal::SampleFormat::U64 => build::<u64>(device, &config, output),
            other => bail!("unsupported audio sample format: {other:?}"),
        }?;
        stream.play().context("start audio output")?;
        println!(
            "AUDIO device={} rate={sample_rate} channels={} buffer={:?} prebuffer_ms=40 capacity_ms=125 format={:?}",
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

    fn failed(&self) -> bool {
        self.counters.failed.load(Ordering::Relaxed)
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

#[derive(Clone, Debug, PartialEq)]
struct Fingerprint {
    name: String,
    rate: u32,
    channels: u16,
    format: cpal::SampleFormat,
}

impl Fingerprint {
    fn read(device: &cpal::Device) -> Result<Self> {
        let config = device.default_output_config()?;
        Ok(Self {
            name: device.name()?,
            rate: config.sample_rate().0,
            channels: config.channels(),
            format: config.sample_format(),
        })
    }
}

fn needs_reopen(
    previous: Option<&Fingerprint>,
    current: Option<&Fingerprint>,
    failed: bool,
    changed: bool,
) -> bool {
    changed || failed || previous != current
}

/// Device discovery/recovery is confined to the UI thread. The realtime callback
/// remains allocation-free and never enumerates devices or takes a mutex.
pub struct Audio {
    stream: Option<StreamAudio>,
    device: Option<String>,
    fingerprint: Option<Fingerprint>,
    volume: f32,
    paused: bool,
    changed: bool,
    next_probe: std::time::Instant,
    last_progress: std::time::Instant,
    seen_callbacks: u64,
    status: String,
    accumulated: AudioStats,
    pub sample_rate: u32,
}

impl Audio {
    pub fn new(volume: f32, device: Option<String>) -> Self {
        let now = std::time::Instant::now();
        let mut audio = Self {
            stream: None,
            device,
            fingerprint: None,
            volume,
            paused: false,
            changed: true,
            next_probe: now,
            last_progress: now,
            seen_callbacks: 0,
            status: String::new(),
            accumulated: AudioStats::default(),
            sample_rate: 48_000,
        };
        audio.maintain(now);
        audio
    }

    pub fn devices() -> Result<Vec<String>> {
        let mut names: Vec<_> = cpal::default_host()
            .output_devices()?
            .filter_map(|device| device.name().ok())
            .collect();
        names.sort();
        names.dedup();
        Ok(names)
    }

    pub fn set_device(&mut self, device: Option<String>) {
        if self.device != device {
            self.device = device;
            self.changed = true;
        }
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    /// Follow the system default or an explicitly selected device. A removed
    /// output does not terminate the game or discard a controller recording.
    /// A newly negotiated rate is returned for the backend's audio resampler.
    pub fn maintain(&mut self, now: std::time::Instant) -> Option<u32> {
        let callbacks = self.stream.as_ref().map_or(0, |stream| {
            stream.counters.callbacks.load(Ordering::Relaxed)
        });
        if callbacks != self.seen_callbacks {
            self.seen_callbacks = callbacks;
            self.last_progress = now;
        }
        let stalled = self.stream.is_some()
            && now.saturating_duration_since(self.last_progress)
                > std::time::Duration::from_secs(2);
        let failed = stalled || self.stream.as_ref().is_some_and(StreamAudio::failed);
        if !self.changed && !failed && now < self.next_probe {
            return None;
        }
        self.next_probe = now + std::time::Duration::from_secs(1);
        let host = cpal::default_host();
        let device = (|| -> Result<cpal::Device> {
            match &self.device {
                None => host
                    .default_output_device()
                    .context("no default audio output is connected"),
                Some(name) => host
                    .output_devices()?
                    .find(|device| device.name().ok().as_ref() == Some(name))
                    .with_context(|| format!("selected audio output {name:?} is disconnected")),
            }
        })();
        let current = device
            .as_ref()
            .ok()
            .and_then(|device| Fingerprint::read(device).ok());
        if self.stream.is_some()
            && !needs_reopen(
                self.fingerprint.as_ref(),
                current.as_ref(),
                failed,
                self.changed,
            )
        {
            return None;
        }
        self.changed = false;
        if let Some(stream) = self.stream.take() {
            let stats = stream.stats();
            self.accumulated.callbacks += stats.callbacks;
            self.accumulated.frames += stats.frames;
            self.accumulated.nonzero_frames += stats.nonzero_frames;
            self.accumulated.underrun_frames += stats.underrun_frames;
            self.accumulated.overflows += stats.overflows;
            self.accumulated.peak = self.accumulated.peak.max(stats.peak);
        }
        self.fingerprint = current;
        match device.and_then(|device| StreamAudio::new(&device, self.volume)) {
            Ok(stream) => {
                stream.set_paused(self.paused);
                let rate = stream.sample_rate;
                let name = self
                    .fingerprint
                    .as_ref()
                    .map_or("output", |device| device.name.as_str());
                let message = format!("Audio connected: {name} · {rate} Hz");
                if self.status != message {
                    println!("{message}");
                    self.status = message;
                }
                self.stream = Some(stream);
                self.seen_callbacks = 0;
                self.last_progress = now;
                let changed_rate = rate != self.sample_rate;
                self.sample_rate = rate;
                changed_rate.then_some(rate)
            }
            Err(error) => {
                let message = format!(
                    "Audio unavailable: {error:#}. Reconnecting automatically; choose another output in Settings."
                );
                if self.status != message {
                    eprintln!("{message}");
                    self.status = message;
                }
                None
            }
        }
    }

    pub fn push(&self, samples: &[[f64; 2]]) {
        if let Some(stream) = &self.stream {
            stream.push(samples);
        }
    }
    pub fn flush(&self) {
        if let Some(stream) = &self.stream {
            stream.flush();
        }
    }
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        if let Some(stream) = &self.stream {
            stream.set_paused(paused);
        }
    }
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        if let Some(stream) = &self.stream {
            stream.set_volume(volume);
        }
    }
    pub fn stats(&self) -> AudioStats {
        let mut stats = self.accumulated;
        if let Some(stream) = &self.stream {
            let current = stream.stats();
            stats.callbacks += current.callbacks;
            stats.frames += current.frames;
            stats.nonzero_frames += current.nonzero_frames;
            stats.underrun_frames += current.underrun_frames;
            stats.overflows += current.overflows;
            stats.peak = stats.peak.max(current.peak);
            stats.queued = current.queued;
        }
        stats
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

    #[test]
    fn audio_reopens_for_loss_default_change_rate_change_and_user_selection() {
        let original = Fingerprint {
            name: "Output".into(),
            rate: 48_000,
            channels: 2,
            format: cpal::SampleFormat::F32,
        };
        assert!(!needs_reopen(
            Some(&original),
            Some(&original),
            false,
            false
        ));
        assert!(needs_reopen(Some(&original), None, false, false));
        assert!(needs_reopen(None, Some(&original), false, false));
        assert!(needs_reopen(Some(&original), Some(&original), true, false));
        assert!(needs_reopen(Some(&original), Some(&original), false, true));
        let changed = Fingerprint {
            rate: 44_100,
            ..original.clone()
        };
        assert!(needs_reopen(Some(&original), Some(&changed), false, false));
        let changed = Fingerprint {
            name: "New default".into(),
            ..original.clone()
        };
        assert!(needs_reopen(Some(&original), Some(&changed), false, false));
    }
}
