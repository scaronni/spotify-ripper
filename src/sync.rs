// --playlist-sync: remember where each track of a playlist was ripped to, so
// files can be renamed or removed when the playlist changes.

use serde_json::{Map, Value};
use std::fs;
use std::path::PathBuf;

use crate::spotify::PlaylistInfo;
use crate::util::{settings_dir, to_ascii};
use crate::{outln, warn};

fn sync_lib_path(playlist: &PlaylistInfo) -> PathBuf {
    let lib_path = settings_dir().join("Sync");
    let _ = fs::create_dir_all(&lib_path);
    lib_path.join(format!("{}.json", playlist.id))
}

fn load_sync_library(playlist: &PlaylistInfo) -> Map<String, Value> {
    fs::read_to_string(sync_lib_path(playlist))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default()
}

fn save_sync_library(playlist: &PlaylistInfo, lib: &[(String, String)]) {
    // keep the playlist order, as Python's json.dumps(indent=4) did
    let mut s = String::from("{");
    for (i, (uri, path)) in lib.iter().enumerate() {
        s.push_str(if i == 0 { "\n    " } else { ",\n    " });
        s.push_str(&format!(
            "{}: {}",
            Value::from(uri.as_str()),
            Value::from(path.as_str())
        ));
    }
    s.push_str(if lib.is_empty() { "}" } else { "\n}" });
    if let Err(e) = fs::write(sync_lib_path(playlist), s) {
        warn!("cannot save playlist sync library: {e}");
    }
}

/// `new_lib`: (track URI, file path) for the current playlist contents.
pub fn sync_playlist(playlist: &PlaylistInfo, new_lib: &[(String, String)]) {
    let lib = load_sync_library(playlist);
    outln!("Syncing playlist {}", to_ascii(&playlist.name));

    // check what items are missing or renamed in the new_lib vs lib
    for (uri, file_path) in &lib {
        let Some(file_path) = file_path.as_str() else {
            continue;
        };
        if !fs::exists(file_path).unwrap_or(false) {
            continue;
        }
        match new_lib.iter().find(|(u, _)| u == uri) {
            Some((_, new_path)) if new_path != file_path => match fs::rename(file_path, new_path) {
                Ok(()) => outln!("Renamed {file_path} -> {new_path}"),
                Err(e) => warn!("cannot rename {file_path}: {e}"),
            },
            Some(_) => {}
            None => match fs::remove_file(file_path) {
                Ok(()) => outln!("Removed {file_path}"),
                Err(e) => warn!("cannot remove {file_path}: {e}"),
            },
        }
    }

    save_sync_library(playlist, new_lib);
}
