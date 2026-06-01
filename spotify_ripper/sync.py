# -*- coding: utf-8 -*-

from __future__ import unicode_literals

from colorama import Fore
from spotify_ripper.utils import *
import os
import json
import codecs
from spotify_ripper.librespot_session import SpotifyError


class Sync(object):

    def __init__(self, args, ripper):
        self.args = args
        self.ripper = ripper

    def sync_lib_path(self, playlist):
        # get playlist_id (last token of the spotify:playlist:ID uri)
        playlist_id = playlist.link.uri.split(':')[-1]
        if not playlist_id:
            return None

        # lib path
        lib_path = os.path.join(settings_dir(), "Sync")

        if not path_exists(lib_path):
            os.makedirs(enc_str(lib_path))

        return os.path.join(lib_path, playlist_id + ".json")

    def save_sync_library(self, playlist, lib):
        args = self.args
        lib_path = self.sync_lib_path(playlist)

        encoding = "ascii" if args.ascii else "utf-8"
        with codecs.open(enc_str(lib_path), 'w', encoding) as lib_file:
            lib_file.write(json.dumps(lib, ensure_ascii=args.ascii, indent=4, separators=(',', ': ')))

    def load_sync_library(self, playlist):
        args = self.args
        lib_path = self.sync_lib_path(playlist)

        if path_exists(lib_path):
            encoding = "ascii" if args.ascii else "utf-8"
            with codecs.open(enc_str(lib_path), 'r', encoding) as lib_file:
                return json.loads(lib_file.read())
        else:
            return {}

    def sync_playlist(self, playlist):
        playlist.load()
        lib = self.load_sync_library(playlist)
        new_lib = {}

        print("Syncing playlist " + to_ascii(playlist.name))

        # create new lib
        for idx, track in enumerate(playlist.tracks):
            try:
                track.load()
                if track.availability != 1 or track.is_local:
                    continue

                audio_file = self.ripper.format_track_path(idx, track)
                new_lib[track.link.uri] = audio_file

            except SpotifyError:
                continue

        # check what items are missing or renamed in the new_lib vs lib
        for uri, file_path in lib.items():
            enc_file_path = enc_str(file_path)

            if os.path.exists(enc_file_path):
                if uri in new_lib:
                    new_file_path = new_lib[uri]
                    if file_path != new_file_path:
                        print(Fore.YELLOW + "Renaming file:" + Fore.RESET + "\n  From: " + file_path + "\n  To:   " + new_file_path)
                        os.rename(enc_file_path, enc_str(new_file_path))
                else:
                    print(Fore.YELLOW + "Removing file:" + Fore.RESET + "\n " + file_path)
                    os.remove(enc_file_path)

        # save new lib
        self.save_sync_library(playlist, new_lib)
