//! Audio output device & volume state.

use cpal::traits::{DeviceTrait, HostTrait};
use serde::Serialize;
use std::sync::Mutex;

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct AudioDevice {
    pub name: String,
    pub sample_rates: Vec<u32>,
}

// ── Managed state ─────────────────────────────────────────────────────────────

pub struct AudioState {
    inner: Mutex<Inner>,
}

struct Inner {
    /// Name of the currently selected output device (or `None` → system default).
    selected_device: Option<String>,
    /// Selected output sample rate for the active local output device.
    selected_sample_rate: Option<u32>,
    /// 0.0 – 1.0
    volume: f32,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                selected_device: None,
                selected_sample_rate: None,
                volume: 0.7,
            }),
        }
    }

    pub fn selected_device_name(&self) -> Option<String> {
        self.inner.lock().unwrap().selected_device.clone()
    }

    pub fn selected_sample_rate(&self) -> Option<u32> {
        self.inner.lock().unwrap().selected_sample_rate
    }

    pub fn volume(&self) -> f32 {
        self.inner.lock().unwrap().volume
    }

    pub fn set_volume(&self, v: f32) {
        self.inner.lock().unwrap().volume = v.clamp(0.0, 1.0);
    }

    pub fn set_device(&self, name: Option<String>, sample_rate: Option<u32>) {
        let mut inner = self.inner.lock().unwrap();
        inner.selected_device = name;
        inner.selected_sample_rate = sample_rate;
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn list_output_devices() -> Vec<AudioDevice> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devs| {
            devs.filter_map(|d| {
                d.name().ok().map(|name| AudioDevice {
                    name,
                    sample_rates: supported_sample_rates(&d),
                })
            })
            .collect()
        })
        .unwrap_or_default()
}

fn supported_sample_rates(device: &cpal::Device) -> Vec<u32> {
    let mut rates = Vec::new();
    let configs = match device.supported_output_configs() {
        Ok(configs) => configs,
        Err(_) => return rates,
    };

    const COMMON_SAMPLE_RATES: &[u32] = &[44_100, 48_000, 88_200, 96_000, 176_400, 192_000];
    for config in configs {
        let min = config.min_sample_rate();
        let max = config.max_sample_rate();
        for rate in COMMON_SAMPLE_RATES {
            if *rate >= min && *rate <= max {
                rates.push(*rate);
            }
        }
    }
    rates.sort_unstable();
    rates.dedup();
    rates
}

fn default_device_name() -> Option<String> {
    cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
}

// ── Tauri commands ────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct DeviceList {
    pub devices: Vec<AudioDevice>,
    /// Name of the currently active device.
    pub current: Option<String>,
    /// Selected output sample rate, or `None` to use the track's native rate.
    pub current_sample_rate: Option<u32>,
}

#[tauri::command]
pub fn get_output_devices(state: tauri::State<'_, AudioState>) -> DeviceList {
    let devices = list_output_devices();
    let selected = state.selected_device_name();
    let current = selected.or_else(default_device_name);
    let current_sample_rate = state.selected_sample_rate();
    DeviceList {
        devices,
        current,
        current_sample_rate,
    }
}

#[tauri::command]
pub fn set_output_device(
    name: Option<String>,
    sample_rate: Option<u32>,
    state: tauri::State<'_, AudioState>,
    playback: tauri::State<'_, crate::playback::PlaybackState>,
) {
    state.set_device(name.clone(), sample_rate);
    playback.set_device(name, sample_rate);
}

#[tauri::command]
pub fn get_volume(state: tauri::State<'_, AudioState>) -> f32 {
    state.volume()
}

#[tauri::command]
pub fn set_volume(value: f32, state: tauri::State<'_, AudioState>) {
    state.set_volume(value);
}
