# -*- coding: utf-8 -*-

from __future__ import unicode_literals

from spotify_ripper.utils import *
import os
import sys
import time
from spotify_ripper.librespot_session import SpotifyError

try:
    from fcntl import ioctl
    from array import array
    import termios
except ImportError:
    pass

# label column width for the bar lines, so "Downloading:", "Progress:" and
# "Total:" line their bars up at the same column
_BAR_LABEL_WIDTH = len("Downloading:")


class Progress(object):
    # per-song state
    current_track = None
    song_position = 0
    song_duration = 0

    # overall state
    show_total = False
    skipped_tracks = 0
    track_idx = 0
    total_tracks = 0
    total_position = 0
    total_duration = 0
    total_size = 0

    # eta calculation
    ema_rate = None
    stat_prev = None
    song_eta = None
    total_eta = None
    _last_eta_calc = 0.0

    # terminal geometry
    term_width = 120
    term_height = 24

    # pinned bottom status block (reserved bottom `reserved` rows)
    active = False
    reserved = 0
    status = []

    def __init__(self, args, ripper):
        self.args = args
        self.ripper = ripper
        self.handle_resize()

    # -- terminal geometry ---------------------------------------------------
    def handle_resize(self, signum=None, frame=None):
        try:
            buf = array("h", [0, 0, 0, 0])
            ioctl(sys.stdout, termios.TIOCGWINSZ, buf)
            if buf[0] > 0:
                self.term_height = buf[0]
            if buf[1] > 0:
                self.term_width = buf[1]
        except (NameError, IOError, OSError):
            self.term_width = int(os.environ.get('COLUMNS', 120))
            self.term_height = int(os.environ.get('LINES', 24))
        if self.active:
            # re-establish the scroll region for the new size and redraw
            sys.stdout.write("\033[1;%dr" % (self.term_height - self.reserved))
            sys.stdout.write("\033[%d;1H" % (self.term_height - self.reserved))
            self.render()

    # -- pinned status block -------------------------------------------------
    def setup(self):
        """Reserve the bottom rows for the live status block and confine
        normal (scrolling) output to the region above."""
        if self.args.has_log or self.active:
            return
        self.reserved = 4 if self.show_total else 3
        if self.term_height - self.reserved < 2:
            return  # terminal too short for a pinned block
        self.status = ["Track download size: -", "Downloading:", "Progress:"]
        if self.show_total:
            self.status.append("Total:")
        self.active = True
        H, K = self.term_height, self.reserved
        sys.stdout.write("\n" * K)                  # free the bottom K rows
        sys.stdout.write("\033[1;%dr" % (H - K))    # scroll region = rows above
        sys.stdout.write("\033[%d;1H" % (H - K))    # park at region bottom
        sys.stdout.flush()
        self.render()

    def teardown(self):
        """Restore the full-screen scroll region and erase the status block.
        Safe to call multiple times and from any exit path."""
        if not self.active:
            return
        self.active = False
        H, K = self.term_height, self.reserved
        sys.stdout.write("\033[r")                          # reset scroll region
        sys.stdout.write("\033[%d;1H\033[J" % (H - K + 1))  # erase the block
        sys.stdout.flush()

    def render(self):
        if not self.active:
            return
        H, K = self.term_height, self.reserved
        parts = []
        for i, line in enumerate(self.status):
            parts.append("\033[%d;1H\033[2K%s"
                         % (H - K + 1 + i, line[:self.term_width]))
        parts.append("\033[%d;1H" % (H - K))   # park back in the scroll region
        sys.stdout.write("".join(parts))
        sys.stdout.flush()

    # -- totals --------------------------------------------------------------
    def calc_total(self, track_pairs):
        self.show_total = len(track_pairs) > 1
        self.track_idx = 0
        self.total_tracks = 0
        self.total_duration = 0
        self.total_size = 0

        for pair in track_pairs:
            try:
                track, audio_file = pair
                track.load()
                if track.availability != 1 or track.is_local:
                    self.skipped_tracks += 1
                    continue
                if not self.args.overwrite and path_exists(audio_file) and \
                        not is_partial(audio_file, track):
                    self.skipped_tracks += 1
                    continue
                self.total_tracks += 1
                self.total_duration += track.duration
                self.total_size += calc_file_size(track)
            except SpotifyError:
                continue

    def counter_prefix(self):
        """Fixed-width '[ N/M] ' counter for the current track, or '' for a
        single-track run.  Uses track_idx+1 (the increment happens as the
        track is processed)."""
        if not self.show_total:
            return ""
        total = self.total_tracks + self.skipped_tracks
        width = len(str(total))
        return "[" + str(self.track_idx + 1).rjust(width) + "/" + \
            str(total) + "] "

    def indent(self):
        """Blank indent matching the counter width, so continuation lines
        align under the track's content."""
        return " " * len(self.counter_prefix())

    # -- per-track -----------------------------------------------------------
    def prepare_track(self, track):
        self.song_position = 0
        self.song_duration = track.duration
        self.current_track = track
        self.track_idx += 1
        self.stat_prev = None
        self.song_eta = None
        self._last_eta_calc = 0.0
        if self.active:
            self.status[1] = "Downloading:"
            self.status[2] = "Progress:"
            self.render()

    def set_download_size(self, size):
        if self.active:
            self.status[0] = "Track download size: " + format_size(size)
            self.render()

    def set_download_progress(self, downloaded, total):
        if self.active and total > 0:
            self.status[1] = self._pct_bar("Downloading:",
                                           int(downloaded * 100 / total))
            self.render()

    # -- bar building --------------------------------------------------------
    def _prog_width(self):
        if self.term_width < 70:
            return 10
        elif self.term_width < 100:
            return 40 - (100 - self.term_width)
        return 40

    def _pct_bar(self, label, pct):
        w = self._prog_width()
        x = int(pct * w // 100)
        return "%s [%s%s] %d%%" % (label.ljust(_BAR_LABEL_WIDTH),
                                   "=" * x, " " * (w - x), pct)

    def _time_bar(self, label, pos_ms, dur_ms, eta):
        w = self._prog_width()
        pct = int(pos_ms * 100 // dur_ms) if dur_ms > 0 else 0
        x = int(pct * w // 100)
        s = "%s [%s%s] %s" % (label.ljust(_BAR_LABEL_WIDTH), "=" * x,
                              " " * (w - x),
                              format_time(pos_ms // 1000, dur_ms // 1000))
        if eta is not None:
            s += "  (~%s remaining)" % format_time(eta, short=True)
        return s

    def update_progress(self, num_frames, sample_rate):
        if not self.active:
            return
        if num_frames > 0 and sample_rate and sample_rate > 0:
            self.song_position += (num_frames * 1000) / sample_rate
        now = time.time()
        if now - self._last_eta_calc >= 2:
            self._last_eta_calc = now
            self.eta_calc()
        self.status[2] = self._time_bar("Progress:", self.song_position,
                                        self.song_duration, self.song_eta)
        if self.show_total:
            total_position = self.total_position + self.song_position
            self.status[3] = self._time_bar("Total:", total_position,
                                            self.total_duration, self.total_eta)
        self.render()

    def end_track(self, show_end=True):
        if show_end and self.active:
            self.song_position = self.song_duration
            self.eta_calc()
            self.update_progress(0, None)
        self.stat_prev = None
        self.song_eta = None
        self.total_eta = None
        if self.current_track is not None:
            self.total_position += self.current_track.duration
        self.current_track = None

    # -- eta -----------------------------------------------------------------
    def eta_calc(self):
        def calc_rate(rate, avg_rate, sf):
            if avg_rate is None:
                return rate
            return (sf * rate) + ((1.0 - sf) * avg_rate)

        def calc(pos, dur, rate, old_eta):
            new_eta = (dur - pos) / rate
            if old_eta is None or abs(new_eta - old_eta) >= 5:
                r = new_eta % 5
                new_eta += ((5 - r) if r >= 3 else (0 - r))
                return new_eta
            return old_eta

        if self.current_track is None:
            return
        if self.stat_prev is not None:
            dt = time.time() - self.stat_prev[1]
            if dt > 0:
                rate = (self.song_position - self.stat_prev[0]) / dt
                if rate > 0.00000001:
                    self.ema_rate = calc_rate(rate, self.ema_rate, 0.005)
                    self.song_eta = calc(self.song_position, self.song_duration,
                                         self.ema_rate, self.song_eta)
                    if self.show_total:
                        tp = self.total_position + self.song_position
                        self.total_eta = calc(tp, self.total_duration,
                                              self.ema_rate, self.total_eta)
        self.stat_prev = (self.song_position, time.time())
