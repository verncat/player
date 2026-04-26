//! Real audio playback engine using symphonia (decode) + cpal (output).
//!
//! Beat detection implements the same energy-based sliding-window algorithm as
//! <https://docs.rs/beat-detector> (`StrategyKind::LPF`) directly in the decode
//! pipeline. The crate's strategy constructors are crate-private, and its only
//! yanked-dep release (0.1.2) currently can't be used as a Cargo dependency.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use tauri::Emitter;

use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

// ── Beat detection (mirrors BeatStrategyKind::LPF) ──────────────────────────

/// Audio window size used by beat-detector's LPF strategy.
const BEAT_WIN_SIZE: usize = 1024;
/// Number of windows kept in the rolling energy history (~1 s at 44 100 Hz).
const BEAT_HIST_LEN: usize = 43;
/// Energy must exceed BEAT_SENSITIVITY × history mean to count as a beat.
const BEAT_SENSITIVITY: f32 = 1.5;
/// Minimum window energy — ignores silence / quiet passages.
const BEAT_MIN_ENERGY: f32 = 0.0008;
/// Minimum gap between consecutive beat events (ms).
const BEAT_COOLDOWN_MS: u64 = 280;

const SPECTRUM_BANDS: usize = 32;
const SPECTRUM_WIN_SIZE: usize = 1024;
const SPECTRUM_HOP_SIZE: usize = SPECTRUM_WIN_SIZE / 2;
const SPECTRUM_EMIT_INTERVAL_MS: u64 = 45;
const SPECTRUM_MIN_HZ: f32 = 32.0;
const SPECTRUM_MAX_HZ: f32 = 16_000.0;

struct SpectrumAnalyzer {
    buffer: Vec<f32>,
    fft: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex32>,
    fft_buffer: Vec<Complex32>,
    window: Vec<f32>,
    band_ranges: Vec<(usize, usize)>,
    band_peaks: [f32; SPECTRUM_BANDS],
    smoothed: [f32; SPECTRUM_BANDS],
    last_emit_ms: u64,
}

impl SpectrumAnalyzer {
    fn new(sample_rate: u32) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(SPECTRUM_WIN_SIZE);
        let scratch = vec![Complex32::default(); fft.get_inplace_scratch_len()];
        let fft_buffer = vec![Complex32::default(); SPECTRUM_WIN_SIZE];
        let window = (0..SPECTRUM_WIN_SIZE)
            .map(|index| {
                let phase = 2.0 * std::f32::consts::PI * index as f32
                    / (SPECTRUM_WIN_SIZE.saturating_sub(1)) as f32;
                0.5 - 0.5 * phase.cos()
            })
            .collect();

        Self {
            buffer: Vec::with_capacity(SPECTRUM_WIN_SIZE),
            fft,
            scratch,
            fft_buffer,
            window,
            band_ranges: build_spectrum_band_ranges(sample_rate),
            band_peaks: [0.0; SPECTRUM_BANDS],
            smoothed: [0.0; SPECTRUM_BANDS],
            last_emit_ms: 0,
        }
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.smoothed = [0.0; SPECTRUM_BANDS];
        self.last_emit_ms = 0;
    }

    fn feed_frame(&mut self, mono: f32, shared: &Shared) {
        self.buffer.push(mono);
        if self.buffer.len() < SPECTRUM_WIN_SIZE {
            return;
        }

        let now = now_ms();
        if now >= self.last_emit_ms + SPECTRUM_EMIT_INTERVAL_MS {
            self.last_emit_ms = now;
            let next = self.analyze();
            *shared.spectrum.lock().unwrap() = next;
        }

        self.buffer
            .copy_within(SPECTRUM_HOP_SIZE..SPECTRUM_WIN_SIZE, 0);
        self.buffer.truncate(SPECTRUM_WIN_SIZE - SPECTRUM_HOP_SIZE);
    }

    fn analyze(&mut self) -> [f32; SPECTRUM_BANDS] {
        for (index, complex) in self.fft_buffer.iter_mut().enumerate() {
            complex.re = self.buffer[index] * self.window[index];
            complex.im = 0.0;
        }

        self.fft
            .process_with_scratch(&mut self.fft_buffer, &mut self.scratch);

        let mut output = [0.0; SPECTRUM_BANDS];
        for (band_index, &(start, end)) in self.band_ranges.iter().enumerate() {
            let mut energy = 0.0;
            for bin in start..end {
                energy += self.fft_buffer[bin].norm_sqr();
            }

            let count = (end - start).max(1) as f32;
            let rms = (energy / count).sqrt();
            let weight = 1.0 + band_index as f32 / SPECTRUM_BANDS as f32 * 0.85;
            let boosted = rms * weight;
            let floor = 0.0006;
            let peak = (self.band_peaks[band_index] * 0.965).max(boosted.max(floor));

            self.band_peaks[band_index] = peak;

            let normalized = ((boosted - floor) / (peak * 1.15 + floor))
                .clamp(0.0, 1.0)
                .powf(0.68);
            self.smoothed[band_index] = self.smoothed[band_index] * 0.45 + normalized * 0.55;
            output[band_index] = self.smoothed[band_index];
        }

        output
    }
}

fn build_spectrum_band_ranges(sample_rate: u32) -> Vec<(usize, usize)> {
    let nyquist_bin = SPECTRUM_WIN_SIZE / 2;
    let max_hz = (sample_rate as f32 * 0.48).min(SPECTRUM_MAX_HZ).max(SPECTRUM_MIN_HZ + 1.0);
    let log_min = SPECTRUM_MIN_HZ.ln();
    let log_max = max_hz.ln();
    let mut ranges = Vec::with_capacity(SPECTRUM_BANDS);
    let mut start = 1usize;

    for band in 0..SPECTRUM_BANDS {
        let t = (band + 1) as f32 / SPECTRUM_BANDS as f32;
        let edge_hz = (log_min + (log_max - log_min) * t).exp();
        let edge_bin = ((edge_hz * SPECTRUM_WIN_SIZE as f32) / sample_rate as f32)
            .round()
            .max((start + 1) as f32) as usize;
        let end = if band + 1 == SPECTRUM_BANDS {
            nyquist_bin
        } else {
            edge_bin.min(nyquist_bin)
        };
        ranges.push((start, end));
        start = end.min(nyquist_bin.saturating_sub(1));
    }

    ranges
}

fn clear_spectrum(shared: &Shared) {
    *shared.spectrum.lock().unwrap() = [0.0; SPECTRUM_BANDS];
}

struct BeatState {
    mono_buf: Vec<f32>,
    energy_hist: [f32; BEAT_HIST_LEN],
    hist_idx: usize,
    hist_count: usize,
    last_beat_ms: u64,
}

impl BeatState {
    fn new() -> Self {
        BeatState {
            mono_buf: Vec::with_capacity(BEAT_WIN_SIZE),
            energy_hist: [0.0; BEAT_HIST_LEN],
            hist_idx: 0,
            hist_count: 0,
            last_beat_ms: 0,
        }
    }

    /// Feed one mono sample; returns `true` when a beat window is complete and
    /// the energy spike passes the threshold.
    fn feed(&mut self, mono: f32) -> bool {
        self.mono_buf.push(mono);
        if self.mono_buf.len() < BEAT_WIN_SIZE {
            return false;
        }
        let energy: f32 =
            self.mono_buf.iter().map(|s| s * s).sum::<f32>() / BEAT_WIN_SIZE as f32;
        self.mono_buf.clear();

        self.energy_hist[self.hist_idx] = energy;
        self.hist_idx = (self.hist_idx + 1) % BEAT_HIST_LEN;
        if self.hist_count < BEAT_HIST_LEN {
            self.hist_count += 1;
        }
        if self.hist_count < 2 {
            return false;
        }
        let avg: f32 = self.energy_hist[..self.hist_count].iter().copied().sum::<f32>()
            / self.hist_count as f32;
        energy > avg * BEAT_SENSITIVITY && energy > BEAT_MIN_ENERGY
    }
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── Ring buffer shared between decode thread and cpal callback ────────────────

struct RingBuf {
    buf: Vec<f32>,
    write: usize,
    read: usize,
    len: usize,
}

impl RingBuf {
    fn new(capacity: usize) -> Self {
        Self {
            buf: vec![0.0; capacity],
            write: 0,
            read: 0,
            len: 0,
        }
    }

    fn push(&mut self, sample: f32) -> bool {
        if self.len == self.buf.len() {
            return false;
        }
        self.buf[self.write] = sample;
        self.write = (self.write + 1) % self.buf.len();
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<f32> {
        if self.len == 0 {
            return None;
        }
        let val = self.buf[self.read];
        self.read = (self.read + 1) % self.buf.len();
        self.len -= 1;
        Some(val)
    }

    fn available(&self) -> usize {
        self.len
    }

    fn free(&self) -> usize {
        self.buf.len() - self.len
    }
}

// ── Public handle ─────────────────────────────────────────────────────────────

pub struct PlaybackState {
    inner: Arc<Shared>,
}

struct Shared {
    ring: Mutex<RingBuf>,
    ring_cvar: Condvar,
    playing: AtomicBool,
    stop: AtomicBool,
    /// current position in seconds × 1000 (millisecond precision)
    position_ms: AtomicU64,
    /// total duration in seconds × 1000
    duration_ms: AtomicU64,
    /// volume 0.0–1.0 stored as u32 (val × 10000)
    volume: AtomicU64,
    /// Finished naturally (not stopped by user)
    finished: AtomicBool,
    /// Currently loaded file path (for display/debug)
    current_file: Mutex<Option<PathBuf>>,
    /// Selected output device name
    selected_device: Mutex<Option<String>>,
    /// Active cpal output stream — dropped when replaced, which stops it.
    active_stream: Mutex<Option<cpal::Stream>>,
    /// App handle used to emit beat events to the frontend.
    app_handle: tauri::AppHandle,
    /// Latest real-time spectrum snapshot for the UI.
    spectrum: Mutex<[f32; SPECTRUM_BANDS]>,
}

impl PlaybackState {
    pub fn new(app_handle: tauri::AppHandle) -> Self {
        Self {
            inner: Arc::new(Shared {
                ring: Mutex::new(RingBuf::new(192_000)), // ~2s at 48kHz stereo
                ring_cvar: Condvar::new(),
                playing: AtomicBool::new(false),
                stop: AtomicBool::new(false),
                position_ms: AtomicU64::new(0),
                duration_ms: AtomicU64::new(0),
                volume: AtomicU64::new(7000), // 0.7
                finished: AtomicBool::new(false),
                current_file: Mutex::new(None),
                selected_device: Mutex::new(None),
                active_stream: Mutex::new(None),
                app_handle,
                spectrum: Mutex::new([0.0; SPECTRUM_BANDS]),
            }),
        }
    }

    /// Start playing a file. Stops any previous playback first.
    pub fn play(&self, path: PathBuf) -> Result<(), String> {
        // Signal previous threads to stop
        self.inner.stop.store(true, Ordering::SeqCst);
        self.inner.ring_cvar.notify_all();
        // Small delay to let threads notice the stop flag
        thread::sleep(std::time::Duration::from_millis(50));

        // Reset state
        self.inner.stop.store(false, Ordering::SeqCst);
        self.inner.finished.store(false, Ordering::SeqCst);
        self.inner.playing.store(true, Ordering::SeqCst);
        self.inner.position_ms.store(0, Ordering::SeqCst);
        {
            let mut ring = self.inner.ring.lock().unwrap();
            *ring = RingBuf::new(ring.buf.len());
        }
        clear_spectrum(&self.inner);
        *self.inner.current_file.lock().unwrap() = Some(path.clone());

        let shared = Arc::clone(&self.inner);
        thread::spawn(move || {
            if let Err(e) = decode_thread(shared, &path) {
                eprintln!("[playback] decode error: {e}");
            }
        });

        Ok(())
    }

    pub fn pause(&self) {
        self.inner.playing.store(false, Ordering::SeqCst);
        clear_spectrum(&self.inner);
    }

    pub fn resume(&self) {
        self.inner.playing.store(true, Ordering::SeqCst);
    }

    pub fn stop(&self) {
        self.inner.stop.store(true, Ordering::SeqCst);
        self.inner.playing.store(false, Ordering::SeqCst);
        self.inner.ring_cvar.notify_all();
        clear_spectrum(&self.inner);
        // Drop the active stream to stop audio output immediately
        *self.inner.active_stream.lock().unwrap() = None;
    }

    pub fn seek(&self, position_secs: f64) {
        // We set a desired position; the decode thread picks it up.
        self.inner
            .position_ms
            .store((position_secs * 1000.0) as u64, Ordering::SeqCst);
        let was_playing = self.inner.playing.load(Ordering::SeqCst);
        // For now, seeking requires restarting decode from the position.
        // We'll implement this by stopping and replaying from offset.
        let file = self.inner.current_file.lock().unwrap().clone();
        if let Some(path) = file {
            self.inner.stop.store(true, Ordering::SeqCst);
            self.inner.ring_cvar.notify_all();
            thread::sleep(std::time::Duration::from_millis(50));

            self.inner.stop.store(false, Ordering::SeqCst);
            self.inner.finished.store(false, Ordering::SeqCst);
            self.inner.playing.store(was_playing, Ordering::SeqCst);
            {
                let mut ring = self.inner.ring.lock().unwrap();
                *ring = RingBuf::new(ring.buf.len());
            }
            clear_spectrum(&self.inner);

            let shared = Arc::clone(&self.inner);
            let seek_to = position_secs;
            thread::spawn(move || {
                if let Err(e) = decode_thread_seek(shared, &path, seek_to) {
                    eprintln!("[playback] decode-seek error: {e}");
                }
            });
        }
    }

    pub fn set_volume(&self, v: f32) {
        self.inner
            .volume
            .store((v.clamp(0.0, 1.0) * 10000.0) as u64, Ordering::SeqCst);
    }

    pub fn volume(&self) -> f32 {
        self.inner.volume.load(Ordering::Relaxed) as f32 / 10000.0
    }

    pub fn position_secs(&self) -> f64 {
        self.inner.position_ms.load(Ordering::Relaxed) as f64 / 1000.0
    }

    pub fn duration_secs(&self) -> f64 {
        self.inner.duration_ms.load(Ordering::Relaxed) as f64 / 1000.0
    }

    pub fn is_playing(&self) -> bool {
        self.inner.playing.load(Ordering::Relaxed)
    }

    pub fn is_finished(&self) -> bool {
        self.inner.finished.load(Ordering::Relaxed)
    }

    pub fn set_device(&self, name: Option<String>) {
        *self.inner.selected_device.lock().unwrap() = name;
    }

    pub fn selected_device_name(&self) -> Option<String> {
        self.inner.selected_device.lock().unwrap().clone()
    }

    pub fn spectrum(&self) -> Vec<f32> {
        self.inner.spectrum.lock().unwrap().to_vec()
    }
}

// ── Decode thread ─────────────────────────────────────────────────────────────

fn decode_thread(shared: Arc<Shared>, path: &Path) -> Result<(), String> {
    decode_thread_seek(shared, path, 0.0)
}

fn notify_playback_finished(shared: &Shared) {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;

        if let Some(window) = shared.app_handle.get_webview_window("main") {
            if window
                .eval("window._playbackFinished && window._playbackFinished()")
                .is_ok()
            {
                return;
            }
        }
    }

    let _ = shared.app_handle.emit("playback-finished", ());
}

fn decode_thread_seek(shared: Arc<Shared>, path: &Path, seek_secs: f64) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("probe error: {e}"))?;

    let mut format = probed.format;

    let track = format
        .default_track()
        .ok_or("no audio track found")?;

    let track_id = track.id;
    let codec_params = track.codec_params.clone();

    let sample_rate = codec_params.sample_rate.unwrap_or(44100);
    let channels = codec_params
        .channels
        .map(|c| c.count())
        .unwrap_or(2);

    // Store duration
    if let Some(n_frames) = codec_params.n_frames {
        let dur_ms = (n_frames as f64 / sample_rate as f64 * 1000.0) as u64;
        shared.duration_ms.store(dur_ms, Ordering::SeqCst);
    }

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|e| format!("decoder error: {e}"))?;

    // Seek if requested
    if seek_secs > 0.0 {
        let _ = format.seek(
            SeekMode::Coarse,
            SeekTo::Time {
                time: Time::new(seek_secs as u64, seek_secs.fract()),
                track_id: Some(track_id),
            },
        );
    }

    // Start cpal output stream (stores it in shared.active_stream)
    start_output_stream(Arc::clone(&shared), sample_rate, channels as u16)?;

    let mut beat_state = BeatState::new();

    loop {
        if shared.stop.load(Ordering::SeqCst) {
            break;
        }

        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                // End of file
                // Wait for ring to drain
                loop {
                    let avail = shared.ring.lock().unwrap().available();
                    if avail == 0 || shared.stop.load(Ordering::SeqCst) {
                        break;
                    }
                    thread::sleep(std::time::Duration::from_millis(10));
                }
                shared.playing.store(false, Ordering::SeqCst);
                shared.finished.store(true, Ordering::SeqCst);
                clear_spectrum(&shared);
                notify_playback_finished(&shared);
                break;
            }
            Err(e) => {
                eprintln!("[playback] packet error: {e}");
                break;
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                write_to_ring(&shared, &decoded, channels, &mut beat_state);
            }
            Err(symphonia::core::errors::Error::DecodeError(e)) => {
                eprintln!("[playback] decode warning: {e}");
                continue;
            }
            Err(e) => {
                eprintln!("[playback] fatal decode error: {e}");
                break;
            }
        }
    }

    Ok(())
}

fn write_to_ring(shared: &Shared, buf: &AudioBufferRef, channels: usize, beat: &mut BeatState) {
    let frames = buf.frames();
    // Convert to interleaved f32
    match buf {
        AudioBufferRef::F32(b) => {
            for frame in 0..frames {
                let mono = (0..channels).map(|ch| b.chan(ch)[frame]).sum::<f32>() / channels as f32;
                check_beat(shared, beat, mono);
                for ch in 0..channels {
                    let sample = b.chan(ch)[frame];
                    push_sample(shared, sample);
                }
            }
        }
        AudioBufferRef::S16(b) => {
            for frame in 0..frames {
                let mono = (0..channels).map(|ch| b.chan(ch)[frame] as f32 / 32768.0).sum::<f32>() / channels as f32;
                check_beat(shared, beat, mono);
                for ch in 0..channels {
                    let sample = b.chan(ch)[frame] as f32 / 32768.0;
                    push_sample(shared, sample);
                }
            }
        }
        AudioBufferRef::S32(b) => {
            for frame in 0..frames {
                let mono = (0..channels).map(|ch| b.chan(ch)[frame] as f32 / 2_147_483_648.0).sum::<f32>() / channels as f32;
                check_beat(shared, beat, mono);
                for ch in 0..channels {
                    let sample = b.chan(ch)[frame] as f32 / 2_147_483_648.0;
                    push_sample(shared, sample);
                }
            }
        }
        AudioBufferRef::U8(b) => {
            for frame in 0..frames {
                let mono = (0..channels).map(|ch| (b.chan(ch)[frame] as f32 - 128.0) / 128.0).sum::<f32>() / channels as f32;
                check_beat(shared, beat, mono);
                for ch in 0..channels {
                    let sample = (b.chan(ch)[frame] as f32 - 128.0) / 128.0;
                    push_sample(shared, sample);
                }
            }
        }
        AudioBufferRef::F64(b) => {
            for frame in 0..frames {
                let mono = (0..channels).map(|ch| b.chan(ch)[frame] as f32).sum::<f32>() / channels as f32;
                check_beat(shared, beat, mono);
                for ch in 0..channels {
                    let sample = b.chan(ch)[frame] as f32;
                    push_sample(shared, sample);
                }
            }
        }
        _ => {} // unsupported format, skip
    }
}

fn check_beat(shared: &Shared, beat: &mut BeatState, mono: f32) {
    if beat.feed(mono) {
        let now = now_ms();
        if now >= beat.last_beat_ms + BEAT_COOLDOWN_MS {
            beat.last_beat_ms = now;
            use tauri::Emitter;
            let _ = shared.app_handle.emit("beat", now);
        }
    }
}

fn push_sample(shared: &Shared, sample: f32) {
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            return;
        }
        {
            let mut ring = shared.ring.lock().unwrap();
            if ring.push(sample) {
                return;
            }
        }
        // Ring full — wait briefly
        let ring = shared.ring.lock().unwrap();
        let _ = shared
            .ring_cvar
            .wait_timeout(ring, std::time::Duration::from_millis(5));
    }
}

// ── cpal output stream ────────────────────────────────────────────────────────

fn start_output_stream(
    shared: Arc<Shared>,
    sample_rate: u32,
    channels: u16,
) -> Result<(), String> {
    let host = cpal::default_host();

    let selected = shared.selected_device.lock().unwrap().clone();
    let device = if let Some(ref name) = selected {
        host.output_devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.name().ok().as_deref() == Some(name.as_str()))
            .or_else(|| host.default_output_device())
    } else {
        host.default_output_device()
    }
    .ok_or("no output device")?;

    let config = cpal::StreamConfig {
        channels,
        sample_rate: sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };

    let shared2 = Arc::clone(&shared);
    let channels_usize = channels as usize;
    let mut spectrum_analyzer = SpectrumAnalyzer::new(sample_rate);
    let mut spectrum_cleared = true;
    let mut pending_position_ms = 0.0f64;
    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let vol = shared2.volume.load(Ordering::Relaxed) as f32 / 10000.0;
                let playing = shared2.playing.load(Ordering::Relaxed);
                let mut played_frames = 0usize;
                {
                    let mut ring = shared2.ring.lock().unwrap();
                    if playing {
                        for frame in data.chunks_mut(channels_usize) {
                            let mut frame_had_audio = true;
                            for sample in frame.iter_mut() {
                                if let Some(value) = ring.pop() {
                                    *sample = value * vol;
                                } else {
                                    *sample = 0.0;
                                    frame_had_audio = false;
                                }
                            }
                            if frame_had_audio {
                                played_frames += 1;
                            }
                        }
                    } else {
                        for sample in data.iter_mut() {
                            *sample = 0.0;
                        }
                    }
                }
                shared2.ring_cvar.notify_one();

                if !playing {
                    if !spectrum_cleared {
                        spectrum_analyzer.reset();
                        clear_spectrum(&shared2);
                        spectrum_cleared = true;
                    }
                    return;
                }

                spectrum_cleared = false;
                if played_frames > 0 {
                    pending_position_ms += played_frames as f64 * 1000.0 / sample_rate as f64;
                    let add_ms = pending_position_ms.floor() as u64;
                    if add_ms > 0 {
                        shared2.position_ms.fetch_add(add_ms, Ordering::SeqCst);
                        pending_position_ms -= add_ms as f64;
                    }
                }
                for frame in data.chunks(channels_usize) {
                    let mono = frame.iter().copied().sum::<f32>() / channels_usize as f32;
                    spectrum_analyzer.feed_frame(mono, &shared2);
                }
            },
            |err| eprintln!("[playback] stream error: {err}"),
            None,
        )
        .map_err(|e| format!("build stream: {e}"))?;

    stream.play().map_err(|e| format!("play stream: {e}"))?;

    // Store the stream — dropping the previous one stops it.
    *shared.active_stream.lock().unwrap() = Some(stream);

    Ok(())
}

// ── Tauri commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub fn playback_play(
    path: String,
    state: tauri::State<'_, PlaybackState>,
    lib: tauri::State<'_, crate::library::LibraryState>,
) -> Result<(), String> {
    let full = lib.data_dir().join(&path);
    state.play(full)
}

#[tauri::command]
pub fn playback_pause(state: tauri::State<'_, PlaybackState>) {
    state.pause();
}

#[tauri::command]
pub fn playback_resume(state: tauri::State<'_, PlaybackState>) {
    state.resume();
}

#[tauri::command]
pub fn playback_stop(state: tauri::State<'_, PlaybackState>) {
    state.stop();
}

#[tauri::command]
pub fn playback_seek(position: f64, state: tauri::State<'_, PlaybackState>) {
    state.seek(position);
}

#[tauri::command]
pub fn playback_set_volume(value: f32, state: tauri::State<'_, PlaybackState>) {
    state.set_volume(value);
}

#[tauri::command]
pub fn playback_spectrum(state: tauri::State<'_, PlaybackState>) -> Vec<f32> {
    state.spectrum()
}

#[derive(serde::Serialize)]
pub struct PlaybackStatus {
    pub playing: bool,
    pub finished: bool,
    pub position: f64,
    pub duration: f64,
}

#[tauri::command]
pub fn playback_status(state: tauri::State<'_, PlaybackState>) -> PlaybackStatus {
    PlaybackStatus {
        playing: state.is_playing(),
        finished: state.is_finished(),
        position: state.position_secs(),
        duration: state.duration_secs(),
    }
}
