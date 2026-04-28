use soulseek_rs::{Client, DownloadStatus};
use std::io::{self, Write};
use std::time::Duration;
use tracing_appender::rolling;
use tracing_subscriber::fmt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file_appender = rolling::never("downloads", "soulseek.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    fmt().with_writer(non_blocking).with_ansi(false).init();

    // Prompt for credentials
    print!("Username: ");
    io::stdout().flush()?;
    let mut username = String::new();
    io::stdin().read_line(&mut username)?;

    print!("Password: ");
    io::stdout().flush()?;
    let mut password = String::new();
    io::stdin().read_line(&mut password)?;

    // Connect and login
    let client = Client::new(username.trim(), password.trim());
    client.connect().await?;
    println!("Logged in!\n");

    // Search
    print!("Search: ");
    io::stdout().flush()?;
    let mut query = String::new();
    io::stdin().read_line(&mut query)?;

    println!("Searching (10s)...");
    let results = client.search(query.trim(), Duration::from_secs(10)).await?;

    // Flatten all files with their source info
    let files: Vec<_> = results
        .iter()
        .flat_map(|r| r.files.iter().map(move |f| (f, r.speed)))
        .collect();

    if files.is_empty() {
        println!("No results.");
        return Ok(());
    }

    // Display results (max 20)
    let display_count = files.len().min(20);
    for (i, (file, speed)) in files.iter().take(display_count).enumerate() {
        let size_mb = file.size as f64 / 1_048_576.0;
        let name = file.name.filename();
        let duration = file
            .attributes
            .duration
            .map(|duration| format!(", {duration}s"))
            .unwrap_or_default();
        println!(
            "[{i}] {name} ({size_mb:.1} MB, {speed} KB/s{duration}) - {}",
            file.username
        );
    }

    // Pick a file
    print!("\nSelect [0-{}]: ", display_count - 1);
    io::stdout().flush()?;
    let mut choice = String::new();
    io::stdin().read_line(&mut choice)?;
    let idx: usize = choice.trim().parse()?;
    let (file, _) = &files[idx];

    // Download
    let (_dl, mut handle) = client.download(
        file.name.clone(),
        file.username.clone(),
        file.size,
        "./downloads".to_string(),
        None,
        None,
    )?;

    println!("Downloading...");
    while let Some(status) = handle.recv().await {
        match status {
            DownloadStatus::QueuedLocally => println!("Queued locally..."),
            DownloadStatus::QueuedRemotely { place } => println!("Queued remotely (place: {:?})...", place),
            DownloadStatus::InProgress {
                bytes_downloaded,
                total_bytes,
                speed_bytes_per_sec,
            } => {
                let pct = bytes_downloaded as f64 / total_bytes as f64 * 100.0;
                let speed_kb = speed_bytes_per_sec / 1024.0;
                print!("\r{pct:5.1}% ({speed_kb:.0} KB/s)");
                io::stdout().flush()?;
            }
            DownloadStatus::Completed => {
                println!("\nDone!");
                break;
            }
            DownloadStatus::Failed => {
                eprintln!("\nDownload failed.");
                break;
            }
            DownloadStatus::TimedOut => {
                eprintln!("\nDownload timed out.");
                break;
            }
            DownloadStatus::Cancelled => {
                eprintln!("\nDownload cancelled.");
                break;
            }
        }
    }

    Ok(())
}
