# -*- coding: utf-8 -*-

from __future__ import unicode_literals

from colorama import Fore
from spotify_ripper.utils import *
import os
import codecs
import shutil

class PostActions(object):
    fail_log_file = None
    success_tracks = []
    failure_tracks = []

    def __init__(self, args, ripper):
        self.args = args
        self.ripper = ripper

        # create a log file for rip failures
        if args.fail_log is not None:
            if not path_exists(base_dir()):
                os.makedirs(enc_str(base_dir()))

            encoding = "ascii" if args.ascii else "utf-8"
            self.fail_log_file = codecs.open(enc_str(os.path.join(base_dir(), args.fail_log)), 'w', encoding)

    def log_success(self, track):
        self.success_tracks.append(track)

    def log_failure(self, track):
        self.failure_tracks.append(track)
        if self.fail_log_file is not None:
            self.fail_log_file.write(track.link.uri + "\n")

    def end_failure_log(self):
        if self.fail_log_file is not None:
            file_name = self.fail_log_file.name
            self.fail_log_file.flush()
            os.fsync(self.fail_log_file.fileno())
            self.fail_log_file.close()
            self.fail_log_file = None

            if os.path.getsize(enc_str(file_name)) == 0:
                rm_file(file_name)

    def print_summary(self):
        if len(self.success_tracks) + len(self.failure_tracks) <= 1:
            return

        def print_with_bullet(_str):
            if self.args.ascii:
                print(" * " + _str)
            else:
                print(" • " + _str)

        def log_tracks(tracks):
            for track in tracks:
                try:
                    track.load()
                    if (len(track.artists) > 0 and track.artists[0].name is not None and track.name is not None):
                        print_with_bullet(track.artists[0].name + " - " + track.name)
                    else:
                        print_with_bullet(track.link.uri)
                except Exception:
                    print_with_bullet(track.link.uri)
            print("")

        if len(self.success_tracks) > 0:
            print(Fore.GREEN + "\nSuccess Summary (" + str(len(self.success_tracks)) + ")\n" + ("-" * 79) + Fore.RESET)
            log_tracks(self.success_tracks)
        if len(self.failure_tracks) > 0:
            print(Fore.RED + "\nFailure Summary (" + str(len(self.failure_tracks)) + ")\n" + ("-" * 79) + Fore.RESET)
            log_tracks(self.failure_tracks)

    def get_playlist_name(self):
        ripper = self.ripper

        if ripper.current_playlist is not None:
            return ripper.current_playlist.name
        elif ripper.current_album is not None:
            return (ripper.current_album.artist.name + " - " + ripper.current_album.name)
        else:
            return None

    def create_playlist_m3u(self, tracks):
        args = self.args
        ripper = self.ripper

        name = self.get_playlist_name()
        if args.playlist_m3u:
            if name is None:
                name = "0_playlist"
            name = sanitize_playlist_name(to_ascii(name))
            playlist_path = to_ascii(os.path.join(base_dir(), name + '.m3u'))

            print(Fore.GREEN + "Creating playlist m3u file " + playlist_path + Fore.RESET)

            encoding = "ascii" if args.ascii else "utf-8"
            with codecs.open(enc_str(playlist_path), 'w', encoding) as playlist:
                for idx, track in enumerate(tracks):
                    track.load()
                    if track.is_local:
                        continue
                    _file = ripper.format_track_path(idx, track)
                    if path_exists(_file):
                        playlist.write(os.path.relpath(_file, base_dir()) + "\n")

    def create_playlist_wpl(self, tracks):
        args = self.args
        ripper = self.ripper

        name = self.get_playlist_name()
        if name is not None and args.playlist_wpl:
            name = sanitize_playlist_name(to_ascii(name))
            playlist_path = to_ascii(os.path.join(base_dir(), name + '.wpl'))

            print(Fore.GREEN + "Creating playlist wpl file " + playlist_path + Fore.RESET)

            encoding = "ascii" if args.ascii else "utf-8"
            with codecs.open(enc_str(playlist_path), 'w', encoding) as playlist:
                # to get an accurate track count
                track_paths = []
                for idx, track in enumerate(tracks):
                    track.load()
                    if track.is_local:
                        continue
                    _file = ripper.format_track_path(idx, track)
                    if path_exists(_file):
                        track_paths.append(_file)

                playlist.write('<?wpl version="1.0"?>\n')
                playlist.write('<smil>\n')
                playlist.write('\t<head>\n')
                playlist.write('\t\t<meta name="Generator" ' 'content="Microsoft Windows Media Player -- ' '12.0.7601.18526"/>\n')
                playlist.write('\t\t<meta name="ItemCount" content="' + str(len(track_paths)) + '"/>\n')
                playlist.write('\t\t<author>' + ripper.user.display_name + '</author>\n')
                playlist.write('\t\t<title>' + name + '</title>\n')
                playlist.write('\t</head>\n')
                playlist.write('\t<body>\n')
                playlist.write('\t\t<seq>\n')
                for _file in track_paths:
                    _file.replace("&", "&amp;")
                    _file.replace("'", "&apos;")
                    playlist.write('\t\t\t<media src="' + os.path.relpath(_file, base_dir()) + "\"/>\n")
                playlist.write('\t\t</seq>\n')
                playlist.write('\t</body>\n')
                playlist.write('</smil>\n')

    def clean_up_partial(self):
        ripper = self.ripper

        if ripper.audio_file is not None and path_exists(ripper.audio_file):
            print(Fore.YELLOW + "Deleting partially ripped file" + Fore.RESET)
            rm_file(ripper.audio_file)

            # check for any extra pcm or wav files
            def delete_extra_file(ext):
                audio_file = change_file_extension(ripper.audio_file, ext)
                if path_exists(audio_file):
                    rm_file(audio_file)

            if self.args.plus_wav:
                delete_extra_file("wav")

            if self.args.plus_pcm:
                delete_extra_file("pcm")

    def cleanup_offline_cache(self):
        # delete librespot's offline audio cache after a successful rip unless
        # the user asked to keep it
        if self.args.keep_offline_cache:
            return
        cache_path = cache_dir()
        if path_exists(cache_path):
            shutil.rmtree(enc_str(cache_path), ignore_errors=True)
