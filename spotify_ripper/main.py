#!/usr/bin/python3
# -*- coding: utf-8 -*-

from __future__ import unicode_literals

from colorama import init, Fore, AnsiToWin32
from spotify_ripper.ripper import Ripper
from spotify_ripper.utils import *
import argparse
import codecs
import json
import os
import select
import signal
import sys
import termios
import tty


# The default configuration.  Keys match argparse option destinations (the
# long option name with dashes turned into underscores).  This is written to
# config.json on first run and used as the baseline that the user's config.json
# is merged on top of.
DEFAULT_CONFIG = {
    # output location and file naming
    "directory": "~/Music",
    "format": None,
    "format_case": None,
    "ascii": False,
    "ascii_path_only": False,
    "normalized_ascii": False,
    "windows_safe": False,
    "overwrite": False,
    "replace": None,
    # Spotify stream quality and encoder settings
    "quality": "320",
    "cbr": False,
    "bitrate": "320",
    "vbr": "0",
    "comp": "10",
    "stereo_mode": None,
    # metadata
    "all_artists": False,
    "genres": None,
    "id3_v23": False,
    "large_cover_art": False,
    "comment": None,
    "grouping": None,
    "cover_file": None,
    "cover_file_and_embed": None,
    # playlists
    "playlist_sync": False,
    "playlist_m3u": False,
    "playlist_wpl": False,
    # artist filtering
    "artist_album_type": None,
    # misc
    "partial_check": "weak",
    "plus_pcm": False,
    "plus_wav": False,
    "keep_offline_cache": False,
    "fail_log": None,
}


def config_path():
    return os.path.join(settings_dir(), "config.json")


def write_json(path, data):
    """Write ``data`` to ``path`` as pretty-formatted JSON."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=4, sort_keys=True, ensure_ascii=False)
        f.write("\n")


def load_config():
    """Return the default configuration merged with the user's config.json.

    On first run (no config.json yet) the default configuration is written out
    so the user has a documented, editable starting point."""
    path = config_path()
    config = dict(DEFAULT_CONFIG)

    if os.path.exists(path):
        try:
            with open(path, "r", encoding="utf-8") as f:
                user_config = json.load(f)
            if isinstance(user_config, dict):
                config.update(user_config)
        except (ValueError, IOError) as e:
            print("\nError parsing config file: " + path)
            print(str(e))
    else:
        try:
            write_json(path, DEFAULT_CONFIG)
        except IOError as e:
            print("\nCould not write default config file: " + path)
            print(str(e))

    return config

def main():
    try:
        from importlib.metadata import version, PackageNotFoundError
        try:
            prog_version = version("spotify-ripper")
        except PackageNotFoundError:
            prog_version = "unknown"
    except ImportError:
        prog_version = "unknown"

    # load config (defaults merged with ~/.config/spotify-ripper/config.json)
    defaults = load_config()

    parser = argparse.ArgumentParser(prog='spotify-ripper', description='Rips Spotify URIs to media files with tags and album covers')

    encoding_group = parser.add_mutually_exclusive_group(required=False)

    # set defaults
    parser.set_defaults(**defaults)

    # Positional arguments
    parser.add_argument('uri', nargs="*", help='One or more Spotify URI(s) (either a URI or a file of URIs)')

    # Login / session
    parser.add_argument('--login', dest='do_login', action='store_true', help='Pair with Spotify over Zeroconf (select "spotify-ripper" in the device list of the official Spotify app), saving reusable credentials for later runs')

    # Optional arguments
    parser.add_argument('-a', '--ascii', action='store_true', help='Convert the file name and the metadata tags to ASCII encoding [Default=utf-8]')
    encoding_group.add_argument('--aiff', action='store_true', help='Rip songs to lossless AIFF encoding instead of MP3')
    encoding_group.add_argument('--alac', action='store_true', help='Rip songs to Apple Lossless format instead of MP3')
    parser.add_argument('--all-artists', action='store_true', help='Store all artists, rather than just the main artist, in the track\'s metadata tag')
    parser.add_argument('--artist-album-type', help='Comma-separated album types to include when ripping an artist URI: album, single, compilation, appears_on [Default=album,single,compilation]')
    parser.add_argument('-A', '--ascii-path-only', action='store_true', help='Convert the file name (but not the metadata tags) to ASCII encoding [Default=utf-8]')
    parser.add_argument('-b', '--bitrate', help='CBR bitrate [Default=320]')
    parser.add_argument('-c', '--cbr', action='store_true', help='CBR encoding [Default=VBR]')
    parser.add_argument('--comp', help='compression complexity for FLAC and Opus [Default=Max]')
    parser.add_argument('--comment', help='Set comment metadata tag to all songs. Can include same tags as --format.')
    parser.add_argument('--cover-file', help='Save album cover image to file name (e.g "cover.jpg") [Default=embed]')
    parser.add_argument('--cover-file-and-embed', metavar="COVER_FILE", help='Same as --cover-file but embeds the cover image too')
    parser.add_argument('-d', '--directory', help='Base directory where ripped songs are saved [Default=~/Music]')
    parser.add_argument('--fail-log', help="Logs the list of track URIs that failed to rip")
    encoding_group.add_argument('--flac', action='store_true', help='Rip songs to lossless FLAC encoding instead of MP3')
    parser.add_argument('-f', '--format', help='Save songs using this path and filename structure (see README)')
    parser.add_argument('--format-case', choices=['upper', 'lower', 'capitalize'], help='Convert all words of the file name to upper-case, lower-case, or capitalized')
    parser.add_argument('--flat', action='store_true', help='Save all songs to a single directory (overrides --format option)')
    parser.add_argument('--flat-with-index', action='store_true', help='Similar to --flat [-f] but includes the playlist index at the start of the song file')
    parser.add_argument('-g', '--genres', choices=['artist', 'album'], help='Attempt to retrieve genre information from Spotify [Default=skip]')
    parser.add_argument('--grouping', help='Set grouping metadata tag to all songs. Can include same tags as --format.')
    encoding_group.add_argument('--id3-v23', action='store_true', help='Store ID3 tags using version v2.3 [Default=v2.4]')
    parser.add_argument('--large-cover-art', action='store_true', help='Attempt to retrieve larger cover art from Spotify [Default=300x300]')
    parser.add_argument('-L', '--log', help='Log in a log-friendly format to a file (use - to log to stdout)')
    encoding_group.add_argument('--pcm', action='store_true', help='Saves a .pcm file with the raw PCM data instead of MP3')
    encoding_group.add_argument('--mp4', action='store_true', help='Rip songs to MP4/M4A format with Fraunhofer FDK AAC codec instead of MP3')
    parser.add_argument('-na', '--normalized-ascii', action='store_true', help='Convert the file name to normalized ASCII with unicodedata.normalize (NFKD)')
    parser.add_argument('-o', '--overwrite', action='store_true', help='Overwrite existing MP3 files [Default=skip]')
    encoding_group.add_argument('--opus', action='store_true', help='Rip songs to Opus encoding instead of MP3')
    parser.add_argument('--partial-check', choices=['none', 'weak', 'strict'], help='Check for and overwrite partially ripped files. "weak" will err on the side of not re-ripping the file if it is unsure, whereas "strict" will re-rip the file [Default=weak]')
    parser.add_argument('--playlist-m3u', action='store_true', help='create a m3u file when ripping a playlist')
    parser.add_argument('--playlist-wpl', action='store_true', help='create a wpl file when ripping a playlist')
    parser.add_argument('--playlist-sync', action='store_true', help='Sync playlist songs (rename and remove old songs)')
    parser.add_argument('--plus-pcm', action='store_true', help='Saves a .pcm file in addition to the encoded file (e.g. mp3)')
    parser.add_argument('--plus-wav', action='store_true', help='Saves a .wav file in addition to the encoded file (e.g. mp3)')
    parser.add_argument('-q', '--vbr', help='VBR quality setting or target bitrate for Opus [Default=0]')
    parser.add_argument('-Q', '--quality', choices=['160', '320', '96'], help='Spotify stream bitrate preference (320 requires Premium) [Default=320]')
    parser.add_argument('--keep-offline-cache', action='store_true', help='Keep librespot\'s offline audio cache instead of deleting it after a successful rip [Default=delete]')
    parser.add_argument('--resume-after', help='Resumes script after a certain amount of time has passed after stopping (e.g. 1h30m). Alternatively, accepts a specific time in 24hr format to start after (e.g 03:30, 16:15). Requires --stop-after option to be set')
    parser.add_argument('-R', '--replace', nargs="+", required=False, help='pattern to replace the output filename separated by "/". The following example replaces all spaces with "_" and all "-" with ".": spotify-ripper --replace " /_" "\\-/." uri')
    parser.add_argument('-s', '--strip-colors', action='store_true', help='Strip coloring from output [Default=colors]')
    parser.add_argument('--stereo-mode', choices=['j', 's', 'f', 'd', 'm', 'l', 'r'], help='Advanced stereo settings for Lame MP3 encoder only')
    parser.add_argument('--stop-after', help='Stops script after a certain amount of time has passed (e.g. 1h30m). Alternatively, accepts a specific time in 24hr format to stop after (e.g 03:30, 16:15)')
    parser.add_argument('-V', '--version', action='version', version=prog_version)
    encoding_group.add_argument('--wav', action='store_true', help='Rip songs to uncompressed WAV file instead of MP3')
    parser.add_argument('--windows-safe', action='store_true', help='Make filename safe for Windows file system (truncate filename to 255 characters)')
    encoding_group.add_argument('--vorbis', action='store_true', help='Rip songs to native Ogg Vorbis (copied straight from Spotify, no re-encode)')

    args = parser.parse_args()

    if not args.do_login and not args.uri:
        parser.error("at least one Spotify URI is required (or use --login to "
                     "pair with the Spotify app)")

    init_util_globals(args)

    # kind of a hack to get colorama stripping to work when outputting
    # to a file instead of stdout.  Taken from initialise.py in colorama
    def wrap_stream(stream, convert, strip, autoreset, wrap):
        if wrap:
            wrapper = AnsiToWin32(stream, convert=convert, strip=strip, autoreset=autoreset)
            if wrapper.should_wrap():
                stream = wrapper.stream
        return stream

    args.has_log = args.log is not None
    if args.has_log:
        if args.log == "-":
            init(strip=True)
        else:
            encoding = "ascii" if args.ascii else "utf-8"
            log_file = codecs.open(enc_str(args.log), 'a', encoding)
            sys.stdout = wrap_stream(log_file, None, True, False, True)
    else:
        init(strip=True if args.strip_colors else None)

    if args.ascii_path_only is True:
        args.ascii = True

    if args.wav:
        args.output_type = "wav"
    elif args.pcm:
        args.output_type = "pcm"
    elif args.flac:
        args.output_type = "flac"
        if args.comp == "10":
            args.comp = "8"
    elif args.vorbis:
        args.output_type = "ogg"
        if args.vbr == "0":
            args.vbr = "9"
    elif args.opus:
        args.output_type = "opus"
        if args.vbr == "0":
            args.vbr = "320"
    elif args.mp4:
        args.output_type = "m4a"
        if args.vbr == "0":
            args.vbr = "5"
    elif args.alac:
        args.output_type = "alac.m4a"
    elif args.aiff:
        args.output_type = "aiff"
    else:
        args.output_type = "mp3"

    # check that the required encoder tool is available.  The native Ogg
    # Vorbis output is a straight copy of the Spotify stream, so it needs no
    # encoder; every other format is produced by decoding that stream to PCM
    # with ffmpeg and (for compressed formats) piping it into an encoder.
    encoders = {
        "flac": ("flac", "flac"),
        "aiff": ("sox", "sox"),
        "opus": ("opusenc", "opus-tools"),
        "mp3": ("lame", "lame"),
        "m4a": ("fdkaac", "aac-enc"),
        "alac.m4a": ("ffmpeg", "ffmpeg"),
    }
    if args.output_type in encoders.keys():
        encoder = encoders[args.output_type][0]
        if which(encoder) is None:
            print(Fore.RED + "Missing dependency '" + encoder + "'. Please install '" + encoders[args.output_type][1] + "'." + Fore.RESET)
            sys.exit(1)

    # ffmpeg is needed to decode the Ogg Vorbis stream for anything other than
    # the native Ogg Vorbis copy
    needs_ffmpeg = args.output_type != "ogg" or args.plus_wav or args.plus_pcm
    if needs_ffmpeg and which("ffmpeg") is None:
        print(Fore.RED + "Missing dependency 'ffmpeg'. Please install 'ffmpeg'." + Fore.RESET)
        sys.exit(1)

    # format string
    if args.flat:
        args.format = "{artist} - {track_name}.{ext}"
    elif args.flat_with_index:
        args.format = "{idx:3} - {artist} - {track_name}.{ext}"
    elif args.format is None:
        args.format = "{album_artist}/{album}/{artist} - {track_name}.{ext}"

    # print some settings
    print(Fore.GREEN + "Spotify Ripper - v" + prog_version + Fore.RESET)

    def encoding_output_str():
        if args.output_type == "wav":
            return "WAV, Stereo 16bit 44100Hz"
        elif args.output_type == "pcm":
            return "Raw Headerless PCM, Stereo 16bit 44100Hz"
        else:
            if args.output_type == "flac":
                return "FLAC, Compression Level: " + args.comp
            elif args.output_type == "aiff":
                return "AIFF"
            elif args.output_type == "alac.m4a":
                return "Apple Lossless (ALAC)"
            elif args.output_type == "ogg":
                return "Ogg Vorbis (native, copied from Spotify)"
            elif args.output_type == "opus":
                codec = "Opus"
            elif args.output_type == "mp3":
                codec = "MP3"
            elif args.output_type == "m4a":
                codec = "MPEG4 AAC"
            else:
                codec = "Unknown"

            if args.cbr:
                return codec + ", CBR " + args.bitrate + " kbps"
            else:
                return codec + ", VBR " + args.vbr

    print(Fore.YELLOW + "  Encoding output:\t" + Fore.RESET + encoding_output_str())
    print(Fore.YELLOW + "  Spotify bitrate:\t" + Fore.RESET + args.quality + " kbps")

    def unicode_support_str():
        if args.ascii_path_only:
            return "Unicode tags, ASCII file path"
        elif args.ascii:
            return "ASCII only"
        else:
            return "Yes"

    # check that --stop-after and --resume-after options are valid
    if args.stop_after is not None and \
            parse_time_str(args.stop_after) is None:
        print(Fore.RED + "--stop-after option is not valid" + Fore.RESET)
        sys.exit(1)
    if args.resume_after is not None and \
            parse_time_str(args.resume_after) is None:
        print(Fore.RED + "--resume-after option is not valid" + Fore.RESET)
        sys.exit(1)

    print(Fore.YELLOW + "  Unicode support:\t" + Fore.RESET + unicode_support_str())
    print(Fore.YELLOW + "  Output directory:\t" + Fore.RESET + base_dir())
    print(Fore.YELLOW + "  Settings directory:\t" + Fore.RESET + settings_dir())

    print(Fore.YELLOW + "  Format String:\t" + Fore.RESET + args.format)
    print(Fore.YELLOW + "  Overwrite files:\t" + Fore.RESET + ("Yes" if args.overwrite else "No"))

    ripper = Ripper(args)
    ripper.start()

    # try to listen for terminal resize events
    # (needs to be called on main thread)
    if not args.has_log:
        ripper.progress.handle_resize()
        signal.signal(signal.SIGWINCH, ripper.progress.handle_resize)

    def hasStdinData():
        return select.select([sys.stdin], [], [], 0) == ([sys.stdin], [], [])

    def abort(set_logged_in=False):
        ripper.abort_rip()
        if set_logged_in:
            ripper.ripper_continue.set()
        ripper.join()
        sys.exit(1)

    def skip():
        if ripper.ripping.is_set():
            ripper.skip.set()

    # check if we were passed a file name
    def check_uri_args():
        if len(args.uri) == 1 and path_exists(args.uri[0]):
            encoding = "ascii" if args.ascii else "utf-8"
            args.uri = [line.strip() for line in codecs.open(enc_str(args.uri[0]), 'r', encoding) if not line.strip().startswith("#") and len(line.strip()) > 0]

    # login and uri_parse on main thread to catch any KeyboardInterrupt
    try:
        if not ripper.login():
            print(Fore.RED + "Encountered issue while logging into " "Spotify, aborting..." + Fore.RESET)
            abort(set_logged_in=True)
        else:
            check_uri_args()
            ripper.ripper_continue.set()

    except (KeyboardInterrupt, Exception) as e:
        if not isinstance(e, KeyboardInterrupt):
            print(str(e))
        print("\n" + Fore.RED + "Aborting..." + Fore.RESET)
        abort(set_logged_in=True)

    # wait for ripping thread to finish
    if not args.has_log:
        try:
            stdin_settings = termios.tcgetattr(sys.stdin)
        except termios.error:
            stdin_settings = None
    try:
        if not args.has_log and stdin_settings:
            tty.setcbreak(sys.stdin.fileno())

        while ripper.is_alive():
            ripper.progress.tick()

            # check if the escape button was pressed
            if not args.has_log and hasStdinData():
                c = sys.stdin.read(1)
                if c == '\x1b':
                    skip()
            ripper.join(0.1)
    except (KeyboardInterrupt, Exception) as e:
        if not isinstance(e, KeyboardInterrupt):
            print(str(e))
        print("\n" + Fore.RED + "Aborting..." + Fore.RESET)
        abort()
    finally:
        if not args.has_log and stdin_settings:
            termios.tcsetattr(sys.stdin, termios.TCSADRAIN, stdin_settings)

if __name__ == '__main__':
    main()
