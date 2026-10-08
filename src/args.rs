// Command line options, with defaults taken from config.json; an option given
// on the command line overrides the value in the config file.

use clap::{Arg, ArgAction, ArgGroup, ArgMatches, Command};
use serde_json::{Map, Value};
use std::sync::OnceLock;

#[derive(Debug, Default)]
pub struct Args {
    pub uri: Vec<String>,
    pub do_login: bool,
    pub ascii: bool,
    pub ascii_path_only: bool,
    pub all_artists: bool,
    pub artist_album_type: Option<String>,
    pub bitrate: String,
    pub cbr: bool,
    pub comp: String,
    pub comment: Option<String>,
    pub cover_file: Option<String>,
    pub cover_file_and_embed: Option<String>,
    pub directory: String,
    pub fail_log: Option<String>,
    pub format: String,
    pub format_case: Option<String>,
    pub genres: Option<String>,
    pub grouping: Option<String>,
    pub id3_v23: bool,
    pub large_cover_art: bool,
    pub log: Option<String>,
    pub normalized_ascii: bool,
    pub overwrite: bool,
    pub partial_check: String,
    pub playlist_m3u: bool,
    pub playlist_wpl: bool,
    pub playlist_sync: bool,
    pub plus_pcm: bool,
    pub plus_wav: bool,
    pub vbr: String,
    pub quality: String,
    pub keep_offline_cache: bool,
    pub retries: u32,
    pub delay: u64,
    pub resume_after: Option<String>,
    pub replace: Option<Vec<String>>,
    pub strip_colors: bool,
    pub stereo_mode: Option<String>,
    pub stop_after: Option<String>,
    pub windows_safe: bool,
    /// Output file extension, e.g. "mp3", "flac", "alac.m4a".
    pub output_type: String,
    pub has_log: bool,
}

static ARGS: OnceLock<Args> = OnceLock::new();

pub fn args() -> &'static Args {
    ARGS.get().expect("arguments not parsed yet")
}

pub fn set_args(args: Args) {
    ARGS.set(args).expect("arguments set twice");
}

const ENCODINGS: [&str; 9] = [
    "aiff", "alac", "flac", "id3_v23", "pcm", "mp4", "opus", "wav", "vorbis",
];

fn flag(id: &'static str, long: &'static str, help: &'static str) -> Arg {
    Arg::new(id)
        .long(long)
        .action(ArgAction::SetTrue)
        .help(help)
}

fn opt(id: &'static str, long: &'static str, help: &'static str) -> Arg {
    Arg::new(id)
        .long(long)
        .value_name(id.to_uppercase())
        .help(help)
}

pub fn command(version: &'static str) -> Command {
    Command::new("spotify-ripper")
        .about("Rips Spotify URIs to media files with tags and album covers")
        .version(version)
        .disable_version_flag(true)
        .infer_long_args(true)
        .arg(Arg::new("uri").num_args(0..).help("One or more Spotify URI(s) (either a URI or a file of URIs)"))
        .arg(flag("do_login", "login", "Pair with Spotify over Zeroconf (select \"spotify-ripper\" in the device list of the official Spotify app), saving reusable credentials for later runs"))
        .arg(flag("ascii", "ascii", "Convert the file name and the metadata tags to ASCII encoding [Default=utf-8]").short('a'))
        .arg(flag("aiff", "aiff", "Rip songs to lossless AIFF encoding instead of MP3"))
        .arg(flag("alac", "alac", "Rip songs to Apple Lossless format instead of MP3"))
        .arg(flag("all_artists", "all-artists", "Store all artists, rather than just the main artist, in the track's metadata tag"))
        .arg(opt("artist_album_type", "artist-album-type", "Comma-separated album types to include when ripping an artist URI: album, single, compilation, appears_on [Default=album,single,compilation]"))
        .arg(flag("ascii_path_only", "ascii-path-only", "Convert the file name (but not the metadata tags) to ASCII encoding [Default=utf-8]").short('A'))
        .arg(opt("bitrate", "bitrate", "CBR bitrate [Default=320]").short('b'))
        .arg(flag("cbr", "cbr", "CBR encoding [Default=VBR]").short('c'))
        .arg(opt("comp", "comp", "compression complexity for FLAC and Opus [Default=Max]"))
        .arg(opt("comment", "comment", "Set comment metadata tag to all songs. Can include same tags as --format."))
        .arg(opt("cover_file", "cover-file", "Save album cover image to file name (e.g \"cover.jpg\") [Default=embed]"))
        .arg(opt("cover_file_and_embed", "cover-file-and-embed", "Same as --cover-file but embeds the cover image too").value_name("COVER_FILE"))
        .arg(opt("directory", "directory", "Base directory where ripped songs are saved [Default=~/Music]").short('d'))
        .arg(opt("fail_log", "fail-log", "Logs the list of track URIs that failed to rip"))
        .arg(flag("flac", "flac", "Rip songs to lossless FLAC encoding instead of MP3"))
        .arg(opt("format", "format", "Save songs using this path and filename structure (see README)").short('f'))
        .arg(opt("format_case", "format-case", "Convert all words of the file name to upper-case, lower-case, or capitalized").value_parser(["upper", "lower", "capitalize"]))
        .arg(flag("flat", "flat", "Save all songs to a single directory (overrides --format option)"))
        .arg(flag("flat_with_index", "flat-with-index", "Similar to --flat [-f] but includes the playlist index at the start of the song file"))
        .arg(opt("genres", "genres", "Attempt to retrieve genre information from Spotify [Default=skip]").short('g').value_parser(["artist", "album"]))
        .arg(opt("grouping", "grouping", "Set grouping metadata tag to all songs. Can include same tags as --format."))
        .arg(flag("id3_v23", "id3-v23", "Store ID3 tags using version v2.3 [Default=v2.4]"))
        .arg(flag("large_cover_art", "large-cover-art", "Attempt to retrieve larger cover art from Spotify [Default=640x640]"))
        .arg(opt("log", "log", "Log in a log-friendly format to a file (use - to log to stdout)").short('L'))
        .arg(flag("pcm", "pcm", "Saves a .pcm file with the raw PCM data instead of MP3"))
        .arg(flag("mp4", "mp4", "Rip songs to MP4/M4A format with Fraunhofer FDK AAC codec instead of MP3"))
        .arg(flag("normalized_ascii", "normalized-ascii", "Convert the file name to normalized ASCII with Unicode NFKD normalization (short option: -na)"))
        .arg(flag("overwrite", "overwrite", "Overwrite existing MP3 files [Default=skip]").short('o'))
        .arg(flag("opus", "opus", "Rip songs to Opus encoding instead of MP3"))
        .arg(opt("partial_check", "partial-check", "Check for and overwrite partially ripped files. \"weak\" will err on the side of not re-ripping the file if it is unsure, whereas \"strict\" will re-rip the file [Default=weak]").value_parser(["none", "weak", "strict"]))
        .arg(flag("playlist_m3u", "playlist-m3u", "create a m3u file when ripping a playlist"))
        .arg(flag("playlist_wpl", "playlist-wpl", "create a wpl file when ripping a playlist"))
        .arg(flag("playlist_sync", "playlist-sync", "Sync playlist songs (rename and remove old songs)"))
        .arg(flag("plus_pcm", "plus-pcm", "Saves a .pcm file in addition to the encoded file (e.g. mp3)"))
        .arg(flag("plus_wav", "plus-wav", "Saves a .wav file in addition to the encoded file (e.g. mp3)"))
        .arg(opt("vbr", "vbr", "VBR quality setting or target bitrate for Opus [Default=0]").short('q'))
        .arg(opt("quality", "quality", "Spotify stream bitrate preference (320 requires Premium) [Default=320]").short('Q').value_parser(["160", "320", "96"]))
        .arg(flag("keep_offline_cache", "keep-offline-cache", "Keep librespot's offline audio cache instead of deleting it after a successful rip [Default=delete]"))
        .arg(opt("retries", "retries", "Number of times to retry a track when Spotify rate-limits its audio key [Default=5]").value_parser(clap::value_parser!(u32)))
        .arg(opt("delay", "delay", "Seconds to wait between tracks and between retries; raise this if you hit audio-key rate limits [Default=0]").value_parser(clap::value_parser!(u64)))
        .arg(opt("resume_after", "resume-after", "Resumes script after a certain amount of time has passed after stopping (e.g. 1h30m). Alternatively, accepts a specific time in 24hr format to start after (e.g 03:30, 16:15). Requires --stop-after option to be set"))
        .arg(opt("replace", "replace", "pattern to replace the output filename separated by \"/\". The following example replaces all spaces with \"_\" and all \"-\" with \".\": spotify-ripper --replace \" /_\" \"\\-/.\" uri").short('R').num_args(1..).action(ArgAction::Append))
        .arg(flag("strip_colors", "strip-colors", "Strip coloring from output [Default=colors]").short('s'))
        .arg(opt("stereo_mode", "stereo-mode", "Advanced stereo settings for Lame MP3 encoder only").value_parser(["j", "s", "f", "d", "m", "l", "r"]))
        .arg(opt("stop_after", "stop-after", "Stops script after a certain amount of time has passed (e.g. 1h30m). Alternatively, accepts a specific time in 24hr format to stop after (e.g 03:30, 16:15)"))
        .arg(flag("version", "version", "show program's version number and exit").short('V'))
        .arg(flag("wav", "wav", "Rip songs to uncompressed WAV file instead of MP3"))
        .arg(flag("windows_safe", "windows-safe", "Make filename safe for Windows file system (truncate filename to 255 characters)"))
        .arg(flag("vorbis", "vorbis", "Rip songs to native Ogg Vorbis (copied straight from Spotify, no re-encode)"))
        .group(ArgGroup::new("encoding").args(ENCODINGS).multiple(false))
}

/// argparse accepted the two-letter "-na" short option, clap does not.
pub fn normalize_argv(argv: impl Iterator<Item = String>) -> Vec<String> {
    argv.map(|a| {
        if a == "-na" {
            "--normalized-ascii".to_owned()
        } else {
            a
        }
    })
    .collect()
}

struct Resolver<'a> {
    matches: &'a ArgMatches,
    config: &'a Map<String, Value>,
}

impl Resolver<'_> {
    fn on_cli(&self, id: &str) -> bool {
        self.matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine)
    }

    fn flag(&self, id: &str) -> bool {
        if self.on_cli(id) {
            return self.matches.get_flag(id);
        }
        match self.config.get(id) {
            Some(Value::Bool(b)) => *b,
            Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
            Some(Value::String(s)) => !s.is_empty(),
            _ => false,
        }
    }

    fn opt_str(&self, id: &str) -> Option<String> {
        if self.on_cli(id) {
            return self.matches.get_one::<String>(id).cloned();
        }
        match self.config.get(id) {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Number(n)) => Some(n.to_string()),
            Some(Value::Bool(b)) => Some(b.to_string()),
            _ => None,
        }
    }

    fn string(&self, id: &str, default: &str) -> String {
        self.opt_str(id).unwrap_or_else(|| default.to_owned())
    }

    fn number(&self, id: &str, default: u64) -> u64 {
        if self.on_cli(id) {
            if let Ok(Some(v)) = self.matches.try_get_one::<u64>(id) {
                return *v;
            }
            if let Ok(Some(v)) = self.matches.try_get_one::<u32>(id) {
                return *v as u64;
            }
        }
        match self.config.get(id) {
            Some(Value::Number(n)) => n.as_u64().unwrap_or(default),
            Some(Value::String(s)) => s.parse().unwrap_or(default),
            _ => default,
        }
    }

    fn list(&self, id: &str) -> Option<Vec<String>> {
        if self.on_cli(id) {
            return self
                .matches
                .get_many::<String>(id)
                .map(|v| v.cloned().collect());
        }
        match self.config.get(id) {
            Some(Value::Array(a)) => Some(
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
            ),
            Some(Value::String(s)) => Some(vec![s.clone()]),
            _ => None,
        }
    }
}

pub fn resolve(matches: &ArgMatches, config: &Map<String, Value>) -> Args {
    let r = Resolver { matches, config };

    let mut args = Args {
        uri: matches
            .get_many::<String>("uri")
            .map(|v| v.cloned().collect())
            .unwrap_or_default(),
        do_login: r.flag("do_login"),
        ascii: r.flag("ascii"),
        ascii_path_only: r.flag("ascii_path_only"),
        all_artists: r.flag("all_artists"),
        artist_album_type: r.opt_str("artist_album_type"),
        bitrate: r.string("bitrate", "320"),
        cbr: r.flag("cbr"),
        comp: r.string("comp", "10"),
        comment: r.opt_str("comment"),
        cover_file: r.opt_str("cover_file"),
        cover_file_and_embed: r.opt_str("cover_file_and_embed"),
        directory: r.string("directory", "~/Music"),
        fail_log: r.opt_str("fail_log"),
        format: r.string("format", ""),
        format_case: r.opt_str("format_case"),
        genres: r.opt_str("genres"),
        grouping: r.opt_str("grouping"),
        id3_v23: r.flag("id3_v23"),
        large_cover_art: r.flag("large_cover_art"),
        log: r.opt_str("log"),
        normalized_ascii: r.flag("normalized_ascii"),
        overwrite: r.flag("overwrite"),
        partial_check: r.string("partial_check", "weak"),
        playlist_m3u: r.flag("playlist_m3u"),
        playlist_wpl: r.flag("playlist_wpl"),
        playlist_sync: r.flag("playlist_sync"),
        plus_pcm: r.flag("plus_pcm"),
        plus_wav: r.flag("plus_wav"),
        vbr: r.string("vbr", "0"),
        quality: r.string("quality", "320"),
        keep_offline_cache: r.flag("keep_offline_cache"),
        retries: r.number("retries", 5) as u32,
        delay: r.number("delay", 0),
        resume_after: r.opt_str("resume_after"),
        replace: r.list("replace"),
        strip_colors: r.flag("strip_colors"),
        stereo_mode: r.opt_str("stereo_mode"),
        stop_after: r.opt_str("stop_after"),
        windows_safe: r.flag("windows_safe"),
        output_type: String::new(),
        has_log: false,
    };
    args.has_log = args.log.is_some();

    if args.ascii_path_only {
        args.ascii = true;
    }

    args.output_type = if r.flag("wav") {
        "wav".into()
    } else if r.flag("pcm") {
        "pcm".into()
    } else if r.flag("flac") {
        if args.comp == "10" {
            args.comp = "8".into();
        }
        "flac".into()
    } else if r.flag("vorbis") {
        if args.vbr == "0" {
            args.vbr = "9".into();
        }
        "ogg".into()
    } else if r.flag("opus") {
        if args.vbr == "0" {
            args.vbr = "320".into();
        }
        "opus".into()
    } else if r.flag("mp4") {
        if args.vbr == "0" {
            args.vbr = "5".into();
        }
        "m4a".into()
    } else if r.flag("alac") {
        "alac.m4a".into()
    } else if r.flag("aiff") {
        "aiff".into()
    } else {
        "mp3".into()
    };

    if r.flag("flat") {
        args.format = "{artist} - {track_name}.{ext}".into();
    } else if r.flag("flat_with_index") {
        args.format = "{idx:3} - {artist} - {track_name}.{ext}".into();
    } else if args.format.is_empty() {
        args.format = "{album_artist}/{album}/{track_num:2} - {track_name}.{ext}".into();
    }

    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_config;

    fn parse(argv: &[&str], config: Map<String, Value>) -> Args {
        let argv = normalize_argv(argv.iter().map(|s| s.to_string()));
        let matches = command("4.0.0").try_get_matches_from(argv).unwrap();
        resolve(&matches, &config)
    }

    #[test]
    fn cli_overrides_config() {
        let mut config = default_config();
        config.insert("quality".into(), Value::from("160"));
        config.insert("all_artists".into(), Value::from(true));
        let a = parse(
            &["spotify-ripper", "-Q", "96", "spotify:track:x"],
            config.clone(),
        );
        assert_eq!(a.quality, "96");
        assert!(a.all_artists);
        assert_eq!(a.uri, ["spotify:track:x"]);
        let a = parse(&["spotify-ripper", "spotify:track:x"], config);
        assert_eq!(a.quality, "160");
    }

    #[test]
    fn output_types_and_defaults() {
        let a = parse(&["spotify-ripper", "--flac", "x"], default_config());
        assert_eq!((a.output_type.as_str(), a.comp.as_str()), ("flac", "8"));
        let a = parse(&["spotify-ripper", "--vorbis", "x"], default_config());
        assert_eq!((a.output_type.as_str(), a.vbr.as_str()), ("ogg", "9"));
        let a = parse(&["spotify-ripper", "x"], default_config());
        assert_eq!(a.output_type, "mp3");
        assert_eq!(
            a.format,
            "{album_artist}/{album}/{track_num:2} - {track_name}.{ext}"
        );
        assert_eq!(a.retries, 5);
        let a = parse(&["spotify-ripper", "-na", "--flat", "x"], default_config());
        assert!(a.normalized_ascii);
        assert_eq!(a.format, "{artist} - {track_name}.{ext}");
    }

    #[test]
    fn encodings_are_exclusive() {
        let argv = ["spotify-ripper", "--flac", "--opus", "x"];
        assert!(command("4.0.0").try_get_matches_from(argv).is_err());
    }

    #[test]
    fn replace_takes_several_patterns() {
        let a = parse(
            &["spotify-ripper", "-R", " /_", r"\-/.", "--", "x"],
            default_config(),
        );
        assert_eq!(a.replace.unwrap(), [" /_", r"\-/."]);
    }
}
