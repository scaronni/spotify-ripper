# -*- coding: utf-8 -*-

# Spotify access via librespot: logging in, fetching metadata and downloading
# the audio stream.
#
# Login happens over Zeroconf: the ripper advertises itself as a Spotify
# Connect device and you pick it once from the official Spotify app; the
# resulting reusable credentials are stored on disk and reused on later runs.
#
# Metadata is fetched over librespot's native protocol (the Mercury channel).
# The protobuf responses are wrapped in small shim objects that expose plain
# track/album/artist/playlist attributes for the rest of spotify-ripper
# (tags.py, utils.py, progress.py, ...).

from __future__ import unicode_literals

import json
import os
import time

from librespot.audio.decoders import AudioQuality, VorbisOnlyAudioQuality
from librespot.core import Session
from librespot.metadata import (
    TrackId, AlbumId, ArtistId, PlaylistId,
)
from librespot.zeroconf import ZeroconfServer

# The client id advertised for the Spotify Connect device.
SPOTIFY_CLIENT_ID = "65b708073fc0480ea92a077233ca87bd"
# Image host (CDN) for album cover art referenced by the protocol metadata.
IMAGE_URL = "https://i.scdn.co/image/"


class SpotifyError(Exception):
    """Error raised for Spotify access and metadata failures."""
    pass


# ---------------------------------------------------------------------------
# Login / session handling
# ---------------------------------------------------------------------------

def credentials_path():
    from spotify_ripper.utils import settings_dir
    return os.path.join(settings_dir(), "credentials.json")


def prettify_json_file(path):
    """Rewrite a JSON file in pretty-printed form so credentials.json stays
    consistently formatted with the other config files."""
    try:
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
        with open(path, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=4, sort_keys=True, ensure_ascii=False)
            f.write("\n")
    except (ValueError, IOError):
        pass


def has_stored_credentials():
    path = credentials_path()
    return os.path.isfile(path) and os.path.getsize(path) > 0


def login_via_zeroconf(device_name="spotify-ripper", timeout=300):
    """Advertise as a Spotify Connect device and wait for the user to select
    it in the official Spotify app.  On success the reusable credentials are
    written to ``credentials_path()`` and True is returned."""
    path = credentials_path()
    os.makedirs(os.path.dirname(path), exist_ok=True)

    ZeroconfServer._ZeroconfServer__default_get_info_fields["clientID"] = \
        SPOTIFY_CLIENT_ID
    builder = ZeroconfServer.Builder()
    builder.device_name = device_name
    builder.conf.stored_credentials_file = path
    zs = builder.create()

    print("Waiting for Spotify Connect login...")
    print("Open the official Spotify app, then in the device/speaker picker "
          "select '" + device_name + "'.")

    deadline = time.time() + timeout
    try:
        while time.time() < deadline:
            time.sleep(1)
            if zs.has_valid_session():
                username = zs._ZeroconfServer__session.username()
                print("Paired with Spotify account: " + str(username))
                # The Zeroconf server has already written the credentials file
                # at builder.conf.stored_credentials_file; re-write it pretty.
                if os.path.isfile(path):
                    prettify_json_file(path)
                    return True
                return False
    finally:
        zs.close()

    print("Timed out waiting for Spotify Connect login.")
    return False


def create_session():
    """Create a librespot Session from the stored credentials file."""
    from spotify_ripper.utils import cache_dir
    path = credentials_path()
    if not has_stored_credentials():
        raise SpotifyError("No stored Spotify credentials. Run with --login "
                           "first to pair via the Spotify app.")
    # store librespot's audio cache in a known, predictable location
    cache = cache_dir()
    os.makedirs(cache, exist_ok=True)
    conf = Session.Configuration.Builder() \
        .set_stored_credential_file(path) \
        .set_cache_enabled(True) \
        .set_cache_dir(cache) \
        .build()
    return Session.Builder(conf=conf).stored_file(path).create()


def quality_for(arg_quality, account_type):
    """Map the -Q/--quality CLI value to a librespot AudioQuality, downgrading
    if the account is not premium."""
    mapping = {
        "320": AudioQuality.VERY_HIGH,
        "160": AudioQuality.HIGH,
        "96": AudioQuality.NORMAL,
    }
    quality = mapping.get(str(arg_quality), AudioQuality.HIGH)
    if quality == AudioQuality.VERY_HIGH and account_type != "premium":
        print("Account is not premium; falling back to 160 kbps stream.")
        quality = AudioQuality.HIGH
    return quality


# ---------------------------------------------------------------------------
# id helpers
# ---------------------------------------------------------------------------

def uri_to_id(uri):
    return uri.split(":")[-1]


# matches https://open.spotify.com/[intl-xx/]<type>/<id>[?si=...]
import re as _re
_URL_RE = _re.compile(
    r"open\.spotify\.com/(?:intl-[a-z]+/)?"
    r"(track|album|playlist|artist)/([A-Za-z0-9]+)")


def normalize_uri(uri):
    """Accept either a spotify: URI or an open.spotify.com URL and return the
    canonical ``spotify:<type>:<id>`` URI.  Unknown inputs are returned as-is
    (load_link will then ignore them)."""
    if uri is None:
        return ""
    uri = uri.strip()
    if uri.startswith("spotify:"):
        return uri.split("?")[0]
    match = _URL_RE.search(uri)
    if match:
        return "spotify:%s:%s" % (match.group(1), match.group(2))
    return uri


def _base62(spotify_id):
    """Return the 22-char base62 id from a librespot *Id object."""
    return spotify_id.to_spotify_uri().split(":")[-1]


def _gid_to_base62(id_cls, gid):
    return _base62(id_cls.from_hex(gid.hex()))


# ---------------------------------------------------------------------------
# shim objects (built from protobuf metadata)
# ---------------------------------------------------------------------------

class Link(object):
    def __init__(self, uri):
        self.uri = uri

    def __str__(self):
        return self.uri


class Cover(object):
    def __init__(self, url):
        # tags.py reads the direct https URL straight from .link
        self.link = url


class Artist(object):
    is_loaded = True

    def __init__(self, api, pb):
        self._api = api
        self.name = pb.name
        self._id = None
        if pb.gid:
            try:
                self._id = _gid_to_base62(ArtistId, pb.gid)
            except Exception:
                self._id = None
        self.link = Link("spotify:artist:%s" % self._id)

    def load(self, timeout=None):
        return self


class _BrowseTrack(object):
    def __init__(self, disc, index):
        self.disc = disc
        self.index = index


class AlbumBrowser(object):
    is_loaded = True

    def __init__(self, api, album):
        self._api = api
        self._album = album
        self.tracks = []
        self.copyrights = []

    def load(self, timeout=None):
        pb = self._album._full_pb()
        tracks = []
        for disc in pb.disc:
            disc_num = disc.number or 1
            for i, _track in enumerate(disc.track):
                tracks.append(_BrowseTrack(disc_num, i + 1))
        self.tracks = tracks
        self.copyrights = [c.text for c in pb.copyright]
        return self


class Album(object):
    is_loaded = True

    def __init__(self, api, base62, name=None, pb=None):
        self._api = api
        self._id = base62
        self.name = name
        self._pb = pb
        self.link = Link("spotify:album:%s" % base62)

    def load(self, timeout=None):
        return self

    def _full_pb(self):
        # the album embedded in a track is partial (no disc list); fetch full
        if self._pb is None or not self._pb.disc:
            self._pb = self._api._album_meta(self._id)
            if not self.name:
                self.name = self._pb.name
        return self._pb

    @property
    def artist(self):
        pb = self._full_pb()
        if pb.artist:
            return Artist(self._api, pb.artist[0])
        return Artist(self._api, _empty_artist())

    @property
    def year(self):
        return self._full_pb().date.year

    def cover(self, size=2):
        pb = self._full_pb()
        images = list(pb.cover_group.image) if pb.cover_group.image \
            else list(pb.cover)
        if not images:
            return Cover(None)
        target = 3 if size >= 3 else 2  # LARGE=2, XLARGE=3
        chosen = None
        for img in images:
            if img.size == target:
                chosen = img
                break
        if chosen is None:
            chosen = max(images, key=lambda i: i.size)
        return Cover(IMAGE_URL + chosen.file_id.hex())

    def browse(self):
        return AlbumBrowser(self._api, self)


class Track(object):
    is_loaded = True

    def __init__(self, api, base62, pb):
        self._api = api
        self._id = base62
        self._pb = pb
        self.name = pb.name
        self.duration = pb.duration
        self.index = pb.number
        self.disc = pb.disc_number
        self.popularity = pb.popularity
        self.is_local = False
        # rely on the download step to fail for genuinely unavailable tracks
        self.availability = 1
        self.artists = [Artist(api, a) for a in pb.artist]
        album_b62 = _gid_to_base62(AlbumId, pb.album.gid)
        self.album = Album(api, album_b62, pb.album.name, pb.album)
        self.link = Link("spotify:track:%s" % base62)

    def load(self, timeout=None):
        return self


class User(object):
    def __init__(self, data):
        self.canonical_name = data.get("id")
        self.display_name = data.get("display_name") or data.get("id")


class Playlist(object):
    is_loaded = True
    tracks_with_metadata = []

    def __init__(self, api, base62, pb):
        self._api = api
        self._id = base62
        self.name = pb.attributes.name
        owner = pb.owner_username
        self.owner = User({"id": owner, "display_name": owner})
        self.link = Link("spotify:playlist:%s" % base62)
        self._track_uris = [item.uri for item in pb.contents.items]
        self._tracks = None

    def load(self, timeout=None):
        return self

    @property
    def tracks(self):
        if self._tracks is None:
            out = []
            for uri in self._track_uris:
                if uri.startswith("spotify:track:"):
                    try:
                        out.append(self._api.get_track(uri_to_id(uri)))
                    except Exception:
                        continue
            self._tracks = out
        return self._tracks


def _empty_artist():
    class _E(object):
        name = None
        gid = b""
    return _E()


# ---------------------------------------------------------------------------
# Metadata + audio access
# ---------------------------------------------------------------------------

class SpotifyAPI(object):
    def __init__(self, session):
        self.session = session
        self._album_cache = {}
        self._artist_cache = {}

    # -- session-derived -----------------------------------------------------
    def account_type(self):
        return self.session.get_user_attribute("type")

    def user(self):
        # the logged-in user, taken from the session
        username = self.session.username()
        return User({"id": username, "display_name": username})

    # -- protocol metadata ---------------------------------------------------
    def _album_meta(self, base62):
        if base62 not in self._album_cache:
            self._album_cache[base62] = self.session.api() \
                .get_metadata_4_album(AlbumId.from_base62(base62))
        return self._album_cache[base62]

    def _artist_meta(self, base62):
        if base62 not in self._artist_cache:
            self._artist_cache[base62] = self.session.api() \
                .get_metadata_4_artist(ArtistId.from_base62(base62))
        return self._artist_cache[base62]

    def get_track(self, base62):
        pb = self.session.api().get_metadata_4_track(
            TrackId.from_base62(base62))
        return Track(self, base62, pb)

    def get_album(self, base62):
        pb = self._album_meta(base62)
        return Album(self, base62, pb.name, pb)

    def get_album_tracks_as_tracks(self, base62):
        pb = self._album_meta(base62)
        tracks = []
        for disc in pb.disc:
            for track_ref in disc.track:
                try:
                    tb62 = _gid_to_base62(TrackId, track_ref.gid)
                    tracks.append(self.get_track(tb62))
                except Exception:
                    continue
        return tracks

    def get_artist_album_ids(self, base62, album_type=None):
        pb = self._artist_meta(base62)
        groups = {
            "album": pb.album_group,
            "single": pb.single_group,
            "compilation": pb.compilation_group,
            "appears_on": pb.appears_on_group,
        }
        if album_type:
            wanted = [t.strip() for t in album_type.split(",")]
        else:
            wanted = ["album", "single", "compilation"]
        ids = []
        for key in wanted:
            for album_group in groups.get(key, []):
                for album_ref in album_group.album:
                    try:
                        ids.append(_gid_to_base62(AlbumId, album_ref.gid))
                    except Exception:
                        continue
        return ids

    def get_artist(self, base62):
        # used by web.get_genres
        pb = self._artist_meta(base62)
        return {"genres": list(pb.genre)}

    def get_album_json(self, base62):
        # used by web.get_genres / get_artists_on_album
        pb = self._album_meta(base62)
        return {
            "genres": list(pb.genre),
            "artists": [{"name": a.name} for a in pb.artist],
        }

    def get_playlist(self, base62):
        pb = self.session.api().get_playlist(PlaylistId(base62))
        return Playlist(self, base62, pb)

    # -- audio download ------------------------------------------------------
    def load_stream(self, track_id, quality):
        playable_id = TrackId.from_base62(track_id)
        return self.session.content_feeder().load(
            playable_id, VorbisOnlyAudioQuality(quality), False, None)
