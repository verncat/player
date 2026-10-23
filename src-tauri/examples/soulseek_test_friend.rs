//! Isolated, allowlisted test friend using the same Soulseek client as Player.
use player_lib::profile::LocalProfileState;
use rusqlite::{params, Connection, OptionalExtension};
use soulseek_rs::actor::server_actor::UserMessage;
use soulseek_rs::{Client, ClientSettings, PeerAddress};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const PREFIX: &str = "[Player test friend] ";

struct Options {
    allowed: String,
    directory: Option<PathBuf>,
    duration: u64,
    max_replies: usize,
    server: Option<(String, u16)>,
}

fn username_valid(username: &str) -> bool {
    !username.is_empty()
        && username.len() <= 30
        && username.trim() == username
        && username
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ')
}

fn options() -> Result<Option<Options>> {
    let mut args = std::env::args().skip(1);
    let mut options = Options {
        allowed: String::new(),
        directory: None,
        duration: 600,
        max_replies: 50,
        server: None,
    };
    while let Some(argument) = args.next() {
        if argument == "--help" || argument == "-h" {
            println!("Player Soulseek test friend\n\nUsage: pnpm soulseek:test-friend --allow YOUR_SOULSEEK_USERNAME\n\n--allow USER       Only reply to this Soulseek account (required)\n--state-dir PATH   Reuse an existing bot identity and credentials\n--duration N       Run for N seconds (default 600, maximum 3600)\n--max-replies N    Stop after N replies (default 50, maximum 100)\n--server HOST:PORT Override the server for local protocol testing\n\nA new state directory is created outside the repository by default.\nThe bot prints its Profile ID and username, but never its password or private key.");
            return Ok(None);
        }
        let value = args
            .next()
            .ok_or("Missing option value. Run with --help.")?;
        match argument.as_str() {
            "--allow" => options.allowed = value,
            "--state-dir" => options.directory = Some(value.into()),
            "--duration" => options.duration = value.parse()?,
            "--max-replies" => options.max_replies = value.parse()?,
            "--server" => {
                let (host, port) = value
                    .rsplit_once(':')
                    .ok_or("Use HOST:PORT for --server.")?;
                if host.is_empty() {
                    return Err("Server host cannot be empty.".into());
                }
                options.server = Some((host.to_owned(), port.parse()?));
            }
            _ => return Err(format!("Unknown option: {argument}").into()),
        }
    }
    if !username_valid(&options.allowed) {
        return Err("Pass your Soulseek username with --allow USER.".into());
    }
    if !(1..=3600).contains(&options.duration) || !(1..=100).contains(&options.max_replies) {
        return Err("Use duration 1..3600 and max-replies 1..100.".into());
    }
    Ok(Some(options))
}

fn state_directory(directory: Option<PathBuf>) -> Result<PathBuf> {
    let path = if let Some(path) = directory {
        if !path.exists() {
            std::fs::create_dir(&path)?;
        }
        if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err("State directory must not be a symlink.".into());
        }
        for entry in std::fs::read_dir(&path)? {
            let entry = entry?;
            let allowed = [
                "bot.sqlite",
                "bot.sqlite-journal",
                "bot.sqlite-wal",
                "bot.sqlite-shm",
                "profile.key",
            ];
            if !allowed.iter().any(|name| entry.file_name() == *name)
                || entry.file_type()?.is_symlink()
            {
                return Err(
                    "Use an empty directory or an existing test friend state directory.".into(),
                );
            }
        }
        path
    } else {
        tempfile::Builder::new()
            .prefix("player-soulseek-test-friend-")
            .tempdir()?
            .keep()
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(std::fs::canonicalize(path)?)
}

fn database(path: &Path) -> Result<Connection> {
    let mut file_options = std::fs::OpenOptions::new();
    file_options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        file_options.mode(0o600);
    }
    file_options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    let connection = Connection::open(path)?;
    init_schema(&connection)?;
    Ok(connection)
}

fn init_schema(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS account (id INTEGER PRIMARY KEY CHECK(id=1), username TEXT NOT NULL, password TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS inbox (sender TEXT NOT NULL, server_id INTEGER NOT NULL, timestamp INTEGER NOT NULL, body TEXT NOT NULL, state TEXT NOT NULL,
            PRIMARY KEY(sender, server_id, timestamp));
        UPDATE inbox SET state = 'unknown' WHERE state = 'sending';")
}

// Commit before acknowledging. Replays must never create a second echo.
fn receive(connection: &Connection, allowed: &str, message: &UserMessage) -> rusqlite::Result<()> {
    if message.username != allowed {
        return Ok(());
    }
    let state = if message.message.starts_with(PREFIX) {
        "ignored"
    } else {
        "pending"
    };
    connection.execute("INSERT INTO inbox(sender, server_id, timestamp, body, state) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING",
        params![message.username, message.id, message.timestamp, message.message, state])?;
    Ok(())
}

fn echo(body: &str) -> String {
    let mut end = body.len().min(4096 - PREFIX.len());
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{PREFIX}{}", &body[..end])
}

async fn run_bot(options: Options, directory: PathBuf) -> Result<()> {
    let profile = LocalProfileState::open(Some(directory.join("profile.key")))?;
    let profile_id = profile.info().user_id;
    let marker = format!("adhdplayer-v1-{profile_id}");
    let connection = Arc::new(Mutex::new(database(&directory.join("bot.sqlite"))?));
    let (username, password) = {
        let connection = connection.lock().unwrap();
        let existing = connection
            .query_row(
                "SELECT username, password FROM account WHERE id = 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some(account) = existing {
            account
        } else {
            let username = format!(
                "PlayerTest_{}",
                &uuid::Uuid::new_v4().simple().to_string()[..12]
            );
            let password = uuid::Uuid::new_v4().simple().to_string();
            connection.execute(
                "INSERT INTO account VALUES (1, ?1, ?2)",
                params![username, password],
            )?;
            (username, password)
        }
    };
    if username == options.allowed {
        return Err("The bot must use a different Soulseek account.".into());
    }
    println!("State directory: {}\nTest Profile ID: {profile_id}\nSoulseek username: {username}\nAllowed sender: {}", directory.display(), options.allowed);
    let mut settings = ClientSettings::new(username, password);
    settings.enable_listen = false;
    settings.shared_folders = 0;
    settings.shared_files = 0;
    if let Some((host, port)) = options.server {
        settings.server_address = PeerAddress::new(host, port);
    }
    let client = Client::with_settings(settings);
    let incoming = connection.clone();
    let allowed = options.allowed.clone();
    client.set_private_message_handler(Arc::new(move |message| {
        let Ok(connection) = incoming.lock() else {
            return false;
        };
        receive(&connection, &allowed, message).is_ok()
    }));
    let result = async {
        println!("Connecting to Soulseek...");
        tokio::time::timeout(Duration::from_secs(30), client.connect()).await??;
        client.set_discovery_interest(None, Some(marker.clone())).await?;
        println!("Ready. Add the Test Profile ID in Friends, then send a message.\nThe bot replies only to {}. Press Ctrl+C to stop.", options.allowed);
        let deadline = Instant::now() + Duration::from_secs(options.duration);
        let mut recent = VecDeque::<Instant>::new();
        let mut replies = 0;
        let mut interval = tokio::time::interval(Duration::from_millis(200));
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                _ = tokio::time::sleep_until(deadline) => break,
                _ = interval.tick() => {}
            }
            let now = Instant::now();
            while recent.front().is_some_and(|time| now.duration_since(*time) >= Duration::from_secs(60)) { recent.pop_front(); }
            if recent.len() >= 10 || recent.back().is_some_and(|time| now.duration_since(*time) < Duration::from_secs(1)) { continue; }
            let pending = {
                let connection = connection.lock().unwrap();
                let message = connection.query_row("SELECT server_id, timestamp, body FROM inbox WHERE sender = ?1 AND state = 'pending' ORDER BY timestamp, server_id LIMIT 1",
                    [&options.allowed], |row| Ok((row.get::<_, u32>(0)?, row.get::<_, u32>(1)?, row.get::<_, String>(2)?))).optional()?;
                if let Some((id, timestamp, _)) = &message {
                    connection.execute("UPDATE inbox SET state = 'sending' WHERE sender = ?1 AND server_id = ?2 AND timestamp = ?3", params![options.allowed, id, timestamp])?;
                }
                message
            };
            let Some((id, timestamp, body)) = pending else { continue; };
            let sent = client.send_private_message(&options.allowed, &echo(&body)).await;
            let state = if sent.is_ok() { "submitted" } else { "unknown" };
            connection.lock().unwrap().execute("UPDATE inbox SET state = ?4 WHERE sender = ?1 AND server_id = ?2 AND timestamp = ?3", params![options.allowed, id, timestamp, state])?;
            recent.push_back(now);
            if sent.is_ok() {
                replies += 1;
                println!("Reply submitted to {} ({replies}/{}).", options.allowed, options.max_replies);
                if replies >= options.max_replies { break; }
            } else {
                eprintln!("Reply could not be confirmed; it will not be retried automatically.");
            }
        }
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    }.await;
    if let Err(error) = client.set_discovery_interest(Some(marker), None).await {
        eprintln!("Could not remove the discovery marker: {error}");
    }
    client.disconnect();
    result
}

#[tokio::main]
async fn main() {
    let result = async {
        let Some(options) = options()? else {
            return Ok(());
        };
        let directory = state_directory(options.directory.clone())?;
        run_bot(options, directory).await
    }
    .await;
    if let Err(error) = result {
        eprintln!("Test friend stopped: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bot_only_queues_allowed_messages_and_deduplicates_replays() {
        let connection = Connection::open_in_memory().unwrap();
        init_schema(&connection).unwrap();
        let message = UserMessage::new(1, 42, "alice".into(), "hello".into(), true);
        receive(&connection, "alice", &message).unwrap();
        receive(&connection, "alice", &message).unwrap();
        receive(
            &connection,
            "alice",
            &UserMessage::new(2, 43, "stranger".into(), "no reply".into(), true),
        )
        .unwrap();
        receive(
            &connection,
            "alice",
            &UserMessage::new(3, 44, "alice".into(), format!("{PREFIX}no loop"), true),
        )
        .unwrap();
        let pending: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM inbox WHERE state = 'pending'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let all: i64 = connection
            .query_row("SELECT COUNT(*) FROM inbox", [], |row| row.get(0))
            .unwrap();
        assert_eq!(pending, 1);
        assert_eq!(all, 2);
    }
    #[test]
    fn echo_respects_utf8_and_message_size() {
        assert_eq!(echo("hello"), format!("{PREFIX}hello"));
        let reply = echo(&"🎵".repeat(4096));
        assert!(reply.len() <= 4096);
        assert!(reply.starts_with(PREFIX));
        assert!(username_valid("alice smith"));
        assert!(!username_valid("alice\n"));
    }
}
