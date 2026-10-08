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
use crate::output::{BOLD, RED, RESET};
use crate::util::{base_dir, parse_time_str, settings_dir, to_ascii, which};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Output format and encoder settings, e.g. "MP3 VBR 0" or "FLAC level 8".
fn format_description() -> String {
    let a = args();
    let encoder = || {
        if a.cbr {
            format!("CBR {} kbps", a.bitrate)
        } else {
            format!("VBR {}", a.vbr)
        }
    };
    match a.output_type.as_str() {
        "wav" => "WAV".into(),
        "pcm" => "raw PCM".into(),
        "flac" => format!("FLAC level {}", a.comp),
        "aiff" => "AIFF".into(),
        "alac.m4a" => "ALAC".into(),
        "ogg" => "Ogg Vorbis (native)".into(),
        "opus" => format!("Opus {}", encoder()),
        "m4a" => format!("AAC {}", encoder()),
        _ => format!("MP3 {}", encoder()),
    }
}

/// Check that --stop-after and --resume-after are valid.
fn validate_times() {
    let a = args();
    for (option, value) in [
        ("--stop-after", &a.stop_after),
        ("--resume-after", &a.resume_after),
    ] {
        if value
            .as_deref()
            .is_some_and(|s| parse_time_str(s).is_none())
        {
            outln!("{RED}{option} option is not valid{RESET}");
            std::process::exit(1);
        }
    }
}

/// Two-line header: version and settings, then the format string.
fn print_header(user: &str) {
    let a = args();
    let mut items = vec![format_description(), format!("{} kbps", a.quality)];
    if a.output_type == "mp3" || a.output_type == "aiff" {
        items.push(if a.id3_v23 { "ID3v2.3" } else { "ID3v2.4" }.into());
    }
    let dir = base_dir().display().to_string();
    let home = std::env::var("HOME").unwrap_or_default();
    items.push(match dir.strip_prefix(&home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => dir,
    });
    items.push(user.to_owned());
    if a.overwrite {
        items.push("overwrite".into());
    }
    if a.ascii_path_only {
        items.push("ASCII paths".into());
    } else if a.ascii {
        items.push("ASCII".into());
    }
    if let Some(f) = &a.cover_file {
        items.push(format!("cover to {f}"));
    } else if let Some(f) = &a.cover_file_and_embed {
        items.push(format!("cover embedded and to {f}"));
    }

    let sep = if a.ascii { " | " } else { " · " };
    outln!(
        "{BOLD}spotify-ripper {VERSION}{RESET}{sep}{}",
        items.join(sep)
    );
    outln!("{BOLD}Format:{RESET} {}", a.format);
    if a.verbose {
        outln!("{BOLD}Settings:{RESET} {}", settings_dir().display());
    }
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
    print_header(&spotify.username());

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
    validate_times();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("creating the tokio runtime");
    let code = runtime.block_on(run());
    runtime.shutdown_timeout(Duration::from_secs(1));
    terminal::exit(code);
}
