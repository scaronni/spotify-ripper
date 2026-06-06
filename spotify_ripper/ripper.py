# -*- coding: utf-8 -*-

from __future__ import unicode_literals

from subprocess import Popen, PIPE
from colorama import Fore, Style
from spotify_ripper.utils import *
from spotify_ripper.tags import set_metadata_tags
from spotify_ripper.progress import Progress
from spotify_ripper.post_actions import PostActions
from spotify_ripper.web import WebAPI
from spotify_ripper.sync import Sync
from spotify_ripper.librespot_session import (
    SpotifyError, SpotifyAPI, create_session, login_via_zeroconf,
    has_stored_credentials, quality_for, uri_to_id, normalize_uri,
)
from datetime import datetime
import os
import sys
import time
import threading
import shutil
import itertools
import wave
import re
import traceback

# raw PCM decoded from the Ogg Vorbis stream is signed 16-bit stereo @ 44100 Hz
PCM_SAMPLE_RATE = 44100
PCM_FRAME_BYTES = 4  # 2 channels * 2 bytes
DOWNLOAD_CHUNK = 64 * 1024
DECODE_CHUNK = 16 * 1024


class Ripper(threading.Thread):
    name = 'SpotifyRipperThread'

    audio_file = None
    pcm_file = None
    wav_file = None
    rip_proc = None
    pipe = None
    current_playlist = None
    current_album = None

    session = None
    api = None
    user = None
    quality = None

    progress = None
    sync = None
    post = None
    web = None
    stop_time = None
    track_path_cache = {}

    # threading events
    ripper_continue = threading.Event()
    ripping = threading.Event()
    finished = threading.Event()
    abort = threading.Event()
    skip = threading.Event()

    def __init__(self, args):
        threading.Thread.__init__(self)

        # initialize progress meter
        self.progress = Progress(args, self)

        self.args = args

        self.post = PostActions(args, self)
        self.web = WebAPI(args, self)

        if not path_exists(settings_dir()):
            os.makedirs(enc_str(settings_dir()))

    # executes on main thread (not SpotifyRipper thread)
    def login(self):
        """Pair via Zeroconf if requested, then create the librespot session
        from the stored credentials."""
        args = self.args

        if getattr(args, "do_login", False):
            if not login_via_zeroconf():
                return False

        if not has_stored_credentials():
            print(Fore.RED + "No stored Spotify credentials found. Run with "
                  "--login to pair with the Spotify app first." + Fore.RESET)
            return False

        try:
            self.session = create_session()
            self.api = SpotifyAPI(self.session)
            self.user = self.api.user()
        except Exception as e:
            print(Fore.RED + "Failed to create Spotify session: " + str(e)
                  + Fore.RESET)
            return False

        print(Fore.GREEN + "Logged in as " + str(self.user.display_name)
              + "\n" + Fore.RESET)
        return True

    def run(self):
        args = self.args

        # wait for main thread to login
        self.ripper_continue.wait()
        if self.abort.is_set():
            return

        # pick the Ogg Vorbis quality based on the account type
        self.quality = quality_for(args.quality, self.api.account_type())

        # list of spotify URIs
        uris = args.uri

        # per-uri playlist/album context so file naming is correct
        uri_context = {}

        def get_tracks_from_uri(uri):
            self.current_playlist = None
            self.current_album = None

            # accept both spotify: URIs and open.spotify.com URLs
            uri = normalize_uri(uri)
            return list(self.load_link(uri))

        # calculate total size and time
        all_tracks = []
        all_uris = {}
        for uri_idx, uri in enumerate(uris):
            all_uris[uri_idx] = get_tracks_from_uri(uri)
            uri_context[uri_idx] = (self.current_playlist,
                                    self.current_album)
            tracks = all_uris[uri_idx]

            for idx, track in enumerate(tracks):
                print('Loading track {}/{}...'.format(idx, len(tracks)),
                      end='\r')
                sys.stdout.flush()
                if track.is_local:
                    continue
                audio_file = self.format_track_path(idx, track)
                all_tracks.append((track, audio_file))
            print('Loading track {}/{}...'.format(len(tracks), len(tracks)))

        self.progress.calc_total(all_tracks)

        if self.progress.total_size > 0:
            print("Total Download Size: " + format_size(self.progress.total_size))

        # pin the live status block to the bottom of the screen (only when
        # there is something to rip and we are on an interactive terminal)
        if self.progress.total_tracks > 0:
            self.progress.setup()

        # ripping loop
        for uri_idx, uri in enumerate(uris):
            if self.abort.is_set():
                break

            tracks = all_uris[uri_idx]

            # restore the naming context captured while loading this uri
            (self.current_playlist, self.current_album) = \
                uri_context.get(uri_idx, (None, None))

            if args.playlist_sync and self.current_playlist:
                self.sync = Sync(args, self)
                self.sync.sync_playlist(self.current_playlist)

            for idx, track in enumerate(tracks):
                try:
                    self.check_stop_time()
                    self.skip.clear()

                    if self.abort.is_set():
                        break

                    prefix = self.progress.counter_prefix()
                    indent = self.progress.indent()

                    if track.availability != 1 or track.is_local:
                        print(prefix + Fore.RED + "Unavailable " +
                              track.link.uri + Fore.RESET)
                        self.post.log_failure(track)
                        self.progress.track_idx += 1
                        continue

                    self.audio_file = self.format_track_path(idx, track)

                    if not args.overwrite and path_exists(self.audio_file):
                        if is_partial(self.audio_file, track):
                            print("Overwriting partial file")
                        else:
                            print(prefix + Fore.CYAN + Style.BRIGHT +
                                  "Skipping " + track.link.uri + Style.NORMAL +
                                  Fore.RESET)
                            print(format_field(indent, "File name",
                                               self.rel_path()))
                            self.post.log_skipped(track)
                            self.progress.track_idx += 1
                            continue

                    print(prefix + Fore.GREEN + Style.BRIGHT + "Ripping " +
                          track.link.uri + Style.NORMAL + Fore.RESET)
                    print(format_field(indent, "File name", self.rel_path()))
                    self.rip_track(idx, track)

                    if self.skip.is_set():
                        print(Fore.YELLOW + "User skipped track..." +
                              Fore.RESET)
                        self.abort_sinks()
                        self.post.clean_up_partial()
                        self.post.log_failure(track)
                        self.progress.end_track(show_end=False)
                        continue

                    if self.abort.is_set():
                        self.abort_sinks()
                        self.post.clean_up_partial()
                        self.post.log_failure(track)
                        break

                    self.finish_rip(track)

                    # update tags and embed front cover image
                    set_metadata_tags(args, self.audio_file, idx, track, self)

                    self.post.log_success(track)

                except (SpotifyError, Exception) as e:
                    print(Fore.RED + "Error while ripping track" + Fore.RESET)
                    print(str(e))
                    traceback.print_exc()
                    print("Skipping to next track...")
                    self.abort_sinks()
                    self.post.clean_up_partial()
                    self.post.log_failure(track)
                    continue

            # create playlist m3u/wpl files if needed
            self.post.create_playlist_m3u(tracks)
            self.post.create_playlist_wpl(tracks)

        # done -- release the pinned status block before the summary
        self.progress.teardown()
        self.post.cleanup_offline_cache()
        self.post.end_failure_log()
        self.post.print_summary()
        self.finished.set()
        sys.exit()

    def check_stop_time(self):
        args = self.args

        def wait_for_resume(resume_time):
            while datetime.now() < resume_time and not self.abort.is_set():
                time.sleep(1)

        def stop_time_triggered():
            print(Fore.YELLOW + "Stop time of " +
                  self.stop_time.strftime("%H:%M") +
                  " has been triggered, stopping..." + Fore.RESET)

            if args.resume_after is not None:
                resume_time = parse_time_str(args.resume_after)
                print(Fore.YELLOW + "Script will resume at " +
                      resume_time.strftime("%H:%M") + Fore.RESET)
                wait_for_resume(resume_time)
                self.stop_time = None
            else:
                self.abort.set()

        if args.stop_after is not None:
            if self.stop_time is None:
                self.stop_time = parse_time_str(args.stop_after)
                print(Fore.YELLOW + "Script will stop after " +
                      self.stop_time.strftime("%H:%M") + Fore.RESET)

            if self.stop_time < datetime.now():
                stop_time_triggered()

    def load_link(self, uri):
        # ignore if the uri is just blank (e.g. from a file)
        if not uri:
            return iter([])

        item_id = uri_to_id(uri)

        if uri.startswith("spotify:track:"):
            return iter([self.api.get_track(item_id)])
        elif uri.startswith("spotify:playlist:"):
            self.current_playlist = self.api.get_playlist(item_id)
            return iter(self.current_playlist.tracks)
        elif uri.startswith("spotify:album:"):
            self.current_album = self.api.get_album(item_id)
            return iter(self.api.get_album_tracks_as_tracks(item_id))
        elif uri.startswith("spotify:artist:"):
            # rip the artist's full discography, filtered by
            # --artist-album-type (default: album,single,compilation)
            album_uris = self.web.get_albums_with_filter(uri)
            return itertools.chain(*[
                iter(self.api.get_album_tracks_as_tracks(uri_to_id(au)))
                for au in album_uris])
        return iter([])

    def format_track_path(self, idx, track):
        args = self.args

        if track.link.uri in self.track_path_cache:
            return self.track_path_cache[track.link.uri]

        audio_file = \
            format_track_string(self, args.format.strip(), idx, track)

        def truncate(_str, max_size):
            return _str[:max_size].strip() if len(_str) > max_size else _str

        def truncate_dir_path(dir_path):
            path_tokens = dir_path.split(os.sep)
            path_tokens = [truncate(token, 255) for token in path_tokens]
            return os.sep.join(path_tokens)

        def truncate_file_name(file_name):
            tokens = file_name.rsplit(os.extsep, 1)
            if len(tokens) > 1:
                tokens[0] = truncate(tokens[0], 255 - len(tokens[1]) - 1)
            else:
                tokens[0] = truncate(tokens[0], 255)
            return os.extsep.join(tokens)

        if args.windows_safe:
            tokens = audio_file.rsplit(os.sep, 1)
            if len(tokens) > 1:
                audio_file = os.path.join(truncate_dir_path(tokens[0]),
                                          truncate_file_name(tokens[1]))
            else:
                audio_file = truncate_file_name(tokens[0])

        if args.replace is not None:
            audio_file = self.replace_filename(audio_file, args.replace)

        if args.windows_safe:
            audio_file = re.sub('[:"*?<>|]', '', audio_file)

        audio_file = to_ascii(os.path.join(base_dir(), audio_file))

        if args.normalized_ascii:
            audio_file = to_normalized_ascii(audio_file)

        audio_path = os.path.dirname(audio_file)
        if not path_exists(audio_path):
            os.makedirs(enc_str(audio_path))

        self.track_path_cache[track.link.uri] = audio_file
        return audio_file

    def replace_filename(self, filename, pattern_list):
        for pattern in pattern_list:
            repl = pattern.split('/')
            filename = re.sub(repl[0], repl[1], filename)
        return filename

    def rel_path(self):
        """The current audio file path relative to the output directory
        (which is already shown once in the header)."""
        try:
            return os.path.relpath(self.audio_file, base_dir())
        except (ValueError, TypeError):
            return self.audio_file

    # -- audio download + encode --------------------------------------------

    def rip_track(self, idx, track):
        """Download the native Ogg Vorbis stream, then produce the requested
        output format from it."""
        args = self.args

        self.progress.prepare_track(track)
        # mark as ripping up front so the Esc-to-skip handler works during the
        # download phase too (prepare_rip below also sets it)
        self.ripping.set()

        track_id = uri_to_id(track.link.uri)
        temp_ogg = self.audio_file + ".part.ogg"

        # 1) download the encrypted Ogg Vorbis stream (librespot decrypts it)
        stream = self.api.load_stream(track_id, self.quality)
        total_size = stream.input_stream.size
        self.progress.set_download_size(total_size)

        downloaded = 0
        with open(enc_str(temp_ogg), 'wb') as ogg:
            while downloaded < total_size:
                if self.abort.is_set() or self.skip.is_set():
                    break
                data = stream.input_stream.stream().read(DOWNLOAD_CHUNK)
                if not data:
                    break
                ogg.write(data)
                downloaded += len(data)
                self.progress.set_download_progress(downloaded, total_size)
        try:
            stream.input_stream.stream().close()
        except Exception:
            pass

        if self.abort.is_set() or self.skip.is_set():
            rm_file(temp_ogg)
            return

        # 2) build encoder / wav / pcm sinks (everything except native ogg)
        self.prepare_rip(idx, track)

        # 3a) native Ogg Vorbis: just keep the downloaded stream as-is
        if args.output_type == "ogg":
            shutil.copyfile(enc_str(temp_ogg), enc_str(self.audio_file))

        # 3b) anything that needs PCM (encoders, wav, pcm, plus_wav, plus_pcm):
        #     decode the ogg to raw PCM with ffmpeg and feed the sinks
        if self.pipe is not None or self.wav_file is not None or \
                self.pcm_file is not None:
            self.decode_and_feed(temp_ogg)

        rm_file(temp_ogg)

    def decode_and_feed(self, temp_ogg):
        """Decode the Ogg Vorbis temp file to raw PCM via ffmpeg and push the
        frames through the existing rip() sinks."""
        decode_proc = Popen(
            ["ffmpeg", "-nostdin", "-loglevel", "quiet",
             "-i", enc_str(temp_ogg),
             "-f", "s16le", "-ar", str(PCM_SAMPLE_RATE), "-ac", "2", "-"],
            stdout=PIPE)
        try:
            while True:
                if self.abort.is_set() or self.skip.is_set():
                    decode_proc.kill()
                    break
                chunk = decode_proc.stdout.read(DECODE_CHUNK)
                if not chunk:
                    break
                num_frames = len(chunk) // PCM_FRAME_BYTES
                self.progress.update_progress(num_frames, PCM_SAMPLE_RATE)
                self.rip(self.session, PCM_SAMPLE_RATE, chunk, num_frames)
        finally:
            try:
                decode_proc.stdout.close()
            except Exception:
                pass
            decode_proc.wait()

    def prepare_rip(self, idx, track):
        args = self.args

        if args.output_type == "wav" or args.plus_wav:
            audio_file = change_file_extension(self.audio_file, "wav") if \
                args.output_type != "wav" else self.audio_file
            self.wav_file = wave.open(enc_str(audio_file), "wb")
            self.wav_file.setparams(
                (2, 2, PCM_SAMPLE_RATE, 0, 'NONE', 'not compressed'))

        if args.output_type == "pcm" or args.plus_pcm:
            audio_file = change_file_extension(self.audio_file, "pcm") if \
                args.output_type != "pcm" else self.audio_file
            self.pcm_file = open(enc_str(audio_file), 'wb')

        audio_file_enc = enc_str(self.audio_file)

        if args.output_type == "flac":
            self.rip_proc = Popen(["flac", "-f", ("-" + str(args.comp)),
                                   "--silent", "--endian", "little",
                                   "--channels", "2", "--bps", "16",
                                   "--sample-rate", str(PCM_SAMPLE_RATE),
                                   "--sign", "signed", "-o", audio_file_enc,
                                   "-"], stdin=PIPE)
        elif args.output_type == "aiff":
            self.rip_proc = Popen(["sox", "-q", "--endian", "little",
                                   "--channels", "2", "--bits", "16",
                                   "--rate", str(PCM_SAMPLE_RATE),
                                   "--encoding", "unsigned-integer", "-t",
                                   "raw", "-", audio_file_enc], stdin=PIPE)
        elif args.output_type == "alac.m4a":
            self.rip_proc = Popen(["ffmpeg", "-nostats", "-loglevel", "0",
                                   "-f", "s16le", "-ar", str(PCM_SAMPLE_RATE),
                                   "-ac", "2", "-channel_layout", "stereo",
                                   "-i", "-", "-acodec", "alac",
                                   audio_file_enc], stdin=PIPE)
        elif args.output_type == "opus":
            if args.cbr:
                self.rip_proc = Popen(["opusenc", "--quiet", "--comp",
                                       args.comp, "--cvbr", "--bitrate",
                                       str(int(args.bitrate) / 2), "--raw",
                                       "--raw-rate", str(PCM_SAMPLE_RATE), "-",
                                       audio_file_enc], stdin=PIPE)
            else:
                self.rip_proc = Popen(["opusenc", "--quiet", "--comp",
                                       args.comp, "--vbr", "--bitrate",
                                       args.vbr, "--raw", "--raw-rate",
                                       str(PCM_SAMPLE_RATE), "-",
                                       audio_file_enc], stdin=PIPE)
        elif args.output_type == "m4a":
            if args.cbr:
                self.rip_proc = Popen(["fdkaac", "-S", "-R", "-b", args.bitrate,
                                       "-o", audio_file_enc, "-"], stdin=PIPE)
            else:
                self.rip_proc = Popen(["fdkaac", "-S", "-R", "-m", args.vbr,
                                       "-o", audio_file_enc, "-"], stdin=PIPE)
        elif args.output_type == "mp3":
            lame_args = ["lame", "--silent"]
            if args.stereo_mode is not None:
                lame_args.extend(["-m", args.stereo_mode])
            if args.cbr:
                lame_args.extend(["-cbr", "-b", args.bitrate])
            else:
                lame_args.extend(["-V", args.vbr])
            lame_args.extend(["-h", "-r", "-", audio_file_enc])
            self.rip_proc = Popen(lame_args, stdin=PIPE)

        if self.rip_proc is not None:
            self.pipe = self.rip_proc.stdin

        self.ripping.set()

    def finish_rip(self, track):
        self.progress.end_track()
        if self.pipe is not None:
            self.pipe.flush()
            self.pipe.close()

            ret_code = self.rip_proc.wait()
            if ret_code != 0:
                print(Fore.YELLOW + "Warning: encoder returned non-zero error "
                      "code " + str(ret_code) + Fore.RESET)
            self.rip_proc = None
            self.pipe = None

        if self.wav_file is not None:
            self.wav_file.close()
            self.wav_file = None

        if self.pcm_file is not None:
            self.pcm_file.flush()
            os.fsync(self.pcm_file.fileno())
            self.pcm_file.close()
            self.pcm_file = None

        self.ripping.clear()

    def rip(self, session, sample_rate, frame_bytes, num_frames):
        if self.ripping.is_set():
            if self.pipe is not None:
                self.pipe.write(frame_bytes)
            if self.wav_file is not None:
                self.wav_file.writeframes(frame_bytes)
            if self.pcm_file is not None:
                self.pcm_file.write(frame_bytes)

    def abort_sinks(self):
        """Tear down any open encoder/wav/pcm sinks without finalizing them
        (used when a track is skipped or aborted mid-rip)."""
        if self.rip_proc is not None:
            try:
                self.pipe.close()
            except Exception:
                pass
            try:
                self.rip_proc.kill()
            except Exception:
                pass
            self.rip_proc = None
            self.pipe = None
        if self.wav_file is not None:
            try:
                self.wav_file.close()
            except Exception:
                pass
            self.wav_file = None
        if self.pcm_file is not None:
            try:
                self.pcm_file.close()
            except Exception:
                pass
            self.pcm_file = None
        self.ripping.clear()

    def abort_rip(self):
        self.ripping.clear()
        self.abort.set()
