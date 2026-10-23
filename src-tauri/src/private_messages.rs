use crate::{friends, library::LibraryState, profile::LocalProfileState, soulseek::SoulseekState};
use rusqlite::{params, Connection};
use serde::Serialize;
use soulseek_rs::actor::server_actor::UserMessage;

#[derive(Serialize)]
pub struct ChatMessage {
    id: i64,
    direction: String,
    body: String,
    timestamp: i64,
    state: String,
    track_request: Option<crate::track_requests::TrackRequest>,
}

#[derive(Serialize)]
pub struct FriendChat {
    friend: friends::Friend,
    account: String,
    usernames: Vec<String>,
    configured: bool,
    enabled: bool,
}

#[derive(Serialize)]
pub struct MessagePage {
    messages: Vec<ChatMessage>,
    has_more: bool,
}

pub(crate) fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    crate::track_requests::init_schema(conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS friend_chat_accounts (
            owner_id TEXT NOT NULL, friend_id TEXT NOT NULL, username TEXT NOT NULL,
            PRIMARY KEY(owner_id, friend_id, username)
        );
        CREATE TABLE IF NOT EXISTS soulseek_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL, account TEXT NOT NULL, username TEXT NOT NULL,
            direction TEXT NOT NULL, body TEXT NOT NULL, timestamp INTEGER NOT NULL,
            state TEXT NOT NULL, server_id INTEGER,
            UNIQUE(owner_id, account, username, server_id, timestamp)
        );
        CREATE INDEX IF NOT EXISTS idx_soulseek_conversation ON soulseek_messages(owner_id, account, username, id);
        UPDATE soulseek_messages SET state = 'unknown' WHERE state = 'sending';",
    )
}

pub(crate) fn receive(
    conn: &Connection,
    owner: &str,
    account: &str,
    message: &UserMessage,
) -> rusqlite::Result<()> {
    let transaction = conn.unchecked_transaction()?;
    let inserted = transaction.execute(
        "INSERT INTO soulseek_messages (owner_id, account, username, direction, body, timestamp, state, server_id)
         VALUES (?1, ?2, ?3, 'incoming', ?4, ?5, 'received', ?6) ON CONFLICT DO NOTHING",
        params![owner, account, message.username, message.message, message.timestamp, message.id],
    )?;
    if inserted > 0 {
        crate::track_requests::receive(&transaction, owner, account, message)?;
    }
    transaction.commit()?;
    Ok(())
}

fn friend_accounts(
    conn: &Connection,
    owner: &str,
    profile_id: &str,
) -> Result<(friends::Friend, Vec<String>), String> {
    let friend = friends::find(conn, owner, profile_id)?;
    for username in &friend.soulseek_users {
        conn.execute(
            "INSERT OR IGNORE INTO friend_chat_accounts VALUES (?1, ?2, ?3)",
            params![owner, friend.profile_id, username],
        )
        .map_err(|e| e.to_string())?;
    }
    let mut statement = conn.prepare("SELECT username FROM friend_chat_accounts WHERE owner_id = ?1 AND friend_id = ?2 ORDER BY username")
        .map_err(|e| e.to_string())?;
    let usernames = statement
        .query_map(params![owner, friend.profile_id], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(|e| e.to_string())?;
    Ok((friend, usernames))
}

pub(crate) fn authorize_target(
    conn: &Connection,
    owner: &str,
    friend_id: &str,
    username: &str,
) -> Result<(), String> {
    let (_, accounts) = friend_accounts(conn, owner, friend_id)?;
    if !accounts.iter().any(|account| account == username) {
        return Err("Choose a Soulseek account linked to this friend.".into());
    }
    Ok(())
}

fn read_page(
    conn: &Connection,
    owner: &str,
    account: &str,
    username: &str,
    before_id: Option<i64>,
) -> Result<MessagePage, String> {
    let mut statement = conn.prepare("SELECT id, direction, body, timestamp, state FROM soulseek_messages
        WHERE owner_id = ?1 AND account = ?2 AND username = ?3 AND id < ?4 ORDER BY id DESC LIMIT 101")
        .map_err(|e| e.to_string())?;
    let mut messages = statement
        .query_map(
            params![owner, account, username, before_id.unwrap_or(i64::MAX)],
            |row| {
                let body: String = row.get(2)?;
                let track_request =
                    crate::track_requests::message_card(conn, owner, account, username, &body)?;
                Ok(ChatMessage {
                    id: row.get(0)?,
                    direction: row.get(1)?,
                    body: crate::track_requests::display_body(&body),
                    track_request,
                    timestamp: row.get(3)?,
                    state: row.get(4)?,
                })
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let has_more = messages.len() > 100;
    messages.truncate(100);
    messages.reverse();
    Ok(MessagePage { messages, has_more })
}

#[tauri::command]
pub fn get_friend_chat(
    profile_id: String,
    library: tauri::State<'_, LibraryState>,
    profile: tauri::State<'_, LocalProfileState>,
) -> Result<FriendChat, String> {
    let owner = profile.info().user_id;
    let settings = library.get_device_settings().map_err(|e| e.to_string())?;
    let connection = library.conn();
    let conn = connection
        .lock()
        .map_err(|_| "Could not open this conversation.")?;
    let (friend, usernames) = friend_accounts(&conn, &owner, &profile_id)?;
    Ok(FriendChat {
        friend,
        usernames,
        configured: !settings.soulseek_username.trim().is_empty()
            && !settings.soulseek_password.is_empty(),
        enabled: settings.soulseek_enabled,
        account: settings.soulseek_username.trim().to_owned(),
    })
}

#[tauri::command]
pub fn get_friend_messages(
    profile_id: String,
    account: String,
    username: String,
    before_id: Option<i64>,
    library: tauri::State<'_, LibraryState>,
    profile: tauri::State<'_, LocalProfileState>,
) -> Result<MessagePage, String> {
    let owner = profile.info().user_id;
    let connection = library.conn();
    let conn = connection.lock().map_err(|_| "Could not load messages.")?;
    authorize_target(&conn, &owner, &profile_id, &username)?;
    read_page(&conn, &owner, &account, &username, before_id)
}

#[tauri::command]
pub async fn send_friend_message(
    profile_id: String,
    account: String,
    username: String,
    body: String,
    library: tauri::State<'_, LibraryState>,
    profile: tauri::State<'_, LocalProfileState>,
    soulseek: tauri::State<'_, SoulseekState>,
) -> Result<(), String> {
    if body.trim().is_empty() || body.len() > 4096 {
        return Err("Enter a message of up to 4096 bytes.".into());
    }
    let owner = profile.info().user_id;
    let settings = library.get_device_settings().map_err(|e| e.to_string())?;
    if !settings.soulseek_enabled || account != settings.soulseek_username.trim() {
        return Err(
            "Your Soulseek account changed or is disabled. Reopen this conversation.".into(),
        );
    }
    let connection = library.conn();
    let message_id = {
        let conn = connection
            .lock()
            .map_err(|_| "Could not save your message.")?;
        authorize_target(&conn, &owner, &profile_id, &username)?;
        conn.execute("INSERT INTO soulseek_messages (owner_id, account, username, direction, body, timestamp, state)
            VALUES (?1, ?2, ?3, 'outgoing', ?4, strftime('%s','now'), 'sending')", params![owner, account, username, body])
            .map_err(|e| e.to_string())?;
        conn.last_insert_rowid()
    };
    let mut attempted = false;
    let result = async {
        let client = soulseek.discovery_client(&library, &profile).await?;
        let latest = library.get_device_settings().map_err(|e| e.to_string())?;
        if profile.info().user_id != owner
            || !latest.soulseek_enabled
            || latest.soulseek_username.trim() != account
        {
            return Err(
                "Your profile or Soulseek account changed. The message was not sent.".into(),
            );
        }
        {
            let conn = connection
                .lock()
                .map_err(|_| "Could not read this conversation.")?;
            authorize_target(&conn, &owner, &profile_id, &username)?;
        }
        attempted = true;
        client
            .send_private_message(&username, &body)
            .await
            .map_err(|error| error.to_string())
    }
    .await;
    let state = if result.is_ok() {
        "submitted"
    } else if attempted {
        "unknown"
    } else {
        "failed"
    };
    {
        let conn = connection
            .lock()
            .map_err(|_| "Could not update message status.")?;
        conn.execute(
            "UPDATE soulseek_messages SET state = ?2 WHERE id = ?1",
            params![message_id, state],
        )
        .map_err(|e| e.to_string())?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_messages_are_durable_deduplicated_and_account_scoped() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("chat.db");
        let message = UserMessage::new(7, 1700000000, "alice".into(), "Hello 🎵".into(), true);
        {
            let conn = Connection::open(&path).unwrap();
            init_schema(&conn).unwrap();
            receive(&conn, "owner", "me", &message).unwrap();
            receive(&conn, "owner", "me", &message).unwrap();
            receive(&conn, "other", "me", &message).unwrap();
            receive(&conn, "owner", "second", &message).unwrap();
        }
        let conn = Connection::open(path).unwrap();
        init_schema(&conn).unwrap();
        let page = read_page(&conn, "owner", "me", "alice", None).unwrap();
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].body, "Hello 🎵");
        assert!(read_page(&conn, "owner", "me", "unknown", None)
            .unwrap()
            .messages
            .is_empty());
        assert_eq!(
            read_page(&conn, "other", "me", "alice", None)
                .unwrap()
                .messages
                .len(),
            1
        );
        assert_eq!(
            read_page(&conn, "owner", "second", "alice", None)
                .unwrap()
                .messages
                .len(),
            1
        );
    }

    #[test]
    fn chat_history_is_paginated_and_interrupted_sends_remain_unconfirmed() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        for id in 0..105 {
            receive(
                &conn,
                "owner",
                "me",
                &UserMessage::new(id, 1700000000 + id, "alice".into(), id.to_string(), true),
            )
            .unwrap();
        }
        let latest = read_page(&conn, "owner", "me", "alice", None).unwrap();
        assert_eq!(latest.messages.len(), 100);
        assert!(latest.has_more);
        let older = read_page(&conn, "owner", "me", "alice", Some(latest.messages[0].id)).unwrap();
        assert_eq!(older.messages.len(), 5);
        assert!(!older.has_more);
        conn.execute("INSERT INTO soulseek_messages (owner_id, account, username, direction, body, timestamp, state)
            VALUES ('owner', 'me', 'alice', 'outgoing', 'pending', 1, 'sending')", []).unwrap();
        init_schema(&conn).unwrap();
        assert_eq!(
            read_page(&conn, "owner", "me", "alice", None)
                .unwrap()
                .messages
                .last()
                .unwrap()
                .state,
            "unknown"
        );
    }

    #[test]
    fn chat_targets_must_belong_to_the_current_friend_and_profile() {
        let conn = Connection::open_in_memory().unwrap();
        friends::init_schema(&conn).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute("INSERT INTO friends(owner_id, profile_id, name, soulseek_users) VALUES ('owner', ?1, 'Alice', '[\"alice\"]')", [&id]).unwrap();
        authorize_target(&conn, "owner", &id, "alice").unwrap();
        assert!(authorize_target(&conn, "owner", &id, "stranger").is_err());
        assert!(authorize_target(&conn, "other", &id, "alice").is_err());
        conn.execute("UPDATE friends SET soulseek_users = '[]'", [])
            .unwrap();
        authorize_target(&conn, "owner", &id, "alice").unwrap();
    }
}
