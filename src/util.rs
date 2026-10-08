use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone};
use regex::Regex;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

use crate::args::args;
use crate::output::{RESET, YELLOW};
use crate::outln;

fn home_dir() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn expand_user(path: &str) -> PathBuf {
    if path == "~" {
        home_dir()
    } else if let Some(rest) = path.strip_prefix("~/") {
        home_dir().join(rest)
    } else {
        PathBuf::from(path)
    }
}

/// Lexically normalize an absolute version of `path` (like os.path.normpath).
fn normpath(path: &Path) -> PathBuf {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir().unwrap_or_default().join(path)
    };
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// Normalize a path, resolving symlinks where it exists (realpath + normpath).
pub fn norm_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| normpath(path))
}

fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    let base = env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(fallback));
    norm_path(&base.join("spotify-ripper"))
}

pub fn settings_dir() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config")
}

pub fn cache_dir() -> PathBuf {
    xdg_dir("XDG_CACHE_HOME", ".cache")
}

pub fn base_dir() -> PathBuf {
    norm_path(&expand_user(&args().directory))
}

/// Replace unwanted path characters.
pub fn sanitize_playlist_name(name: &str) -> String {
    name.replace(['\\', '/'], "-")
}

static ESC_SLASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*/\s*").unwrap());
static ESC_CHARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\s*[\\/:"*?<>|]+\s*"#).unwrap());
static ESC_LEADING_DOTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\.+\s*").unwrap());
static ESC_TRAILING_DOTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\.+$").unwrap());
static ESC_DOT_RUNS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.{2,}").unwrap());

/// Escape possible offending characters in a file name component.
pub fn escape_filename_part(part: &str) -> String {
    let part = ESC_SLASH.replace_all(part, " & ");
    let part = ESC_CHARS.replace_all(&part, " ");
    let part = part.trim();
    let part = ESC_LEADING_DOTS.replace(part, "");
    let part = ESC_TRAILING_DOTS.replace(&part, "");
    ESC_DOT_RUNS.replace_all(&part, ".").into_owned()
}

/// Convert to ASCII when --ascii is set; `replace` substitutes '?' for
/// unrepresentable characters instead of dropping them.
pub fn to_ascii_with(s: &str, replace: bool) -> String {
    if !args().ascii {
        return s.to_owned();
    }
    s.chars()
        .filter_map(|c| {
            if c.is_ascii() {
                Some(c)
            } else if replace {
                Some('?')
            } else {
                None
            }
        })
        .collect()
}

pub fn to_ascii(s: &str) -> String {
    to_ascii_with(s, false)
}

pub fn to_normalized_ascii(s: &str) -> String {
    s.nfkd().filter(char::is_ascii).collect()
}

pub fn rm_file(path: &Path) {
    if let Err(e) = fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        outln!(
            "{YELLOW}Warning: error while trying to remove file {}{RESET}",
            path.display()
        );
        outln!("{e}");
    }
}

/// Estimated download size in bytes of a track at the selected bitrate.
pub fn calc_file_size(duration_ms: u32) -> u64 {
    let kbps: u64 = args().quality.parse().unwrap_or(320);
    kbps * duration_ms as u64 / 8
}

static TIME_HHMM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{2}:\d{2}$").unwrap());
static TIME_OFFSET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:(\d+)h)?(?:(\d+)m)?$").unwrap());

/// Parse "HH:MM" (next occurrence) or an offset like "1h30m" / "45m" / "2h".
pub fn parse_time_str(s: &str) -> Option<DateTime<Local>> {
    let now = Local::now();
    if TIME_HHMM.is_match(s) {
        let t = NaiveTime::parse_from_str(s, "%H:%M").ok()?;
        let mut calc = Local
            .from_local_datetime(&now.date_naive().and_time(t))
            .earliest()?;
        if now > calc {
            calc += Duration::days(1);
        }
        return Some(calc);
    }
    let caps = TIME_OFFSET.captures(s)?;
    if caps.get(1).is_none() && caps.get(2).is_none() {
        return None;
    }
    let hours: i64 = caps.get(1).map_or(Ok(0), |m| m.as_str().parse()).ok()?;
    let minutes: i64 = caps.get(2).map_or(Ok(0), |m| m.as_str().parse()).ok()?;
    Some(now + Duration::hours(hours) + Duration::minutes(minutes))
}

pub fn change_file_extension(path: &Path, ext: &str) -> PathBuf {
    path.with_extension(ext)
}

/// Return the path of an executable found in $PATH.
pub fn which(program: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let is_exe = |p: &Path| {
        p.metadata()
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    };
    if program.contains('/') {
        let p = PathBuf::from(program);
        return is_exe(&p).then_some(p);
    }
    env::split_paths(&env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|p| is_exe(p))
}

const KB_BYTES: f64 = 1024.0;
const MB_BYTES: f64 = 1048576.0;
const GB_BYTES: f64 = 1073741824.0;

pub fn format_size(size: u64) -> String {
    let size = size as f64;
    if size >= GB_BYTES {
        format!("{:.2} GB", size / GB_BYTES)
    } else if size >= MB_BYTES {
        format!("{:.2} MB", size / MB_BYTES)
    } else if size >= KB_BYTES {
        format!("{:.2} KB", size / KB_BYTES)
    } else {
        format!("{size:.2} Bytes")
    }
}

fn time_str(seconds: u64) -> String {
    let (hours, mins, secs) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if hours > 0 {
        format!("{hours:02}:{mins:02}:{secs:02}")
    } else {
        format!("{mins:02}:{secs:02}")
    }
}

/// "MM:SS" (or "HH:MM:SS"), optionally followed by " / <total>".
pub fn format_time(seconds: u64, total: Option<u64>) -> String {
    match total {
        Some(total) if total > 0 => format!("{} / {}", time_str(seconds), time_str(total)),
        _ => time_str(seconds),
    }
}

/// Short 6-character form, e.g. "01h 05m" or "00m 15s".
pub fn format_time_short(seconds: u64) -> String {
    const UNITS: [(&str, u64); 6] = [
        ("y", 60 * 60 * 24 * 7 * 52),
        ("w", 60 * 60 * 24 * 7),
        ("d", 60 * 60 * 24),
        ("h", 60 * 60),
        ("m", 60),
        ("s", 1),
    ];
    if seconds < 60 {
        return format!("00m {seconds:02}s");
    }
    for pair in UNITS.windows(2) {
        let ((unit1, limit1), (unit2, limit2)) = (pair[0], pair[1]);
        if seconds >= limit1 {
            return format!(
                "{:02}{unit1} {:02}{unit2}",
                seconds / limit1,
                (seconds % limit1) / limit2
            );
        }
    }
    "  ~inf".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_file_name_parts() {
        assert_eq!(escape_filename_part("AC/DC"), "AC & DC");
        assert_eq!(escape_filename_part("What? Why: Now"), "What Why Now");
        assert_eq!(escape_filename_part("...Ready.."), "Ready");
        assert_eq!(escape_filename_part("A...B"), "A.B");
    }

    #[test]
    fn formats_sizes_and_times() {
        assert_eq!(format_size(512), "512.00 Bytes");
        assert_eq!(format_size(1536), "1.50 KB");
        assert_eq!(format_time(65, Some(3725)), "01:05 / 01:02:05");
        assert_eq!(format_time_short(15), "00m 15s");
        assert_eq!(format_time_short(3900), "01h 05m");
    }

    #[test]
    fn parses_time_offsets() {
        assert!(parse_time_str("1h30m").is_some());
        assert!(parse_time_str("45m").is_some());
        assert!(parse_time_str("2h").is_some());
        assert!(parse_time_str("03:30").is_some());
        assert!(parse_time_str("bogus").is_none());
        assert!(parse_time_str("").is_none());
    }

    #[test]
    fn normalizes_to_ascii() {
        assert_eq!(to_normalized_ascii("Motörhead café"), "Motorhead cafe");
    }
}
