use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::args::args;
use crate::output::{BOLD, DIM, ORANGE, RED, RESET};
use crate::util::{
    base_dir, cache_dir, change_file_extension, format_duration, rm_file, sanitize_playlist_name,
    to_ascii,
};
use crate::{outln, warn};

/// A track as listed in the summary: (URI, "Artist - Title").
type Entry = (String, String);

pub struct PostActions {
    success: Vec<Entry>,
    skipped: Vec<Entry>,
    unavailable: Vec<Entry>,
    failure: Vec<Entry>,
    fail_log: Option<(PathBuf, File)>,
}

/// Path of `path` relative to the output directory.
pub fn rel_path(path: &Path) -> String {
    let base = base_dir();
    path.strip_prefix(&base)
        .unwrap_or(path)
        .display()
        .to_string()
}

impl PostActions {
    pub fn new() -> Self {
        let a = args();
        let mut fail_log = None;
        if let Some(name) = &a.fail_log {
            let path = base_dir().join(name);
            let opened = fs::create_dir_all(base_dir()).and_then(|_| File::create(&path));
            match opened {
                Ok(f) => fail_log = Some((path, f)),
                Err(e) => warn!("cannot create fail log: {e}"),
            }
        }
        Self {
            success: Vec::new(),
            skipped: Vec::new(),
            unavailable: Vec::new(),
            failure: Vec::new(),
            fail_log,
        }
    }

    pub fn log_success(&mut self, entry: Entry) {
        self.success.push(entry);
    }

    pub fn log_skipped(&mut self, entry: Entry) {
        self.skipped.push(entry);
    }

    /// Not available in the user's region; also listed in the fail log.
    pub fn log_unavailable(&mut self, entry: Entry) {
        if let Some((_, f)) = &mut self.fail_log {
            let _ = writeln!(f, "{}", entry.0);
        }
        self.unavailable.push(entry);
    }

    pub fn log_failure(&mut self, entry: Entry) {
        if let Some((_, f)) = &mut self.fail_log {
            let _ = writeln!(f, "{}", entry.0);
        }
        self.failure.push(entry);
    }

    pub fn end_failure_log(&mut self) {
        if let Some((path, f)) = self.fail_log.take() {
            let _ = f.sync_all();
            drop(f);
            if fs::metadata(&path).is_ok_and(|m| m.len() == 0) {
                rm_file(&path);
            }
        }
    }

    /// One summary line; with --verbose also the tracks that didn't make it.
    pub fn print_summary(&self, elapsed: Duration) {
        let (ripped, skipped, unavailable, failed) = (
            self.success.len(),
            self.skipped.len(),
            self.unavailable.len(),
            self.failure.len(),
        );
        if ripped + skipped + unavailable + failed == 0 {
            return;
        }
        outln!(
            "{BOLD}Done in {}:{RESET} {ripped} ripped, {skipped} skipped, {unavailable} unavailable, {failed} failed",
            format_duration(elapsed.as_secs())
        );

        if args().verbose {
            let bullet = if args().ascii { " * " } else { " • " };
            let print_list = |color: &str, title: &str, entries: &[Entry]| {
                if !entries.is_empty() {
                    outln!("{color}{title}:{RESET}");
                    for (uri, name) in entries {
                        outln!(
                            "{bullet}{} {DIM}{uri}{RESET}",
                            if name.is_empty() { uri } else { name }
                        );
                    }
                }
            };
            print_list(ORANGE, "Unavailable tracks", &self.unavailable);
            print_list(RED, "Failed tracks", &self.failure);
        }
    }

    fn playlist_path(name: &str, ext: &str) -> PathBuf {
        let name = sanitize_playlist_name(&to_ascii(name));
        PathBuf::from(to_ascii(
            &base_dir().join(format!("{name}.{ext}")).to_string_lossy(),
        ))
    }

    /// `name` is the playlist name, or "artist - album" for an album URI.
    pub fn create_playlist_m3u(&self, name: Option<&str>, files: &[PathBuf]) {
        if !args().playlist_m3u {
            return;
        }
        let path = Self::playlist_path(name.unwrap_or("0_playlist"), "m3u");
        let content: String = files
            .iter()
            .filter(|f| f.exists())
            .map(|f| rel_path(f) + "\n")
            .collect();
        match fs::write(&path, content) {
            Ok(()) => outln!("Created playlist {}", path.display()),
            Err(e) => warn!("cannot write {}: {e}", path.display()),
        }
    }

    pub fn create_playlist_wpl(&self, name: Option<&str>, files: &[PathBuf], user: &str) {
        let Some(name) = name.filter(|_| args().playlist_wpl) else {
            return;
        };
        let path = Self::playlist_path(name, "wpl");

        let escape = |s: &str| {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&apos;")
        };
        let files: Vec<_> = files.iter().filter(|f| f.exists()).collect();
        let mut s = String::new();
        s.push_str("<?wpl version=\"1.0\"?>\n<smil>\n\t<head>\n");
        s.push_str("\t\t<meta name=\"Generator\" content=\"Microsoft Windows Media Player -- 12.0.7601.18526\"/>\n");
        s.push_str(&format!(
            "\t\t<meta name=\"ItemCount\" content=\"{}\"/>\n",
            files.len()
        ));
        s.push_str(&format!("\t\t<author>{}</author>\n", escape(user)));
        s.push_str(&format!(
            "\t\t<title>{}</title>\n",
            escape(&sanitize_playlist_name(&to_ascii(name)))
        ));
        s.push_str("\t</head>\n\t<body>\n\t\t<seq>\n");
        for f in files {
            s.push_str(&format!(
                "\t\t\t<media src=\"{}\"/>\n",
                escape(&rel_path(f))
            ));
        }
        s.push_str("\t\t</seq>\n\t</body>\n</smil>\n");
        match fs::write(&path, s) {
            Ok(()) => outln!("Created playlist {}", path.display()),
            Err(e) => warn!("cannot write {}: {e}", path.display()),
        }
    }

    pub fn clean_up_partial(&self, audio_file: &Path) {
        if audio_file.exists() {
            rm_file(audio_file);
        }
        // check for any extra pcm or wav files
        let a = args();
        for (enabled, ext) in [(a.plus_wav, "wav"), (a.plus_pcm, "pcm")] {
            let extra = change_file_extension(audio_file, ext);
            if enabled && extra != audio_file && extra.exists() {
                rm_file(&extra);
            }
        }
    }

    /// Delete librespot's offline audio cache unless asked to keep it.
    pub fn cleanup_offline_cache(&self) {
        if !args().keep_offline_cache {
            let _ = fs::remove_dir_all(cache_dir());
        }
    }
}
