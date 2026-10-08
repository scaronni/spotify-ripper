// ~/.config/spotify-ripper/config.json: the default configuration merged with
// the user's file. Keys match the long option names with dashes turned into
// underscores.

use serde_json::{Map, Value, json};
use std::fs;
use std::io;
use std::path::Path;

use crate::util::settings_dir;

pub fn default_config() -> Map<String, Value> {
    let config = json!({
        // output location and file naming
        "directory": "~/Music",
        "format": null,
        "format_case": null,
        "ascii": false,
        "ascii_path_only": false,
        "normalized_ascii": false,
        "windows_safe": false,
        "overwrite": false,
        "replace": null,
        // Spotify stream quality and encoder settings
        "quality": "320",
        "cbr": false,
        "bitrate": "320",
        "vbr": "0",
        "comp": "10",
        "stereo_mode": null,
        // metadata
        "all_artists": false,
        "id3_v23": false,
        "large_cover_art": false,
        "comment": null,
        "grouping": null,
        "cover_file": null,
        "cover_file_and_embed": null,
        // playlists
        "playlist_sync": false,
        "playlist_m3u": false,
        "playlist_wpl": false,
        // artist filtering
        "artist_album_type": null,
        // rate-limit handling
        "retries": 5,
        "delay": 0,
        // misc
        "partial_check": "weak",
        "plus_pcm": false,
        "plus_wav": false,
        "keep_offline_cache": false,
        "fail_log": null,
    });
    match config {
        Value::Object(map) => map,
        _ => unreachable!(),
    }
}

/// Serialize like Python's json.dump(indent=4, sort_keys=True, ensure_ascii=False).
pub fn to_pretty_json(value: &Value) -> String {
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    serde::Serialize::serialize(value, &mut ser).expect("serializing JSON");
    let mut s = String::from_utf8(buf).expect("JSON is UTF-8");
    s.push('\n');
    s
}

pub fn write_json(path: &Path, value: &Value) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, to_pretty_json(value))
}

/// Return the default configuration merged with the user's config.json.
///
/// On first run (no config.json yet) the default configuration is written out
/// so the user has a documented, editable starting point.
pub fn load_config() -> Map<String, Value> {
    let path = settings_dir().join("config.json");
    let mut config = default_config();

    if path.exists() {
        match fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str::<Value>(&s).map_err(|e| e.to_string()))
        {
            Ok(Value::Object(user)) => config.extend(user),
            Ok(_) => {}
            Err(e) => {
                println!("\nError parsing config file: {}", path.display());
                println!("{e}");
            }
        }
    } else if let Err(e) = write_json(&path, &Value::Object(default_config())) {
        println!("\nCould not write default config file: {}", path.display());
        println!("{e}");
    }

    config
}
