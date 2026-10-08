// Format strings (--format, --comment, --grouping), see README.

use chrono::{Local, TimeZone};
use regex::Regex;
use std::sync::LazyLock;

use crate::args::args;
use crate::spotify::{AlbumInfo, PlaylistInfo, TrackInfo};
use crate::util::{escape_filename_part, sanitize_playlist_name, to_ascii};

/// What is being ripped: the playlist or album URI the track came from.
#[derive(Clone, Copy, Default)]
pub struct Context<'a> {
    pub playlist: Option<&'a PlaylistInfo>,
    pub album: Option<&'a AlbumInfo>,
    pub user: &'a str,
}

static PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.*)\s+-\s+([^-]+)").unwrap());

fn zfill(s: &str, width: usize) -> String {
    format!("{s:0>width$}")
}

pub fn format_track_string(ctx: &Context, format_string: &str, idx: usize, track: &TrackInfo) -> String {
    let a = args();
    let album = ctx.album.unwrap_or(&track.album);
    let esc = |s: &str| to_ascii(&escape_filename_part(s));

    let track_artist = esc(&track.artist_name());
    let track_artists = esc(&track.artist_names());
    let featuring_artists = if track.artists.len() > 1 {
        esc(&track.artists[1..]
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", "))
    } else {
        String::new()
    };
    let album_artist = esc(&album.artist_name());
    let album_artists_web = if album.artists.is_empty() {
        track_artists.clone()
    } else {
        esc(&album
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", "))
    };
    let album_name = esc(&track.album.name);
    let track_name = esc(&track.name);
    let year = track.album.year.to_string();
    let extension = a.output_type.clone();
    let idx_str = (idx + 1).to_string();
    let track_num = track.number.to_string();
    let disc_num = track.disc.to_string();
    let track_uri = track.uri();

    let smart_num = if track.album.num_discs() >= 2 {
        (track.disc * 100 + track.number).to_string()
    } else {
        track_num.clone()
    };

    let (playlist_name, playlist_owner) = match ctx.playlist {
        Some(p) => (
            to_ascii(&sanitize_playlist_name(&p.name)),
            to_ascii(&p.owner),
        ),
        None => ("No Playlist".to_owned(), "No Playlist Owner".to_owned()),
    };
    let user = ctx.user.to_owned();

    let copyright = track
        .album
        .copyrights
        .first()
        .map(|c| escape_filename_part(c))
        .unwrap_or_default();
    let label = Regex::new(r"^[0-9]+\s+")
        .unwrap()
        .replace(&copyright, "")
        .into_owned();

    let (create_time, creator) = ctx
        .playlist
        .and_then(|p| p.added.get(&track_uri))
        .map(|(time, user)| {
            let time = Local
                .timestamp_opt(*time, 0)
                .single()
                .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_default();
            (time, user.clone())
        })
        .unwrap_or_default();

    let tags: Vec<(&str, String)> = vec![
        ("track_artist", track_artist.clone()),
        ("track_artists", track_artists.clone()),
        ("album_artist", album_artist),
        ("album_artists_web", album_artists_web),
        ("artist", track_artist),
        ("artists", track_artists),
        ("album", album_name),
        ("track_name", track_name.clone()),
        ("track", track_name),
        ("year", year),
        ("ext", extension.clone()),
        ("extension", extension),
        ("idx", idx_str.clone()),
        ("index", idx_str),
        ("track_num", track_num.clone()),
        ("track_idx", track_num.clone()),
        ("track_index", track_num),
        ("disc_num", disc_num.clone()),
        ("disc_idx", disc_num.clone()),
        ("disc_index", disc_num),
        ("smart_track_num", smart_num.clone()),
        ("smart_track_idx", smart_num.clone()),
        ("smart_track_index", smart_num),
        ("playlist", playlist_name.clone()),
        ("playlist_name", playlist_name),
        ("playlist_owner", playlist_owner.clone()),
        ("playlist_user", playlist_owner.clone()),
        ("playlist_username", playlist_owner),
        ("user", user.clone()),
        ("username", user),
        ("feat_artists", featuring_artists.clone()),
        ("featuring_artists", featuring_artists),
        ("copyright", copyright),
        ("label", label.clone()),
        ("copyright_holder", label),
        ("playlist_track_add_time", create_time.clone()),
        ("track_add_time", create_time),
        ("playlist_track_add_user", creator.clone()),
        ("track_add_user", creator),
        ("track_uri", track_uri.clone()),
        ("uri", track_uri),
    ];
    const FILL_TAGS: [&str; 11] = [
        "idx", "index", "track_num", "track_idx", "track_index", "disc_num", "disc_idx",
        "disc_index", "smart_track_num", "smart_track_idx", "smart_track_index",
    ];
    const PREFIX_TAGS: [&str; 2] = ["feat_artists", "featuring_artists"];
    const PAREN_TAGS: [&str; 2] = ["track_name", "track"];

    let mut s = format_string.to_owned();
    for (tag, value) in &tags {
        s = s.replace(&format!("{{{tag}}}"), value);

        if FILL_TAGS.contains(tag) {
            let re = Regex::new(&format!(r"\{{{tag}:(\d+)\}}")).unwrap();
            if let Some(c) = re.captures(&s) {
                let m = c.get(0).unwrap();
                let width: usize = c[1].parse().unwrap_or(0);
                s = format!("{}{}{}", &s[..m.start()], zfill(value, width), &s[m.end()..]);
            }
        }

        if PREFIX_TAGS.contains(tag) {
            // don't print the prefix if there are no values
            if !value.is_empty() {
                let re = Regex::new(&format!(r"\{{{tag}:([^}}]+)\}}")).unwrap();
                if let Some(c) = re.captures(&s) {
                    let m = c.get(0).unwrap();
                    s = format!("{}{} {value}{}", &s[..m.start()], &c[1], &s[m.end()..]);
                }
            } else {
                let re = Regex::new(&format!(r"\s*\{{{tag}:[^}}]+\}}")).unwrap();
                if let Some(m) = re.find(&s) {
                    s = format!("{}{}", &s[..m.start()], &s[m.end()..]);
                }
            }
        }

        if PAREN_TAGS.contains(tag) {
            let pattern = format!("{{{tag}:paren}}");
            if let Some(start) = s.find(&pattern) {
                let replacement = match PAREN.captures(value) {
                    Some(c) => format!("{} ({})", &c[1], &c[2]),
                    None => value.clone(),
                };
                s = format!("{}{replacement}{}", &s[..start], &s[start + pattern.len()..]);
            }
        }
    }

    match a.format_case.as_deref() {
        Some("upper") => s.to_uppercase(),
        Some("lower") => s.to_lowercase(),
        Some("capitalize") => s
            .split_whitespace()
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => s,
    }
}

/// Convert a Python re.sub() replacement ("\1") into Rust regex syntax ("${1}").
pub fn python_replacement(repl: &str) -> String {
    let mut out = String::new();
    let mut chars = repl.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '$' => out.push_str("$$"),
            '\\' => match chars.peek() {
                Some(d) if d.is_ascii_digit() => {
                    let mut num = String::new();
                    while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                        num.push(*d);
                        chars.next();
                    }
                    out.push_str(&format!("${{{num}}}"));
                }
                Some('\\') => {
                    out.push('\\');
                    chars.next();
                }
                _ => out.push('\\'),
            },
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_python_replacements() {
        assert_eq!(python_replacement(r"\1-\2"), "${1}-${2}");
        assert_eq!(python_replacement("_"), "_");
        assert_eq!(python_replacement("$"), "$$");
    }

    #[test]
    fn zero_fills() {
        assert_eq!(zfill("7", 3), "007");
        assert_eq!(zfill("123", 2), "123");
    }
}
