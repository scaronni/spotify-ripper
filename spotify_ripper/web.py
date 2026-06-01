# -*- coding: utf-8 -*-

from __future__ import unicode_literals

from colorama import Fore
from spotify_ripper.librespot_session import uri_to_id
import requests


class WebAPI(object):
    """Thin helper for the handful of extra lookups the ripper needs over the
    librespot protocol (artist album lists, genres, album artists) plus cover
    art image downloads from the CDN."""

    def __init__(self, args, ripper):
        self.args = args
        self.ripper = ripper
        self.cache = {
            "albums_with_filter": {},
            "artists_on_album": {},
            "genres": {},
            "large_coverart": {}
        }

    @property
    def api(self):
        return self.ripper.api

    def cache_result(self, name, uri, result):
        self.cache[name][uri] = result

    def get_cached_result(self, name, uri):
        return self.cache[name].get(uri)

    # resolve an artist URI to the URIs of all their albums (filtered by
    # --artist-album-type, defaulting to album,single,compilation)
    def get_albums_with_filter(self, uri):
        args = self.args

        cached_result = self.get_cached_result("albums_with_filter", uri)
        if cached_result is not None:
            return cached_result

        artist_id = uri_to_id(uri)
        try:
            album_ids = self.api.get_artist_album_ids(
                artist_id, args.artist_album_type)
        except Exception as e:
            print(Fore.YELLOW + "Failed to load artist albums: " + str(e) +
                  Fore.RESET)
            return []

        album_uris = ["spotify:album:" + album_id for album_id in album_ids]
        print(str(len(album_uris)) + " albums found")
        self.cache_result("albums_with_filter", uri, album_uris)
        return album_uris

    def get_artists_on_album(self, uri):
        cached_result = self.get_cached_result("artists_on_album", uri)
        if cached_result is not None:
            return cached_result

        try:
            album = self.api.get_album_json(uri_to_id(uri))
        except Exception:
            return None

        result = [artist['name'] for artist in album.get('artists', [])]
        self.cache_result("artists_on_album", uri, result)
        return result

    # genre_type can be "artist" or "album"
    def get_genres(self, genre_type, track):
        if genre_type == "artist":
            item_id = track.artists[0]._id
            uri = track.artists[0].link.uri
        else:
            item_id = track.album._id
            uri = track.album.link.uri

        cached_result = self.get_cached_result("genres", uri)
        if cached_result is not None:
            return cached_result

        try:
            if genre_type == "artist":
                json_obj = self.api.get_artist(item_id)
            else:
                json_obj = self.api.get_album_json(item_id)
        except Exception:
            return None

        result = json_obj.get("genres", [])
        self.cache_result("genres", uri, result)
        return result

    def get_large_coverart(self, uri):
        cached_result = self.get_cached_result("large_coverart", uri)
        if cached_result is not None:
            return self._get_image_data(cached_result)

        try:
            track = self.api.get_track(uri_to_id(uri))
            url = track.album.cover(3).link  # XLARGE
        except Exception:
            print(Fore.RED + "Failed to retrieve track information, cover art "
                  "cannot be set" + Fore.RESET)
            return None

        if not url:
            return None
        self.cache_result("large_coverart", uri, url)
        return self._get_image_data(url)

    def _get_image_data(self, url):
        res = requests.get(url)
        if res.status_code == 200:
            return res.content
        return None
