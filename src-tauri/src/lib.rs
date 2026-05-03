pub mod about;
pub mod audio;
pub mod discovery;
pub mod identify;
pub mod library;
pub mod playback;
pub mod soulseek;
pub mod sync;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::f32::consts::PI;
use std::sync::Once;

static INIT_TRACING: Once = Once::new();

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

fn init_tracing() {
    INIT_TRACING.call_once(|| {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            tracing_subscriber::EnvFilter::new(
                "warn,player_lib=info,soulseek_rs=trace"
            )
        });

        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .with_ansi(false)
            .compact()
            .try_init();
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // let _stream = play_sine(440.0);
    init_tracing();

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
            let sync_enabled = library
                .get_device_settings()
                .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?
                .sync_enabled;
            app.manage(library);
            app.manage(audio::AudioState::new());
            app.manage(playback::PlaybackState::new(app.handle().clone()));
            app.manage(discovery::DiscoveryState::new());
            app.manage(soulseek::SoulseekState::new());
            app.manage(sync::SyncState::new(sync_enabled));
            // Auto-start discovery
            let _ = discovery::discovery_start(
                app.state::<discovery::DiscoveryState>(),
                app.handle().clone(),
            );
            sync::ensure_http_server_started(
                &app.state::<sync::SyncState>(),
                &app.state::<library::LibraryState>(),
                &app.handle().clone(),
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            library::search_tracks,
            library::get_all_tracks,
            library::index_track_by_path,
            library::reindex,
            library::get_track_cover,
            library::update_track,
            library::replace_track_with_file,
            library::toggle_like,
            library::get_data_dir,
            library::reveal_track_in_folder,
            library::record_play,
            library::get_recent_tracks,
            library::get_play_history,
            library::get_playlists,
            library::create_playlist,
            library::rename_playlist,
            library::delete_playlist,
            library::set_playlist_pinned,
            library::get_playlist_tracks,
            library::add_track_to_playlist,
            library::remove_track_from_playlist,
            library::get_smart_playlists,
            library::save_smart_playlist,
            library::delete_smart_playlist,
            library::set_smart_playlist_pinned,
            library::find_duplicates,
            library::apply_dedup,
            library::unmark_duplicates,
            library::get_device_emoji,
            library::set_device_emoji,
            library::get_device_settings,
            library::set_device_settings,
            soulseek::soulseek_get_status,
            soulseek::soulseek_search,
            soulseek::soulseek_fetch_cover,
            soulseek::soulseek_download,
            soulseek::soulseek_preview,
            soulseek::soulseek_cancel_preview,
            soulseek::soulseek_promote_preview,
            audio::get_output_devices,
            audio::set_output_device,
            audio::get_volume,
            audio::set_volume,
            about::about_info,
            about::about_check_updates,
            playback::playback_play,
            playback::playback_play_absolute,
            playback::playback_pause,
            playback::playback_resume,
            playback::playback_stop,
            playback::playback_seek,
            playback::playback_set_volume,
            playback::playback_spectrum,
            playback::playback_status,
            identify::identify_tracks,
            discovery::discovery_start,
            discovery::discovery_stop,
            discovery::discovery_peers,
            sync::sync_set_enabled,
            sync::sync_get_enabled,
            sync::sync_with_peer,
            sync::remote_playback_status,
            sync::remote_playback_transfer,
            sync::remote_playback_pause,
            sync::remote_playback_resume,
            sync::remote_playback_stop,
            sync::remote_playback_seek,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
