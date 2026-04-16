with open("src-tauri/src/lib.rs", "r") as f:
    content = f.read()

content = content.replace("library::update_track,", "library::update_track,\n            library::toggle_like,")

with open("src-tauri/src/lib.rs", "w") as f:
    f.write(content)
