mod args;
mod config;
mod format;
mod output;
mod post_actions;
mod progress;
mod ripper;
mod spotify;
mod sync;
mod tags;
mod terminal;
mod util;

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::args::{args, set_args};
use crate::output::{GREEN, RED, RESET, YELLOW};
use crate::util::{base_dir, parse_time_str, settings_dir, to_ascii, which};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn format_name(output_type: &str) -> &str {
    match output_type {
        "wav" => "WAV",
        "pcm" => "Raw headerless PCM",
        "flac" => "FLAC",
        "aiff" => "AIFF",
        "alac.m4a" => "Apple Lossless (ALAC)",
        "ogg" => "Ogg Vorbis (native, copied from Spotify)",
        "opus" => "Opus",
        "mp3" => "MP3",
        "m4a" => "MPEG-4 AAC",
        other => other,
    }
}

fn quality_str() -> String {
    let a = args();
    match a.output_type.as_str() {
        "wav" | "pcm" => "Stereo 16-bit 44100 Hz".into(),
        "ogg" => "copied as-is from Spotify".into(),
        "flac" => format!("compression level {}", a.comp),
        "alac.m4a" => "lossless".into(),
        _ if a.cbr => format!("CBR {} kbps", a.bitrate),
        _ => format!("VBR {}", a.vbr),
    }
}

fn print_settings() {
    let a = args();
    outln!("{GREEN}Spotify Ripper - v{VERSION}{RESET}");
    outln!("{YELLOW}  Format:\t\t{RESET}{}", format_name(&a.output_type));
    outln!("{YELLOW}  Quality:\t\t{RESET}{}", quality_str());
    outln!("{YELLOW}  Spotify bitrate:\t{RESET}{} kbps", a.quality);
    if a.output_type == "mp3" || a.output_type == "aiff" {
        outln!(
            "{YELLOW}  ID3 tags:\t\t{RESET}{}",
            if a.id3_v23 { "v2.3" } else { "v2.4" }
        );
    }
    if a.output_type != "wav" && a.output_type != "pcm" {
        let cover = if let Some(f) = &a.cover_file {
            format!("Saved to {f}")
        } else if let Some(f) = &a.cover_file_and_embed {
            format!("Embedded + saved to {f}")
        } else {
            "Embedded".to_owned()
        };
        outln!("{YELLOW}  Cover image:\t\t{RESET}{cover}");
    }

    // check that --stop-after and --resume-after options are valid
    if a.stop_after.as_deref().is_some_and(|s| parse_time_str(s).is_none()) {
        outln!("{RED}--stop-after option is not valid{RESET}");
        std::process::exit(1);
    }
    if a.resume_after.as_deref().is_some_and(|s| parse_time_str(s).is_none()) {
        outln!("{RED}--resume-after option is not valid{RESET}");
        std::process::exit(1);
    }

    let unicode = if a.ascii_path_only {
        "Unicode tags, ASCII file path"
    } else if a.ascii {
        "ASCII only"
    } else {
        "Yes"
    };
    outln!("{YELLOW}  Unicode support:\t{RESET}{unicode}");
    outln!("{YELLOW}  Output directory:\t{RESET}{}", base_dir().display());
    outln!("{YELLOW}  Settings directory:\t{RESET}{}", settings_dir().display());
    outln!("{YELLOW}  Format String:\t{RESET}{}", a.format);
    outln!(
        "{YELLOW}  Overwrite files:\t{RESET}{}",
        if a.overwrite { "Yes" } else { "No" }
    );
}

/// Check that the encoder for the output format (and ffmpeg) are installed.
fn check_dependencies() {
    let a = args();
    let encoder = match a.output_type.as_str() {
        "flac" => Some(("flac", "flac")),
        "aiff" => Some(("sox", "sox")),
        "opus" => Some(("opusenc", "opus-tools")),
        "mp3" => Some(("lame", "lame")),
        "m4a" => Some(("fdkaac", "aac-enc")),
        "alac.m4a" => Some(("ffmpeg", "ffmpeg")),
        _ => None,
    };
    if let Some((program, package)) = encoder
        && which(program).is_none()
    {
        outln!("{RED}Missing dependency '{program}'. Please install '{package}'.{RESET}");
        std::process::exit(1);
    }

    // ffmpeg decodes the Ogg Vorbis stream for anything but the native copy
    let needs_ffmpeg = a.output_type != "ogg" || a.plus_wav || a.plus_pcm;
    if needs_ffmpeg && which("ffmpeg").is_none() {
        outln!("{RED}Missing dependency 'ffmpeg'. Please install 'ffmpeg'.{RESET}");
        std::process::exit(1);
    }
}

/// A single existing file argument is a list of URIs, one per line.
fn expand_uri_file(uris: Vec<String>) -> Vec<String> {
    if uris.len() == 1 && Path::new(&uris[0]).exists() {
        match fs::read_to_string(&uris[0]) {
            Ok(content) => {
                return content
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty() && !l.starts_with('#'))
                    .map(to_ascii)
                    .collect();
            }
            Err(e) => outln!("{RED}Cannot read {}: {e}{RESET}", uris[0]),
        }
    }
    uris
}

async fn run() -> i32 {
    let a = args();

    if a.do_login {
        match spotify::login_via_zeroconf("spotify-ripper", Duration::from_secs(300)).await {
            Ok(true) => {}
            Ok(false) => return login_failed(),
            Err(e) => {
                outln!("{RED}Spotify Connect login failed: {e}{RESET}");
                return login_failed();
            }
        }
    }

    if !spotify::has_stored_credentials() {
        outln!(
            "{RED}No stored Spotify credentials found. Run with --login to pair with the Spotify app first.{RESET}"
        );
        return login_failed();
    }

    let spotify = match spotify::Spotify::connect().await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            outln!("{RED}Failed to create Spotify session: {e}{RESET}");
            return login_failed();
        }
    };
    outln!("{GREEN}Logged in as {}\n{RESET}", spotify.username());

    terminal::start(!a.has_log);

    let uris = expand_uri_file(a.uri.clone());
    let mut ripper = ripper::Ripper::new(spotify.clone());
    ripper.run(&uris).await;
    spotify.shutdown();

    if terminal::aborted() { 1 } else { 0 }
}

fn login_failed() -> i32 {
    outln!("{RED}Encountered issue while logging into Spotify, aborting...{RESET}");
    1
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("off")).init();

    // config.json provides the defaults for the command line options
    let config = config::load_config();
    let mut cmd = args::command(VERSION);
    let matches = cmd
        .clone()
        .get_matches_from(args::normalize_argv(std::env::args()));
    if matches.get_flag("version") {
        println!("{VERSION}");
        return;
    }
    let resolved = args::resolve(&matches, &config);

    if !resolved.do_login && resolved.uri.is_empty() {
        cmd.error(
            clap::error::ErrorKind::MissingRequiredArgument,
            "at least one Spotify URI is required (or use --login to pair with the Spotify app)",
        )
        .exit();
    }

    if let Err(e) = output::init(resolved.log.as_deref(), resolved.strip_colors) {
        eprintln!("Cannot open log file: {e}");
        std::process::exit(1);
    }
    set_args(resolved);

    check_dependencies();
    print_settings();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("creating the tokio runtime");
    let code = runtime.block_on(run());
    runtime.shutdown_timeout(Duration::from_secs(1));
    terminal::exit(code);
}
