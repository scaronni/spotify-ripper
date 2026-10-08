use chrono::{DateTime, Local};
use regex::Regex;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use crate::args::args;
use crate::format::{Context, format_track_string, python_replacement};
use crate::output::{DIM, GREEN, ORANGE, RED, RESET, format_field};
use crate::post_actions::{PostActions, rel_path};
use crate::spotify::{AlbumInfo, PlaylistInfo, Spotify, TrackInfo};
use crate::tags::{file_duration, set_metadata_tags};
use crate::terminal::{RIPPING, SKIP, aborted, progress, skipped};
use crate::util::{
    base_dir, calc_file_size, change_file_extension, format_size, format_time, parse_time_str,
    rm_file, to_ascii, to_normalized_ascii,
};
use crate::{outln, warn};

// raw PCM decoded from the Ogg Vorbis stream is signed 16-bit stereo @ 44100 Hz
const PCM_SAMPLE_RATE: u32 = 44100;
const PCM_FRAME_BYTES: usize = 4;
const DOWNLOAD_CHUNK: usize = 64 * 1024;
const DECODE_CHUNK: usize = 16 * 1024;

static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"open\.spotify\.com/(?:intl-[a-z]+/)?(track|album|playlist|artist)/([A-Za-z0-9]+)")
        .unwrap()
});

/// Accept either a spotify: URI or an open.spotify.com URL and return the
/// canonical "spotify:<type>:<id>" URI; unknown inputs are returned as-is.
pub fn normalize_uri(uri: &str) -> String {
    let uri = uri.trim();
    if uri.starts_with("spotify:") {
        return uri.split('?').next().unwrap_or_default().to_owned();
    }
    match URL_RE.captures(uri) {
        Some(c) => format!("spotify:{}:{}", &c[1], &c[2]),
        None => uri.to_owned(),
    }
}

fn uri_to_id(uri: &str) -> &str {
    uri.rsplit(':').next().unwrap_or_default()
}

/// The outcome of a track, the first word of its line.
#[derive(Clone, Copy)]
enum Status {
    Ripped,
    Skipped,
    Unavailable,
    Failed,
}

/// One greppable line per track: "[ N/M] status  what · detail".
fn print_status(prefix: &str, status: Status, what: &str, detail: &str) {
    let sep = if args().ascii { " - " } else { " · " };
    let (word, color) = match status {
        Status::Ripped => ("ripped", GREEN),
        Status::Skipped => ("skipped", DIM),
        Status::Unavailable => ("unavailable", ORANGE),
        Status::Failed => ("failed", RED),
    };
    let detail = if detail.is_empty() {
        String::new()
    } else {
        format!("{DIM}{sep}{detail}{RESET}")
    };
    match status {
        Status::Skipped => outln!("{DIM}{prefix}{word:<11} {what}{RESET}{detail}"),
        _ => outln!("{prefix}{color}{word:<11}{RESET} {what}{detail}"),
    }
}

/// "Artist, Other Artist - Title" for the track lines and bars.
fn track_name(track: &TrackInfo) -> String {
    to_ascii(&format!("{} - {}", track.artist_names(), track.name))
}

/// First line of an error, for the one-line track output.
fn short_error(e: &dyn std::fmt::Display) -> String {
    e.to_string().lines().next().unwrap_or_default().to_owned()
}

fn summary_entry(track: &TrackInfo) -> (String, String) {
    let name = if track.artist_name().is_empty() || track.name.is_empty() {
        String::new()
    } else {
        format!("{} - {}", track.artist_name(), track.name)
    };
    (track.uri(), name)
}

/// The tracks of one URI given on the command line, with its context.
struct Job {
    tracks: Vec<TrackInfo>,
    playlist: Option<PlaylistInfo>,
    album: Option<Arc<AlbumInfo>>,
}

impl Job {
    fn context<'a>(&'a self, user: &'a str) -> Context<'a> {
        Context {
            playlist: self.playlist.as_ref(),
            album: self.album.as_deref(),
            user,
        }
    }

    /// Playlist name for m3u/wpl files.
    fn playlist_name(&self) -> Option<String> {
        if let Some(p) = &self.playlist {
            Some(p.name.clone())
        } else {
            self.album
                .as_ref()
                .map(|a| format!("{} - {}", a.artist_name(), a.name))
        }
    }
}

/// Minimal streaming WAV writer, the header is completed when finished.
struct WavWriter {
    file: BufWriter<File>,
    data_len: u32,
}

impl WavWriter {
    fn create(path: &Path) -> io::Result<Self> {
        let mut file = BufWriter::new(File::create(path)?);
        let byte_rate = PCM_SAMPLE_RATE * PCM_FRAME_BYTES as u32;
        file.write_all(b"RIFF\0\0\0\0WAVEfmt ")?;
        file.write_all(&16u32.to_le_bytes())?;
        file.write_all(&1u16.to_le_bytes())?; // PCM
        file.write_all(&2u16.to_le_bytes())?; // channels
        file.write_all(&PCM_SAMPLE_RATE.to_le_bytes())?;
        file.write_all(&byte_rate.to_le_bytes())?;
        file.write_all(&(PCM_FRAME_BYTES as u16).to_le_bytes())?;
        file.write_all(&16u16.to_le_bytes())?; // bits per sample
        file.write_all(b"data\0\0\0\0")?;
        Ok(Self { file, data_len: 0 })
    }

    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        self.data_len += data.len() as u32;
        self.file.write_all(data)
    }

    fn finish(mut self) -> io::Result<()> {
        let mut file = self.file.into_inner().map_err(|e| e.into_error())?;
        file.seek(SeekFrom::Start(4))?;
        file.write_all(&(36 + self.data_len).to_le_bytes())?;
        file.seek(SeekFrom::Start(40))?;
        file.write_all(&self.data_len.to_le_bytes())?;
        self.data_len = 0;
        file.sync_all()
    }
}

/// Encoder process plus the optional WAV and raw PCM outputs.
#[derive(Default)]
struct Sinks {
    encoder: Option<(Child, ChildStdin)>,
    wav: Option<WavWriter>,
    pcm: Option<BufWriter<File>>,
}

impl Sinks {
    fn needs_pcm(&self) -> bool {
        self.encoder.is_some() || self.wav.is_some() || self.pcm.is_some()
    }

    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        if let Some((_, stdin)) = &mut self.encoder {
            stdin.write_all(data)?;
        }
        if let Some(wav) = &mut self.wav {
            wav.write(data)?;
        }
        if let Some(pcm) = &mut self.pcm {
            pcm.write_all(data)?;
        }
        Ok(())
    }

    fn finish(&mut self) -> io::Result<()> {
        if let Some((mut child, stdin)) = self.encoder.take() {
            drop(stdin);
            let status = child.wait()?;
            if !status.success() {
                warn!(
                    "encoder returned non-zero error code {}",
                    status.code().unwrap_or(-1)
                );
            }
        }
        if let Some(wav) = self.wav.take() {
            wav.finish()?;
        }
        if let Some(mut pcm) = self.pcm.take() {
            pcm.flush()?;
            pcm.get_ref().sync_all()?;
        }
        Ok(())
    }

    /// Tear down without finalizing (skipped or aborted track).
    fn abort(&mut self) {
        if let Some((mut child, stdin)) = self.encoder.take() {
            drop(stdin);
            let _ = child.kill();
            let _ = child.wait();
        }
        self.wav = None;
        self.pcm = None;
    }
}

fn encoder_command(audio_file: &Path) -> Option<Command> {
    let a = args();
    let rate = PCM_SAMPLE_RATE.to_string();
    let file = audio_file.as_os_str();
    let mut cmd;
    match a.output_type.as_str() {
        "flac" => {
            cmd = Command::new("flac");
            cmd.args([
                "-f",
                &format!("-{}", a.comp),
                "--silent",
                "--endian",
                "little",
            ])
            .args(["--channels", "2", "--bps", "16", "--sample-rate", &rate])
            .args(["--sign", "signed", "-o"])
            .arg(file)
            .arg("-");
        }
        "aiff" => {
            cmd = Command::new("sox");
            cmd.args([
                "-q",
                "--endian",
                "little",
                "--channels",
                "2",
                "--bits",
                "16",
            ])
            .args([
                "--rate",
                &rate,
                "--encoding",
                "signed-integer",
                "-t",
                "raw",
                "-",
            ])
            .arg(file);
        }
        "alac.m4a" => {
            cmd = Command::new("ffmpeg");
            cmd.args(["-nostats", "-loglevel", "0", "-f", "s16le", "-ar", &rate])
                .args([
                    "-ac",
                    "2",
                    "-channel_layout",
                    "stereo",
                    "-i",
                    "-",
                    "-acodec",
                    "alac",
                ])
                .arg(file);
        }
        "opus" => {
            cmd = Command::new("opusenc");
            cmd.args(["--quiet", "--comp", &a.comp]);
            if a.cbr {
                let bitrate = a.bitrate.parse::<u32>().unwrap_or(320) / 2;
                cmd.args(["--cvbr", "--bitrate", &bitrate.to_string()]);
            } else {
                cmd.args(["--vbr", "--bitrate", &a.vbr]);
            }
            cmd.args(["--raw", "--raw-rate", &rate, "-"]).arg(file);
        }
        "m4a" => {
            cmd = Command::new("fdkaac");
            if a.cbr {
                cmd.args(["-S", "-R", "-b", &a.bitrate, "-o"]);
            } else {
                cmd.args(["-S", "-R", "-m", &a.vbr, "-o"]);
            }
            cmd.arg(file).arg("-");
        }
        "mp3" => {
            cmd = Command::new("lame");
            cmd.arg("--silent");
            if let Some(mode) = &a.stereo_mode {
                cmd.args(["-m", mode]);
            }
            if a.cbr {
                cmd.args(["-cbr", "-b", &a.bitrate]);
            } else {
                cmd.args(["-V", &a.vbr]);
            }
            cmd.args(["-h", "-r", "-"]).arg(file);
        }
        _ => return None,
    }
    Some(cmd)
}

fn prepare_sinks(audio_file: &Path) -> io::Result<Sinks> {
    let a = args();
    let mut sinks = Sinks::default();

    if a.output_type == "wav" || a.plus_wav {
        sinks.wav = Some(WavWriter::create(&change_file_extension(
            audio_file, "wav",
        ))?);
    }
    if a.output_type == "pcm" || a.plus_pcm {
        let path = change_file_extension(audio_file, "pcm");
        sinks.pcm = Some(BufWriter::new(File::create(path)?));
    }
    if let Some(mut cmd) = encoder_command(audio_file) {
        let mut child = cmd.stdin(Stdio::piped()).spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        sinks.encoder = Some((child, stdin));
    }
    Ok(sinks)
}

pub struct Ripper {
    spotify: Arc<Spotify>,
    user: String,
    kbps: u32,
    post: PostActions,
    path_cache: HashMap<String, PathBuf>,
    stop_time: Option<DateTime<Local>>,
}

impl Ripper {
    pub fn new(spotify: Arc<Spotify>) -> Self {
        Self {
            user: spotify.username(),
            spotify,
            kbps: 320,
            post: PostActions::new(),
            path_cache: HashMap::new(),
            stop_time: None,
        }
    }

    /// Pick the stream bitrate, falling back to 160 kbps without Premium.
    async fn select_quality(&mut self) {
        let a = args();
        self.kbps = a.quality.parse().unwrap_or(320);
        // the account attributes arrive shortly after connecting
        let mut account_type = None;
        for _ in 0..50 {
            account_type = self.spotify.account_type();
            if account_type.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if self.kbps == 320
            && let Some(t) = account_type
            && t != "premium"
        {
            warn!("account is not premium, falling back to the 160 kbps stream");
            self.kbps = 160;
        }
    }

    /// Load track metadata; `what` prefixes the loading status line.
    async fn load_tracks(&self, ids: &[String], what: &str) -> Vec<TrackInfo> {
        let mut tracks = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            if aborted() {
                break;
            }
            progress().loading(&format!("{what}loading track {}/{}", i + 1, ids.len()));
            match self.spotify.track(id).await {
                Ok(t) => tracks.push(t),
                Err(e) => {
                    progress().clear_loading();
                    warn!("cannot load spotify:track:{id}: {}", short_error(&e));
                }
            }
        }
        tracks
    }

    fn album_track_ids(album: &AlbumInfo) -> Vec<String> {
        album
            .discs
            .iter()
            .flat_map(|(_, tracks)| tracks.iter())
            .filter_map(|t| librespot_core::SpotifyId::try_from(t).ok())
            .map(|id| id.to_base62())
            .collect()
    }

    async fn load_link(&self, uri: &str) -> Result<Job, crate::spotify::Error> {
        let mut job = Job {
            tracks: Vec::new(),
            playlist: None,
            album: None,
        };
        let id = uri_to_id(uri).to_owned();

        if uri.starts_with("spotify:track:") {
            job.tracks = self.load_tracks(&[id], "").await;
        } else if uri.starts_with("spotify:playlist:") {
            let playlist = self.spotify.playlist(&id).await?;
            let ids: Vec<String> = playlist
                .tracks
                .iter()
                .map(|t| uri_to_id(t).to_owned())
                .collect();
            job.tracks = self.load_tracks(&ids, "").await;
            job.playlist = Some(playlist);
        } else if uri.starts_with("spotify:album:") {
            let album = self.spotify.album(&id).await?;
            job.tracks = self.load_tracks(&Self::album_track_ids(&album), "").await;
            job.album = Some(album);
        } else if uri.starts_with("spotify:artist:") {
            // the full discography, filtered by --artist-album-type
            let artist = self.spotify.artist(&id).await?;
            let wanted: Vec<String> = match &args().artist_album_type {
                Some(t) => t.split(',').map(|s| s.trim().to_owned()).collect(),
                None => vec!["album".into(), "single".into(), "compilation".into()],
            };
            let album_ids: Vec<&String> = wanted
                .iter()
                .filter_map(|w| artist.albums.get(w.as_str()))
                .flatten()
                .collect();
            let count = album_ids.len();
            for (i, album_id) in album_ids.into_iter().enumerate() {
                if aborted() {
                    break;
                }
                let what = format!("album {}/{count}, ", i + 1);
                match self.spotify.album(album_id).await {
                    Ok(album) => {
                        let ids = Self::album_track_ids(&album);
                        job.tracks.extend(self.load_tracks(&ids, &what).await);
                    }
                    Err(e) => {
                        progress().clear_loading();
                        warn!("cannot load spotify:album:{album_id}: {}", short_error(&e));
                    }
                }
            }
        } else if !uri.is_empty() {
            warn!("ignoring unsupported URI {uri}");
        }
        Ok(job)
    }

    fn format_track_path(&mut self, ctx: &Context, idx: usize, track: &TrackInfo) -> PathBuf {
        let a = args();
        let uri = track.uri();
        if let Some(path) = self.path_cache.get(&uri) {
            return path.clone();
        }

        let mut audio_file = format_track_string(ctx, a.format.trim(), idx, track);

        let truncate = |s: &str, max: usize| -> String {
            if s.chars().count() > max {
                s.chars().take(max).collect::<String>().trim().to_owned()
            } else {
                s.to_owned()
            }
        };
        let truncate_file_name = |name: &str| -> String {
            match name.rsplit_once('.') {
                Some((stem, ext)) => {
                    let max = 255usize.saturating_sub(ext.chars().count() + 1);
                    format!("{}.{ext}", truncate(stem, max))
                }
                None => truncate(name, 255),
            }
        };

        if a.windows_safe {
            audio_file = match audio_file.rsplit_once('/') {
                Some((dir, name)) => {
                    let dir: Vec<String> = dir.split('/').map(|t| truncate(t, 255)).collect();
                    format!("{}/{}", dir.join("/"), truncate_file_name(name))
                }
                None => truncate_file_name(&audio_file),
            };
        }

        if let Some(patterns) = &a.replace {
            for pattern in patterns {
                let mut parts = pattern.splitn(2, '/');
                let (Some(from), Some(to)) = (parts.next(), parts.next()) else {
                    continue;
                };
                match Regex::new(from) {
                    Ok(re) => {
                        audio_file = re
                            .replace_all(&audio_file, python_replacement(to).as_str())
                            .into_owned()
                    }
                    Err(e) => warn!("invalid --replace pattern {from}: {e}"),
                }
            }
        }

        if a.windows_safe {
            audio_file.retain(|c| !matches!(c, ':' | '"' | '*' | '?' | '<' | '>' | '|'));
        }

        let mut path = to_ascii(&base_dir().join(&audio_file).to_string_lossy());
        if a.normalized_ascii {
            path = to_normalized_ascii(&path);
        }
        let path = PathBuf::from(path);

        if let Some(dir) = path.parent()
            && let Err(e) = fs::create_dir_all(dir)
        {
            warn!("cannot create {}: {e}", dir.display());
        }

        self.path_cache.insert(uri, path.clone());
        path
    }

    /// Whether an existing file is a partial rip of `track`.
    fn is_partial(audio_file: &Path, track: &TrackInfo) -> bool {
        let duration = file_duration(audio_file).map(|d| d.as_millis() as u64);
        let track_ms = track.duration as u64;
        match args().partial_check.as_str() {
            "none" => false,
            // strict re-rips if unsure
            "strict" => duration.is_none_or(|d| track_ms > d),
            // weak gives a ~1.5 second wiggle-room
            _ => duration.is_some_and(|d| track_ms.saturating_sub(1500) > d),
        }
    }

    fn check_stop_time(&mut self) {
        let a = args();
        let Some(stop_after) = &a.stop_after else {
            return;
        };
        let stop_time = *self.stop_time.get_or_insert_with(|| {
            let t = parse_time_str(stop_after).expect("validated at startup");
            outln!("Stopping after {}", t.format("%H:%M"));
            t
        });
        if stop_time >= Local::now() {
            return;
        }
        outln!("Stop time {} reached", stop_time.format("%H:%M"));
        match a.resume_after.as_deref().and_then(parse_time_str) {
            Some(resume_time) => {
                outln!("Resuming at {}", resume_time.format("%H:%M"));
                while Local::now() < resume_time && !aborted() {
                    std::thread::sleep(Duration::from_secs(1));
                }
                self.stop_time = None;
            }
            None => crate::terminal::ABORT.store(true, Ordering::Relaxed),
        }
    }

    /// Sleep `seconds`, but stop early if the user aborts or skips.
    async fn interruptible_sleep(seconds: u64) {
        for _ in 0..seconds {
            if aborted() || skipped() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    pub async fn run(&mut self, uris: &[String]) {
        let a = args();
        let started = Instant::now();
        self.select_quality().await;

        // load everything first, to calculate the total size and time
        let mut jobs = Vec::new();
        for uri in uris {
            if aborted() {
                break;
            }
            let uri = normalize_uri(uri);
            match self.load_link(&uri).await {
                Ok(job) => jobs.push(job),
                Err(e) => {
                    progress().clear_loading();
                    print_status("", Status::Failed, &uri, &short_error(&e));
                }
            }
        }

        let user = self.user.clone();
        let mut totals = Vec::new();
        let (mut existing, mut unavailable) = (0, 0);
        for job in &jobs {
            let ctx = job.context(&user);
            for (idx, track) in job.tracks.iter().enumerate() {
                let path = self.format_track_path(&ctx, idx, track);
                let exists = !a.overwrite && path.exists() && !Self::is_partial(&path, track);
                existing += usize::from(track.available && exists);
                unavailable += usize::from(!track.available);
                totals.push((
                    track.duration,
                    calc_file_size(track.duration),
                    !track.available || exists,
                ));
            }
        }
        progress().calc_total(&totals);
        let (total_size, total_tracks) = {
            let p = progress();
            (p.total_size, p.total_tracks)
        };
        progress().clear_loading();
        if !totals.is_empty() {
            let plural = if totals.len() == 1 { "" } else { "s" };
            let mut found = format!("Found {} track{plural}: ", totals.len());
            if total_tracks > 0 {
                found.push_str(&format!(
                    "{total_tracks} to rip ({})",
                    format_size(total_size)
                ));
            } else {
                found.push_str("nothing to rip");
            }
            if existing > 0 {
                found.push_str(&format!(", {existing} already ripped"));
            }
            if unavailable > 0 {
                found.push_str(&format!(", {unavailable} unavailable"));
            }
            outln!("{found}");
        }
        // pin the live status bars to the bottom of the screen
        if total_tracks > 0 {
            progress().setup();
        }

        for job in &jobs {
            if aborted() {
                break;
            }
            let ctx = job.context(&user);

            if a.playlist_sync
                && let Some(playlist) = &job.playlist
            {
                let lib: Vec<(String, String)> = job
                    .tracks
                    .iter()
                    .enumerate()
                    .map(|(idx, t)| {
                        let path = self.format_track_path(&ctx, idx, t);
                        (t.uri(), path.to_string_lossy().into_owned())
                    })
                    .collect();
                crate::sync::sync_playlist(playlist, &lib);
            }

            for (idx, track) in job.tracks.iter().enumerate() {
                self.check_stop_time();
                SKIP.store(false, Ordering::Relaxed);
                if aborted() {
                    break;
                }
                if !self.rip_one(&ctx, idx, track).await {
                    break;
                }
            }

            // create playlist m3u/wpl files if needed
            let files: Vec<PathBuf> = job
                .tracks
                .iter()
                .enumerate()
                .map(|(idx, t)| self.format_track_path(&ctx, idx, t))
                .collect();
            let name = job.playlist_name();
            self.post.create_playlist_m3u(name.as_deref(), &files);
            self.post
                .create_playlist_wpl(name.as_deref(), &files, &user);
        }

        // done -- release the pinned status block before the summary
        progress().teardown();
        self.post.cleanup_offline_cache();
        self.post.end_failure_log();
        self.post.print_summary(started.elapsed());
    }

    /// Rip one track; returns false if ripping should stop (abort).
    async fn rip_one(&mut self, ctx: &Context<'_>, idx: usize, track: &TrackInfo) -> bool {
        let a = args();
        let (prefix, indent) = {
            let p = progress();
            (p.counter_prefix(), p.indent())
        };
        let audio_file = self.format_track_path(ctx, idx, track);
        let name = track_name(track);
        let details = |file: bool| {
            if a.verbose {
                if file {
                    outln!("{}", format_field(&indent, "file", rel_path(&audio_file)));
                }
                outln!("{}", format_field(&indent, "uri", track.uri()));
            }
        };

        if !track.available {
            print_status(
                &prefix,
                Status::Unavailable,
                &name,
                "not available in your region",
            );
            details(false);
            self.post.log_unavailable(summary_entry(track));
            progress().track_idx += 1;
            return true;
        }

        if !a.overwrite && audio_file.exists() {
            if Self::is_partial(&audio_file, track) {
                if a.verbose {
                    outln!("{indent}{DIM}replacing partial file{RESET}");
                }
            } else {
                print_status(&prefix, Status::Skipped, &name, "exists");
                details(true);
                self.post.log_skipped(summary_entry(track));
                progress().track_idx += 1;
                return true;
            }
        }

        let mut sinks = Sinks::default();
        let result = self
            .rip_track(track, &audio_file, &indent, &mut sinks)
            .await;
        RIPPING.store(false, Ordering::Relaxed);

        let failure = match result {
            Err(e) => Some(short_error(&e)),
            Ok(()) if skipped() => Some("skipped by user".to_owned()),
            Ok(()) if aborted() => Some("aborted".to_owned()),
            Ok(()) => None,
        };
        if let Some(reason) = failure {
            sinks.abort();
            self.post.clean_up_partial(&audio_file);
            self.post.log_failure(summary_entry(track));
            progress().end_track();
            print_status(&prefix, Status::Failed, &name, &reason);
            details(true);
            return !aborted();
        }

        if let Err(e) = tokio::task::block_in_place(|| sinks.finish()) {
            self.post.clean_up_partial(&audio_file);
            self.post.log_failure(summary_entry(track));
            progress().end_track();
            print_status(&prefix, Status::Failed, &name, &short_error(&e));
            details(true);
            return true;
        }

        // update tags and embed front cover image
        progress().set_phase("tagging", 1.0);
        let image = self.cover_image(track).await;
        let length = match set_metadata_tags(ctx, &audio_file, idx, track, image) {
            Ok(length) => length,
            Err(e) => {
                warn!("cannot save tags: {e}");
                Duration::from_millis(track.duration as u64)
            }
        };
        progress().end_track();

        let size = fs::metadata(&audio_file).map(|m| m.len()).unwrap_or(0);
        let sep = if a.ascii { " - " } else { " · " };
        print_status(
            &prefix,
            Status::Ripped,
            &format!("{name} ({})", track.album.year),
            &format!(
                "{}{sep}{}",
                format_time(length.as_secs(), None),
                format_size(size)
            ),
        );
        if a.verbose {
            details(true);
            let album = &track.album;
            outln!(
                "{}",
                format_field(
                    &indent,
                    "album",
                    to_ascii(&format!("{} ({})", album.name, album.artist_name()))
                )
            );
            outln!(
                "{}",
                format_field(
                    &indent,
                    "track",
                    format!(
                        "{}/{}, disc {}/{}",
                        track.number,
                        album.num_tracks(track.disc),
                        track.disc,
                        album.num_discs()
                    )
                )
            );
            if let Some(comment) = &a.comment {
                let comment = format_track_string(ctx, comment, idx, track);
                outln!("{}", format_field(&indent, "comment", comment));
            }
            if let Some(grouping) = &a.grouping {
                let grouping = format_track_string(ctx, grouping, idx, track);
                outln!("{}", format_field(&indent, "grouping", grouping));
            }
        }
        self.post.log_success(summary_entry(track));

        // pace requests to avoid hitting audio-key rate limits
        Self::interruptible_sleep(a.delay).await;
        true
    }

    /// Album cover to embed or save, if any.
    async fn cover_image(&self, track: &TrackInfo) -> Option<Vec<u8>> {
        let a = args();
        if a.output_type == "wav" || a.output_type == "pcm" {
            return None;
        }
        let url = track.album.cover_url(a.large_cover_art)?;
        match self.spotify.fetch_url(&url).await {
            Ok(data) => Some(data.to_vec()),
            Err(e) => {
                warn!("cannot retrieve the cover art: {}", short_error(&e));
                None
            }
        }
    }

    async fn rip_track(
        &self,
        track: &TrackInfo,
        audio_file: &Path,
        indent: &str,
        sinks: &mut Sinks,
    ) -> Result<(), crate::spotify::Error> {
        let a = args();
        progress().prepare_track(track.duration, &track_name(track));
        // mark as ripping up front so Esc works during the download too
        RIPPING.store(true, Ordering::Relaxed);

        let temp_ogg = PathBuf::from(format!("{}.part.ogg", audio_file.display()));

        // Spotify rate-limits audio key requests, so retry (--retries) with an
        // optional wait (--delay) for the limit to clear.
        let (track_id, file_id, kbps) = self.spotify.audio_file(&track.id, self.kbps).await?;
        let retries = a.retries.max(1);
        let mut attempt = 1;
        let key = loop {
            let result = self.spotify.audio_key(track_id, file_id).await;
            if aborted() || skipped() {
                return Ok(());
            }
            match result {
                Ok(key) => break key,
                Err(e) if attempt < retries => {
                    // without --delay, back off: 5, 10, 20, 40, 60, 60... seconds
                    let wait = if a.delay > 0 {
                        a.delay
                    } else {
                        (5u64 << (attempt - 1).min(4)).min(60)
                    };
                    if a.verbose {
                        outln!(
                            "{indent}{DIM}audio key rate-limited, retry {attempt}/{retries} in {wait}s{RESET}"
                        );
                    }
                    log::debug!("audio key error: {e}");
                    for left in (1..=wait).rev() {
                        if aborted() || skipped() {
                            break;
                        }
                        progress().set_phase(
                            &format!("waiting {left}s (retry {attempt}/{retries})"),
                            0.0,
                        );
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    if aborted() || skipped() {
                        return Ok(());
                    }
                    attempt += 1;
                }
                Err(e) => return Err(format!("audio key: {e}").into()),
            }
        };
        if aborted() || skipped() {
            return Ok(());
        }

        let mut stream = self.spotify.open_audio(file_id, kbps, key).await?;

        // download the decrypted Ogg Vorbis stream
        let total = stream.size;
        tokio::task::block_in_place(|| -> io::Result<()> {
            let mut ogg = BufWriter::new(File::create(&temp_ogg)?);
            let mut buf = vec![0u8; DOWNLOAD_CHUNK];
            let mut downloaded = 0u64;
            while downloaded < total && !aborted() && !skipped() {
                let n = stream.reader.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                ogg.write_all(&buf[..n])?;
                downloaded += n as u64;
                progress().add_downloaded(n as u64, downloaded, total);
            }
            ogg.flush()
        })?;

        if aborted() || skipped() {
            rm_file(&temp_ogg);
            return Ok(());
        }

        let result = tokio::task::block_in_place(|| -> io::Result<()> {
            *sinks = prepare_sinks(audio_file)?;

            // native Ogg Vorbis: keep the downloaded stream as-is
            if a.output_type == "ogg" {
                fs::copy(&temp_ogg, audio_file)?;
            }
            // everything else is decoded to PCM with ffmpeg
            if sinks.needs_pcm() {
                decode_and_feed(&temp_ogg, sinks)?;
            }
            Ok(())
        });
        rm_file(&temp_ogg);
        Ok(result?)
    }
}

/// Decode the Ogg Vorbis file to raw PCM via ffmpeg and feed the sinks.
fn decode_and_feed(temp_ogg: &Path, sinks: &mut Sinks) -> io::Result<()> {
    let rate = PCM_SAMPLE_RATE.to_string();
    let mut decoder = Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "quiet", "-i"])
        .arg(temp_ogg)
        .args(["-f", "s16le", "-ar", &rate, "-ac", "2", "-"])
        .stdout(Stdio::piped())
        .spawn()?;
    let mut stdout = decoder.stdout.take().expect("piped stdout");
    let mut buf = vec![0u8; DECODE_CHUNK];
    let mut position_ms = 0.0;
    let result = (|| -> io::Result<()> {
        loop {
            if aborted() || skipped() {
                return Ok(());
            }
            let n = stdout.read(&mut buf)?;
            if n == 0 {
                return Ok(());
            }
            position_ms += (n / PCM_FRAME_BYTES) as f64 * 1000.0 / PCM_SAMPLE_RATE as f64;
            progress().set_encoded(position_ms);
            sinks.write(&buf[..n])?;
        }
    })();
    if result.is_err() || aborted() || skipped() {
        let _ = decoder.kill();
    }
    drop(stdout);
    decoder.wait()?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_uris() {
        assert_eq!(
            normalize_uri("https://open.spotify.com/artist/1c3B6lj7BUcjM6R3tdTSh2?si=pHKo"),
            "spotify:artist:1c3B6lj7BUcjM6R3tdTSh2"
        );
        assert_eq!(
            normalize_uri("https://open.spotify.com/intl-de/track/2ZxdB1OprQj9TJBR5n0FMX"),
            "spotify:track:2ZxdB1OprQj9TJBR5n0FMX"
        );
        assert_eq!(normalize_uri("spotify:album:abc?x=1"), "spotify:album:abc");
        assert_eq!(normalize_uri("bogus"), "bogus");
    }
}
