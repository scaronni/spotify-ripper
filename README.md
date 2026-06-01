# spotify-ripper

`spotify-ripper` is a small ripper program for Spotify that rips Spotify URIs to audio files with metadata and cover art. By default `spotify-ripper` will encode to MP3 files, but includes the ability to rip to WAV, FLAC, Ogg Vorbis, Opus, and MP4/M4A.

**Note that stream ripping violates Spotify's ToS**

## Login

`spotify-ripper` logs in over **Zeroconf / Spotify Connect**. To pair the ripper with your account, run it once with `--login`:

```
spotify-ripper --login
```

This advertises the ripper as a Spotify Connect device on your local network. Open the **official Spotify app** (desktop or phone, logged into your account, on the same network), open the device/speaker picker, and select **`spotify-ripper`**. The reusable credentials are then saved to `~/.config/spotify-ripper/credentials.json` and used automatically on every later run — you only need to do this once (until the credentials are revoked or expire).

A few notes:
- A **Spotify Premium** account is required to download the 320 kbps stream; free accounts are limited to 160 kbps and the ripper falls back automatically.
- Track, album, artist and playlist metadata (and cover art and genres) are fetched over the Spotify protocol. Ripping is done by **track / album / playlist / artist URL or URI**.

## Features

-  Downloads the native Ogg Vorbis stream from Spotify and transcodes to other formats with ffmpeg
-  Writes ID3v2/metadata tags (including album covers)
-  Rips files into the following directory structure: `artist/album/artist - song.mp3` by default or optionally into a user-specified structure (see `Format String`_ section below)
-  Option to skip or overwrite existing files
-  Accepts tracks, playlists, albums, and artist URIs
-  One-time Zeroconf pairing with the official Spotify app; reusable credentials stored for later runs
-  Globally installs ripper using pip3
-  Use a config file to specify common command-line options
-  Helpful progress bar to gauge the time remaining until completion
-  Keep local files in sync with a Spotify playlist, m3u and wpl playlist file
-  Option to rip to ALAC/FLAC/AIFF (loseless codecs) or Ogg Vorbis, Opus and MP4/M4A instead of MP3
-  Option to replace output filenames
-  Option to normalize output filenames to NFKD (see http://unicode.org/faq/normalization.html)

**Please note: Spotify’s highest quality setting is 320 kbps, so the benefit of ripping to a lossless format is to not double encode the audio data. It’s not possible to rip in true lossless quality.**

## Spotify URIs/URLs

You can rip **songs, albums, playlists and artists**, passing either the share
URL straight from the Spotify client or the equivalent `spotify:` URI. Both of
these refer to the same artist and work identically:

```
https://open.spotify.com/artist/0zfT626RwO6zN3RDYeRit5
spotify:artist:0zfT626RwO6zN3RDYeRit5
```

Pass one or more of them on the command line, or put a list of them (one per
line, `#` for comments) in a text file and pass that file as a download queue.
Nothing else is supported — search, charts and the "liked songs" library are
not available.

An **artist** URI rips the artist's full discography (all albums, singles and
compilations). Use `--artist-album-type` to narrow that down, e.g.
`--artist-album-type album` for studio albums only.

## Program usage

The program takes many command-line options:

```
usage: spotify-ripper [-h] [--login] [-a] [--aiff | --alac | --flac | --id3-v23 | --pcm | --mp4 | --opus | --wav | --vorbis] [--all-artists]
                      [--artist-album-type ARTIST_ALBUM_TYPE] [-A] [-b BITRATE] [-c] [--comp COMP]
                      [--comment COMMENT] [--cover-file COVER_FILE] [--cover-file-and-embed COVER_FILE] [-d DIRECTORY] [--fail-log FAIL_LOG] [-f FORMAT]
                      [--format-case {upper,lower,capitalize}] [--flat] [--flat-with-index] [-g {artist,album}] [--grouping GROUPING] [--large-cover-art]
                      [-L LOG] [-na] [-o] [--partial-check {none,weak,strict}] [--playlist-m3u] [--playlist-wpl] [--playlist-sync] [--plus-pcm]
                      [--plus-wav] [-q VBR] [-Q {160,320,96}] [--keep-offline-cache] [--resume-after RESUME_AFTER] [-R REPLACE [REPLACE ...]] [-s]
                      [--stereo-mode {j,s,f,d,m,l,r}] [--stop-after STOP_AFTER] [-V] [--windows-safe]
                      [uri ...]

Rips Spotify URIs to media files with tags and album covers

positional arguments:
  uri                   One or more Spotify URI(s) (either a URI or a file of URIs)

options:
  -h, --help            show this help message and exit
  --login               Pair with Spotify over Zeroconf (select "spotify-ripper" in the device list of the official Spotify app), saving reusable credentials
                        for later runs
  -a, --ascii           Convert the file name and the metadata tags to ASCII encoding [Default=utf-8]
  --aiff                Rip songs to lossless AIFF encoding instead of MP3
  --alac                Rip songs to Apple Lossless format instead of MP3
  --all-artists         Store all artists, rather than just the main artist, in the track's metadata tag
  --artist-album-type ARTIST_ALBUM_TYPE
                        Comma-separated album types to include when ripping an artist URI: album, single, compilation, appears_on [Default=album,single,compilation]
  -A, --ascii-path-only
                        Convert the file name (but not the metadata tags) to ASCII encoding [Default=utf-8]
  -b, --bitrate BITRATE
                        CBR bitrate [Default=320]
  -c, --cbr             CBR encoding [Default=VBR]
  --comp COMP           compression complexity for FLAC and Opus [Default=Max]
  --comment COMMENT     Set comment metadata tag to all songs. Can include same tags as --format.
  --cover-file COVER_FILE
                        Save album cover image to file name (e.g "cover.jpg") [Default=embed]
  --cover-file-and-embed COVER_FILE
                        Same as --cover-file but embeds the cover image too
  -d, --directory DIRECTORY
                        Base directory where ripped songs are saved [Default=~/Music]
  --fail-log FAIL_LOG   Logs the list of track URIs that failed to rip
  --flac                Rip songs to lossless FLAC encoding instead of MP3
  -f, --format FORMAT   Save songs using this path and filename structure (see README)
  --format-case {upper,lower,capitalize}
                        Convert all words of the file name to upper-case, lower-case, or capitalized
  --flat                Save all songs to a single directory (overrides --format option)
  --flat-with-index     Similar to --flat [-f] but includes the playlist index at the start of the song file
  -g, --genres {artist,album}
                        Attempt to retrieve genre information from Spotify [Default=skip]
  --grouping GROUPING   Set grouping metadata tag to all songs. Can include same tags as --format.
  --id3-v23             Store ID3 tags using version v2.3 [Default=v2.4]
  --large-cover-art     Attempt to retrieve larger cover art from Spotify [Default=300x300]
  -L, --log LOG         Log in a log-friendly format to a file (use - to log to stdout)
  --pcm                 Saves a .pcm file with the raw PCM data instead of MP3
  --mp4                 Rip songs to MP4/M4A format with Fraunhofer FDK AAC codec instead of MP3
  -na, --normalized-ascii
                        Convert the file name to normalized ASCII with unicodedata.normalize (NFKD)
  -o, --overwrite       Overwrite existing MP3 files [Default=skip]
  --opus                Rip songs to Opus encoding instead of MP3
  --partial-check {none,weak,strict}
                        Check for and overwrite partially ripped files. "weak" will err on the side of not re-ripping the file if it is unsure, whereas
                        "strict" will re-rip the file [Default=weak]
  --playlist-m3u        create a m3u file when ripping a playlist
  --playlist-wpl        create a wpl file when ripping a playlist
  --playlist-sync       Sync playlist songs (rename and remove old songs)
  --plus-pcm            Saves a .pcm file in addition to the encoded file (e.g. mp3)
  --plus-wav            Saves a .wav file in addition to the encoded file (e.g. mp3)
  -q, --vbr VBR         VBR quality setting or target bitrate for Opus [Default=0]
  -Q, --quality {160,320,96}
                        Spotify stream bitrate preference (320 requires Premium) [Default=320]
  --keep-offline-cache  Keep librespot's offline audio cache instead of deleting it after a successful rip [Default=delete]
  --resume-after RESUME_AFTER
                        Resumes script after a certain amount of time has passed after stopping (e.g. 1h30m). Alternatively, accepts a specific time in 24hr
                        format to start after (e.g 03:30, 16:15). Requires --stop-after option to be set
  -R, --replace REPLACE [REPLACE ...]
                        pattern to replace the output filename separated by "/". The following example replaces all spaces with "_" and all "-" with ".":
                        spotify-ripper --replace " /_" "\-/." uri
  -s, --strip-colors    Strip coloring from output [Default=colors]
  --stereo-mode {j,s,f,d,m,l,r}
                        Advanced stereo settings for Lame MP3 encoder only
  --stop-after STOP_AFTER
                        Stops script after a certain amount of time has passed (e.g. 1h30m). Alternatively, accepts a specific time in 24hr format to stop
                        after (e.g 03:30, 16:15)
  -V, --version         show program's version number and exit
  --wav                 Rip songs to uncompressed WAV file instead of MP3
  --windows-safe        Make filename safe for Windows file system (truncate filename to 255 characters)
  --vorbis              Rip songs to native Ogg Vorbis (copied straight from Spotify, no re-encode)

Example usage:
    pair with Spotify (once): spotify-ripper --login
    rip a single track: spotify-ripper spotify:track:52xaypL0Kjzk0ngwv3oBPR
    rip an entire album: spotify-ripper spotify:album:6QaVfG1pHYl1z15ZxkvVDW
    rip an entire playlist: spotify-ripper spotify:playlist:4vkGNcsS8lRXj4q945NIA4
    rip a list of URIs: spotify-ripper list_of_uris.txt
```

### Config File

For options that you want set on every run, `spotify-ripper` uses a JSON config file at `~/.config/spotify-ripper/config.json` (honouring `$XDG_CONFIG_HOME`). On first run the full **default configuration** is written there, so you can open it and tweak whatever you like. The keys match the command-line option names with dashes turned into underscores; any option given on the command line overrides the value in the config file.

Here is an example config file:

```json
{
    "directory": "~/Music",
    "format": "{album_artist}/{album}/{artist} - {track_name}.{ext}",
    "quality": "160",
    "ascii": true,
    "all_artists": true
}
```

The file is always stored pretty-printed; you only need to include the keys you want to change. To regenerate the default configuration, just delete the file and run the tool again.

### Format String

The format string dictates how `spotify-ripper` will organize your ripped files. This is controlled through the `-f | --format` option. The string should include the format of the file name and optionally a directory structure. If you do not include a format string, the default format will be used: `{album_artist}/{album}/{artist} - {track_name}.{ext}`.

The `--flat` option is shorthand for using the format string: `{artist} - {track_name}.{ext}`, and the `--flat-with-index` option is shorthand for using the format string: `{idx:3} - {artist} - {track_name}.{ext}`.  The use of these shorthand options will override any `--format` string option given.

Your format string can include the following variables names, which are case-sensitive and wrapped in curly braces, if you want your file/path name to be overwritten with Spotify metadata.

### Format String Variables

Names and Aliases | Description |
----------------- | ----------- |
`{track_artist}`, `{artist}` | The track's artist |
`{track_artists}`, `{artists}`  | Similar to `{track_artist}` but will join multiple artists with a comma (e.g. "artist 1, artist 2") |
`{album_artist}` | When passing an album, the album's artist (e.g. "Various Artists"). If no album artist exists, the track artist is used instead |
`{album_artists_web}` | Similar to `{album_artist}` but retrieves the full list of artists credited on the album from Spotify. Unlike ``{album_artist}``, multiple album artists can be retrieved and will be joined with a comma (e.g. "artist 1, artist 2") |
`{album}` | Album name |
`{track_name}`, `{track}` | Track name |
`{year}` | Release year of the album |
`{ext}`, `{extension}` | Filename extension (i.e. "mp3", "ogg", "flac", ...) |
`{idx}`, `{index}` | Playlist index |
`{track_num}`, `{track_idx}`, `{track_index}` | The track number of the disc |
`{disc_num}`, `{disc_idx}`, `{disc_index}` | The disc number of the album |
`{smart_track_num}`, `{smart_track_idx}` | For a multi-disc album, `{smart_track_num}` will return a number combining the disc and `{smart_track_index}` the track number. e.g. for disc 2, track 4 it will return "204". For a single disc album, it will return the track num. |
`{playlist}`, `{playlist_name}` | Name of playlist if passed a playlist uri, otherwise "No Playlist" |
`{playlist_owner}`, `{playlist_user}`, `{playlist_username}` | User name of playlist's owner if passed a playlist uri, otherwise "No Playlist Owner" |
`{playlist_track_add_time}`, `{track_add_time}` | When the track was added to the playlist |
`{playlist_track_add_user}`, `{track_add_user}` | The user that added the track to the playlist |
`{user}`, `{username}` | Spotify username of logged-in user |
`{feat_artists}`, `{featuring_artists}` | Featuring artists join by commas (see Prefix String section below) |
`{copyright}` | Album copyright message |
`{label}`, `{copyright_holder}` | Album copyright message with the year removed at the start of the string if it exists |
`{track_uri}`, `{uri}` | Spotify track uri |

Any substring in the format string that does not match a variable above will be passed through to the file/path name unchanged.

### Zero-Filled Padding

Format variables that represent an index can be padded with zeros to a user-specified length.  For example, `{idx:3}` will produce the following output: 001, 002, 003, etc.  If no number is provided, no zero-filled padding will occur (e.g. 8, 9, 10, 11, ...). The variables that accept this option include `{idx}`, `{track_num}`, `{disc_num}`, `{smart_track_num}` and their aliases.

### Prefix String

Format variable `feat_artists` takes a prefix string to be prepended before the output. For example, `{feat_artists:featuring}` will produce the follow output `featuing Bruno Mars`.  If there are no featuring artists, the prefix string (and any preceding spaces) will not be included.

### Playlist Sync Option

By default, other than checking for an overwrite, `spotify-ripper` will not keep track of local files once they are ripped from Spotify. However, if you use the `--playlist-sync` option when passing a playlist URI, `spotify-ripper` will store a json file in your settings directory that keeps track of location of your ripped files for that playlist.

If at a later time, the playlist is changed on Spotify (i.e. songs reordered, removed or added), `spotify-ripper` will try to keep your local files "in sync" the playlist if you rerun the same command.  For example, if your format string is `{index} {artist} - {track_name}.{ext}`, it will rename is existing files so the index is correct. Note that with option set, `spotify-ripper` will delete a song that was previously on the playlist, but was removed but still exists on your local machine.  It does not affect files outside of the playlist and has no effect on non-playlist URIs.

If you want to redownload a playlist (for example with improved quality), you either need to remove the song files from your local or use the `--overwrite` option.

## Installation

### Fedora

1. Install the command-line tools. `ffmpeg` is required (plus `lame` for the default MP3 output); the rest are optional and only needed for their respective output formats:

   ```
   $ sudo dnf install ffmpeg lame
   $ sudo dnf install flac opus-tools sox fdkaac
   ```

2. Install the ripper with `pip` (this pulls in the Python dependencies automatically):

   ```
   $ pip3 install --user --upgrade git+https://github.com/scaronni/spotify-ripper
   ```

### Other systems

Install the system tools listed under [Prerequisites](#prerequisites) with your package manager, then install the ripper with `pip3`:

```
$ pip3 install --user --upgrade git+https://github.com/scaronni/spotify-ripper
```

### Prerequisites

A **Spotify Premium** account (required for 320 kbps; free accounts get 160 kbps).

Python libraries (installed automatically by pip):

-  [librespot](https://github.com/justin025/librespot-python)
-  [colorama](https://pypi.python.org/pypi/colorama)
-  [mutagen](https://mutagen.readthedocs.org/en/latest/)
-  [requests](https://pypi.org/project/requests/)
-  [schedule](https://pypi.org/project/schedule/)

System libraries and commands:

-  [ffmpeg](https://ffmpeg.org/download.html#releases) — required for every output format except native Ogg Vorbis (`--vorbis`); it decodes the downloaded Spotify stream to PCM
-  [lame](http://lame.sourceforge.net) — for MP3 output (the default)
-  [fdkaac](https://github.com/mstorsjo/fdk-aac/releases) (optional, for `--mp4`)
-  [flac](https://xiph.org/flac/index.html) (optional, for `--flac`)
-  [opus-tools](http://www.opus-codec.org/downloads/) (optional, for `--opus`)
-  [sox](http://sox.sourceforge.net) (optional, for `--aiff`)

After installing, pair once with `spotify-ripper --login` (see the [Login](#login) section).

## License

[MIT License](http://en.wikipedia.org/wiki/MIT_License)
