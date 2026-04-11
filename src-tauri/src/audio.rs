//! Audio output device & volume state.

use cpal::traits::{DeviceTrait, HostTrait};
use serde::Serialize;
use std::sync::Mutex;

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct AudioDevice {
    pub name: String,
}

// ── Managed state ─────────────────────────────────────────────────────────────

pub struct AudioState {
    inner: Mutex<Inner>,
}

struct Inner {
    /// Name of the currently selected output device (or `None` → system default).
    selected_device: Option<String>,
    /// 0.0 – 1.0
    volume: f32,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                selected_device: None,
                volume: 0.7,
            }),
        }
    }

    pub fn selected_device_name(&self) -> Option<String> {
        self.inner.lock().unwrap().selected_device.clone()
    }

    pub fn volume(&self) -> f32 {
        self.inner.lock().unwrap().volume
    }

    pub fn set_volume(&self, v: f32) {
        self.inner.lock().unwrap().volume = v.clamp(0.0, 1.0);
    }

    pub fn set_device(&self, name: Option<String>) {
        self.inner.lock().unwrap().selected_device = name;
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn list_output_devices() -> Vec<AudioDevice> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devs| {
            devs.filter_map(|d| {
                d.name().ok().map(|name| AudioDevice { name })
            })
            .collect()
        })
        .unwrap_or_default()
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
}

#[tauri::command]
pub fn get_output_devices(state: tauri::State<'_, AudioState>) -> DeviceList {
    let devices = list_output_devices();
    let selected = state.selected_device_name();
    let current = selected.or_else(default_device_name);
    DeviceList { devices, current }
}

#[tauri::command]
pub fn set_output_device(name: Option<String>, state: tauri::State<'_, AudioState>) {
    state.set_device(name);
}

#[tauri::command]
pub fn get_volume(state: tauri::State<'_, AudioState>) -> f32 {
    state.volume()
}

#[tauri::command]
pub fn set_volume(value: f32, state: tauri::State<'_, AudioState>) {
    state.set_volume(value);
}
