import re

with open("src-tauri/src/library.rs", "r") as f:
    content = f.read()

# 1. Update Track struct
content = content.replace(
    "pub manually_edited: bool,\n}",
    "pub manually_edited: bool,\n    pub is_liked: bool,\n    pub play_count: i64,\n}"
)

# 2. Update SELECT statements
select_old = "SELECT id, path, title, artist, album, track_number, duration_secs, file_hash, rarity, manually_edited"
select_new = "SELECT id, path, title, artist, album, track_number, duration_secs, file_hash, rarity, manually_edited, is_liked, play_count"
content = content.replace(select_old, select_new)

select_recent_old = "SELECT DISTINCT t.id, t.path, t.title, t.artist, t.album, t.track_number,\n                t.duration_secs, t.file_hash, t.rarity, t.manually_edited"
select_recent_new = "SELECT DISTINCT t.id, t.path, t.title, t.artist, t.album, t.track_number,\n                t.duration_secs, t.file_hash, t.rarity, t.manually_edited, t.is_liked, t.play_count"
content = content.replace(select_recent_old, select_recent_new)

# 3. Update row_to_track
row_old = """        manually_edited: row.get::<_, i64>(9).unwrap_or(0) != 0,
    })"""
row_new = """        manually_edited: row.get::<_, i64>(9).unwrap_or(0) != 0,
        is_liked: row.get::<_, i64>(10).unwrap_or(0) != 0,
        play_count: row.get::<_, i64>(11).unwrap_or(0),
    })"""
content = content.replace(row_old, row_new)

# 4. Update init_schema
schema_old = """    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN manually_edited INTEGER NOT NULL DEFAULT 0");"""
schema_new = """    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN manually_edited INTEGER NOT NULL DEFAULT 0");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN is_liked INTEGER NOT NULL DEFAULT 0");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN play_count INTEGER NOT NULL DEFAULT 0");"""
content = content.replace(schema_old, schema_new)

# 5. Add toggle_like command
toggle_cmd = """
#[tauri::command]
pub fn toggle_like(id: i64, state: tauri::State<'_, LibraryState>) -> Result<bool, String> {
    let conn = state.conn.lock().unwrap();
    let current_liked: i64 = conn.query_row(
        "SELECT is_liked FROM tracks WHERE id = ?1",
        rusqlite::params![id],
        |row| row.get(0),
    ).unwrap_or(0);
    
    let new_liked = if current_liked == 0 { 1 } else { 0 };
    
    conn.execute(
        "UPDATE tracks SET is_liked = ?1 WHERE id = ?2",
        rusqlite::params![new_liked, id],
    ).map_err(|e| e.to_string())?;
    
    Ok(new_liked != 0)
}
"""
content += toggle_cmd

# 6. Increment play_count in record_play
update_play_old = "    .map_err(|e| e.to_string())?;\n    Ok(())"
update_play_new = """    .map_err(|e| e.to_string())?;
    let _ = conn.execute(
        "UPDATE tracks SET play_count = play_count + 1 WHERE id = ?1",
        rusqlite::params![id],
    ).map_err(|e| e.to_string())?;
    Ok(())"""
content = content.replace(update_play_old, update_play_new)

with open("src-tauri/src/library.rs", "w") as f:
    f.write(content)

