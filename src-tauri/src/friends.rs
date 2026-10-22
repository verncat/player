use crate::{library::LibraryState, profile::LocalProfileState};
use rusqlite::{params, Connection};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
pub struct Friend {
    pub profile_id: String,
    name: String,
    pub soulseek_users: Vec<String>,
    resolved_at: Option<i64>,
}

pub(crate) fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    crate::private_messages::init_schema(conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS friends (
            owner_id TEXT NOT NULL,
            profile_id TEXT NOT NULL,
            name TEXT NOT NULL DEFAULT '',
            soulseek_users TEXT NOT NULL DEFAULT '[]',
            resolved_at INTEGER,
            PRIMARY KEY (owner_id, profile_id)
        );",
    )?;
    let columns = conn
        .prepare("PRAGMA table_info(friends)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.iter().any(|column| column == "soulseek_users") {
        conn.execute_batch(
            "ALTER TABLE friends ADD COLUMN soulseek_users TEXT NOT NULL DEFAULT '[]'",
        )?;
    }
    if !columns.iter().any(|column| column == "resolved_at") {
        conn.execute_batch("ALTER TABLE friends ADD COLUMN resolved_at INTEGER")?;
    }
    conn.execute_batch("CREATE TABLE IF NOT EXISTS soulseek_profile_markers (username TEXT PRIMARY KEY, marker TEXT NOT NULL)")
}

fn list(conn: &Connection, owner: &str) -> Result<Vec<Friend>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT profile_id, name, soulseek_users, resolved_at FROM friends WHERE owner_id = ?1
         ORDER BY name COLLATE NOCASE, profile_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([owner], |row| {
            Ok(Friend {
                profile_id: row.get(0)?,
                name: row.get(1)?,
                soulseek_users: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or_default(),
                resolved_at: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

pub(crate) fn find(conn: &Connection, owner: &str, profile_id: &str) -> Result<Friend, String> {
    use rusqlite::OptionalExtension;
    conn.query_row("SELECT profile_id, name, soulseek_users, resolved_at FROM friends WHERE owner_id = ?1 AND profile_id = ?2",
        params![owner, profile_id], |row| Ok(Friend {
            profile_id: row.get(0)?, name: row.get(1)?,
            soulseek_users: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or_default(), resolved_at: row.get(3)?,
        })).optional().map_err(|e| e.to_string())?.ok_or_else(|| "This friend is no longer in your list.".into())
}

fn add(conn: &Connection, owner: &str, profile_id: &str, name: &str) -> Result<(), String> {
    let id = Uuid::parse_str(profile_id.trim()).map_err(|_| "Enter a valid Profile ID.")?;
    if id.is_nil() {
        return Err("Enter a valid Profile ID.".into());
    }
    let id = id.to_string();
    if id == owner {
        return Err("You cannot add your own profile.".into());
    }
    let name = name.trim();
    if name.chars().count() > 100 {
        return Err("Use a name of up to 100 characters.".into());
    }
    let inserted = conn
        .execute(
            "INSERT INTO friends (owner_id, profile_id, name) VALUES (?1, ?2, ?3)
         ON CONFLICT(owner_id, profile_id) DO NOTHING",
            params![owner, id, name],
        )
        .map_err(|e| e.to_string())?;
    if inserted == 0 {
        return Err("This friend is already in your list.".into());
    }
    Ok(())
}

#[tauri::command]
pub fn get_friends(
    library: tauri::State<'_, LibraryState>,
    profile: tauri::State<'_, LocalProfileState>,
) -> Result<Vec<Friend>, String> {
    let owner = profile.info().user_id;
    let connection = library.conn();
    let conn = connection
        .lock()
        .map_err(|_| "Could not open the friends list.")?;
    list(&conn, &owner)
}

#[tauri::command]
pub fn add_friend(
    profile_id: String,
    name: String,
    library: tauri::State<'_, LibraryState>,
    profile: tauri::State<'_, LocalProfileState>,
) -> Result<(), String> {
    let owner = profile.info().user_id;
    let connection = library.conn();
    let conn = connection
        .lock()
        .map_err(|_| "Could not open the friends list.")?;
    add(&conn, &owner, &profile_id, &name)
}

#[tauri::command]
pub async fn resolve_friend(
    profile_id: String,
    library: tauri::State<'_, LibraryState>,
    profile: tauri::State<'_, LocalProfileState>,
    soulseek: tauri::State<'_, crate::soulseek::SoulseekState>,
) -> Result<(), String> {
    let owner = profile.info().user_id;
    let id = Uuid::parse_str(profile_id.trim())
        .map_err(|_| "Enter a valid Profile ID.")?
        .to_string();
    let marker = crate::soulseek::profile_interest(&id)?;
    {
        let connection = library.conn();
        let conn = connection
            .lock()
            .map_err(|_| "Could not open the friends list.")?;
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM friends WHERE owner_id = ?1 AND profile_id = ?2)",
                params![owner, id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !exists {
            return Err("This friend is no longer in your list.".into());
        }
    }
    let client = soulseek.discovery_client(&library, &profile).await?;
    let users = client
        .find_interest_users(&marker)
        .await
        .map_err(|e| e.to_string())?;
    if profile.info().user_id != owner {
        return Err("Your profile changed. Please try again.".into());
    }
    let connection = library.conn();
    let conn = connection
        .lock()
        .map_err(|_| "Could not save the Soulseek usernames.")?;
    conn.execute("UPDATE friends SET soulseek_users = ?3, resolved_at = strftime('%s','now') WHERE owner_id = ?1 AND profile_id = ?2",
        params![owner, id, serde_json::to_string(&users).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn remove_friend(
    profile_id: String,
    library: tauri::State<'_, LibraryState>,
    profile: tauri::State<'_, LocalProfileState>,
) -> Result<(), String> {
    let owner = profile.info().user_id;
    let connection = library.conn();
    let conn = connection
        .lock()
        .map_err(|_| "Could not open the friends list.")?;
    conn.execute(
        "DELETE FROM friends WHERE owner_id = ?1 AND profile_id = ?2",
        params![owner, profile_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_existing_contacts_and_stores_discovery_candidates() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE friends (owner_id TEXT, profile_id TEXT, name TEXT, PRIMARY KEY(owner_id, profile_id));
            INSERT INTO friends VALUES ('owner', 'friend', 'Alice');").unwrap();
        init_schema(&conn).unwrap();
        init_schema(&conn).unwrap();
        let initial = list(&conn, "owner").unwrap();
        assert!(initial[0].soulseek_users.is_empty());
        assert_eq!(initial[0].resolved_at, None);
        conn.execute(
            "UPDATE friends SET soulseek_users = ?1, resolved_at = 42",
            [r#"["alice","second-device"]"#],
        )
        .unwrap();
        let updated = list(&conn, "owner").unwrap();
        assert_eq!(updated[0].name, "Alice");
        assert_eq!(updated[0].soulseek_users, vec!["alice", "second-device"]);
        assert_eq!(updated[0].resolved_at, Some(42));
    }

    #[test]
    fn marker_uses_only_the_canonical_public_profile_id() {
        let id = Uuid::new_v4();
        assert_eq!(
            crate::soulseek::profile_interest(&format!(" {} ", id.to_string().to_uppercase()))
                .unwrap(),
            format!("adhdplayer-v1-{id}")
        );
        assert!(crate::soulseek::profile_interest("invalid").is_err());
        assert!(crate::soulseek::profile_interest(&Uuid::nil().to_string()).is_err());
    }

    #[test]
    fn validates_identity_and_does_not_replace_existing_friends() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let owner = Uuid::new_v4().to_string();
        let friend = Uuid::new_v4().to_string();
        assert!(add(&conn, &owner, "not-an-id", "").is_err());
        assert!(add(&conn, &owner, &Uuid::nil().to_string(), "").is_err());
        assert!(add(&conn, &owner, &owner, "").is_err());
        assert!(add(&conn, &owner, &friend, &"a".repeat(101)).is_err());
        add(
            &conn,
            &owner,
            &format!(" {} ", friend.to_uppercase()),
            " Alice ",
        )
        .unwrap();
        assert!(add(&conn, &owner, &friend, "Replacement").is_err());
        let friends = list(&conn, &owner).unwrap();
        assert_eq!(friends.len(), 1);
        assert_eq!(friends[0].profile_id, friend);
        assert_eq!(friends[0].name, "Alice");
    }

    #[test]
    fn persists_contacts_and_isolates_profiles() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("friends.db");
        let owner = Uuid::new_v4().to_string();
        let other = Uuid::new_v4().to_string();
        let friend = Uuid::new_v4().to_string();
        {
            let conn = Connection::open(&path).unwrap();
            init_schema(&conn).unwrap();
            add(&conn, &owner, &friend, "Alice").unwrap();
            assert!(list(&conn, &other).unwrap().is_empty());
            add(&conn, &other, &friend, "Different name").unwrap();
        }
        let conn = Connection::open(path).unwrap();
        init_schema(&conn).unwrap();
        assert_eq!(list(&conn, &owner).unwrap()[0].name, "Alice");
        conn.execute(
            "DELETE FROM friends WHERE owner_id = ?1 AND profile_id = ?2",
            params![other, friend],
        )
        .unwrap();
        assert!(list(&conn, &other).unwrap().is_empty());
        assert_eq!(list(&conn, &owner).unwrap().len(), 1);
    }
}
