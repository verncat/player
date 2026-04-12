pub mod audio;
pub mod discovery;
pub mod identify;
pub mod library;
pub mod playback;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::f32::consts::PI;

fn play_sine(freq: f32) -> cpal::Stream {
    let host = cpal::default_host();
    let device = host.default_output_device().expect("no output device available");
    let config = device.default_output_config().expect("no default output config");
    let sample_rate = config.sample_rate() as f32;
    let channels = config.channels() as usize;

    let mut sample_clock = 0f32;
    let stream = device
        .build_output_stream(
            &config.into(),
            move |data: &mut [f32], _| {
                for frame in data.chunks_mut(channels) {
                    let value = (2.0 * PI * freq * sample_clock / sample_rate).sin() * 0.2;
                    sample_clock = (sample_clock + 1.0) % sample_rate;
                    for sample in frame.iter_mut() {
                        *sample = value;
                    }
                }
            },
            |err| eprintln!("stream error: {err}"),
            None,
        )
        .expect("failed to build output stream");
    stream.play().expect("failed to play stream");
    stream
}

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // let _stream = play_sine(440.0);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            use tauri::Manager;
            #[cfg(target_os = "android")]
            let data_dir = std::path::PathBuf::from("/sdcard/Player");
            #[cfg(not(target_os = "android"))]
            let data_dir = app.path().app_data_dir()?.join("data");
            let library = library::LibraryState::new(data_dir, app.handle().clone())
                .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;
            app.manage(library);
            app.manage(audio::AudioState::new());
            app.manage(playback::PlaybackState::new(app.handle().clone()));
            app.manage(discovery::DiscoveryState::new());
            // Auto-start discovery
            let _ = discovery::discovery_start(
                app.state::<discovery::DiscoveryState>(),
                app.handle().clone(),
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            library::search_tracks,
            library::get_all_tracks,
            library::reindex,
            library::get_track_cover,
            library::update_track,
            library::get_data_dir,
            library::record_play,
            library::get_recent_tracks,
            audio::get_output_devices,
            audio::set_output_device,
            audio::get_volume,
            audio::set_volume,
            playback::playback_play,
            playback::playback_pause,
            playback::playback_resume,
            playback::playback_stop,
            playback::playback_seek,
            playback::playback_set_volume,
            playback::playback_status,
            identify::identify_tracks,
            discovery::discovery_start,
            discovery::discovery_stop,
            discovery::discovery_peers,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
