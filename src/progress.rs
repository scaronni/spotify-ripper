// Live status bars pinned to the bottom rows of the terminal (the current
// track and the whole run), while normal output scrolls in the region above.

use std::io::IsTerminal;
use std::time::Instant;

use crate::args::args;
use crate::output::{DIM, RESET, write_raw};
use crate::util::{format_duration, format_size};

pub struct Progress {
    // counters for the "[ N/M]" prefix
    pub show_total: bool,
    pub skipped_tracks: usize,
    pub track_idx: usize,
    pub total_tracks: usize,

    // the current track
    label: String,
    phase: String,
    /// Progress of the current phase, 0.0 - 1.0.
    phase_done: f64,
    /// Share of the current track already done, 0.0 - 1.0.
    track_done: f64,
    track_duration: f64,

    // the whole run, by audio duration (ms) and download size (bytes)
    total_duration: f64,
    done_duration: f64,
    pub total_size: u64,
    downloaded: u64,
    started: Instant,

    term_width: usize,
    term_height: usize,

    // pinned status block and transient loading line
    active: bool,
    reserved: usize,
    loading: bool,
}

/// Visible width of the label text in front of the bars.
const BAR_LABEL_WIDTH: usize = "Track".len();

impl Progress {
    pub fn new() -> Self {
        let mut p = Self {
            show_total: false,
            skipped_tracks: 0,
            track_idx: 0,
            total_tracks: 0,
            label: String::new(),
            phase: String::new(),
            phase_done: 0.0,
            track_done: 0.0,
            track_duration: 0.0,
            total_duration: 0.0,
            done_duration: 0.0,
            total_size: 0,
            downloaded: 0,
            started: Instant::now(),
            term_width: 120,
            term_height: 24,
            active: false,
            reserved: 0,
            loading: false,
        };
        p.handle_resize();
        p
    }

    fn terminal_size() -> Option<(usize, usize)> {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
        (ret == 0 && ws.ws_row > 0 && ws.ws_col > 0)
            .then_some((ws.ws_col as usize, ws.ws_row as usize))
    }

    /// Whether live (cursor-moving) output can be used at all.
    fn interactive() -> bool {
        !args().has_log && std::io::stdout().is_terminal()
    }

    pub fn handle_resize(&mut self) {
        let env = |name: &str, default: usize| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        (self.term_width, self.term_height) =
            Self::terminal_size().unwrap_or_else(|| (env("COLUMNS", 120), env("LINES", 24)));
        if self.active {
            // re-establish the scroll region for the new size and redraw
            let top = self.term_height - self.reserved;
            write_raw(&format!("\x1b[1;{top}r\x1b[{top};1H"));
            self.render();
        }
    }

    /// Show a transient one-line status while loading metadata.
    pub fn loading(&mut self, message: &str) {
        if Self::interactive() {
            let message: String = message.chars().take(self.term_width - 1).collect();
            write_raw(&format!("\r\x1b[2K{DIM}{message}{RESET}"));
            self.loading = true;
        }
    }

    /// Remove the loading line, before printing anything else.
    pub fn clear_loading(&mut self) {
        if self.loading {
            write_raw("\r\x1b[2K");
            self.loading = false;
        }
    }

    /// Reserve the bottom rows for the bars and confine normal (scrolling)
    /// output to the region above.
    pub fn setup(&mut self) {
        self.clear_loading();
        self.started = Instant::now();
        if self.active || !Self::interactive() {
            return;
        }
        self.reserved = if self.show_total { 2 } else { 1 };
        if self.term_height < self.reserved + 2 {
            return; // terminal too short for a pinned block
        }
        self.active = true;
        // free the bottom rows (scrolling if needed), go back to where the
        // output continues and confine scrolling to the rows above the block;
        // setting the scroll region moves the cursor, so save and restore it
        let top = self.term_height - self.reserved;
        write_raw(&format!(
            "{}\x1b[{}A\x1b7\x1b[1;{top}r\x1b8",
            "\n".repeat(self.reserved),
            self.reserved
        ));
        self.render();
    }

    /// Restore the full-screen scroll region and erase the status block.
    /// Safe to call multiple times and from any exit path.
    pub fn teardown(&mut self) {
        self.clear_loading();
        if !self.active {
            return;
        }
        self.active = false;
        let row = self.term_height - self.reserved + 1;
        write_raw(&format!("\x1b7\x1b[r\x1b[{row};1H\x1b[J\x1b8"));
    }

    fn bar(&self, fraction: f64) -> String {
        let width = self.bar_width();
        let done = ((fraction.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
        let (full, empty) = if args().ascii {
            ("=", "-")
        } else {
            ("━", "─")
        };
        format!(
            "{}{DIM}{}{RESET}",
            full.repeat(done),
            empty.repeat(width - done)
        )
    }

    fn render(&self) {
        if !self.active {
            return;
        }
        let mut lines = Vec::new();

        let fixed = format!(
            "{:<BAR_LABEL_WIDTH$} {} {:>3}%  {}",
            "Track",
            self.bar(self.phase_done),
            (self.phase_done * 100.0) as u32,
            self.phase
        );
        // the bar has invisible color codes: count only what is visible
        let visible = BAR_LABEL_WIDTH + 1 + self.bar_width() + 6 + self.phase.chars().count();
        let room = self.term_width.saturating_sub(visible + 3);
        let label: String = self.label.chars().take(room).collect();
        lines.push(if label.is_empty() {
            fixed
        } else {
            format!("{fixed}  {DIM}{label}{RESET}")
        });

        if self.show_total {
            let fraction = self.total_fraction();
            let mut s = format!(
                "{:<BAR_LABEL_WIDTH$} {} {}/{}  {}/{}",
                "Total",
                self.bar(fraction),
                self.track_idx,
                self.total_tracks + self.skipped_tracks,
                format_size(self.downloaded),
                format_size(self.total_size)
            );
            let elapsed = self.started.elapsed().as_secs_f64();
            if fraction > 0.02 && elapsed > 5.0 {
                let left = elapsed * (1.0 - fraction) / fraction;
                s.push_str(&format!("  ~{} left", format_duration(left as u64)));
            }
            lines.push(s);
        }

        let top = self.term_height - self.reserved;
        let mut s = String::from("\x1b7");
        for (i, line) in lines.iter().enumerate() {
            s.push_str(&format!("\x1b[{};1H\x1b[2K{line}", top + 1 + i));
        }
        s.push_str("\x1b8");
        write_raw(&s);
    }

    fn bar_width(&self) -> usize {
        match self.term_width {
            w if w >= 110 => 30,
            w if w >= 80 => 20,
            _ => 10,
        }
    }

    fn total_fraction(&self) -> f64 {
        if self.total_duration <= 0.0 {
            return 0.0;
        }
        (self.done_duration + self.track_done * self.track_duration) / self.total_duration
    }

    /// `tracks`: (duration in ms, estimated size, nothing to rip).
    pub fn calc_total(&mut self, tracks: &[(u32, u64, bool)]) {
        self.show_total = tracks.len() > 1;
        self.track_idx = 0;
        self.total_tracks = 0;
        self.total_duration = 0.0;
        self.total_size = 0;
        for (duration, size, skip) in tracks {
            if *skip {
                self.skipped_tracks += 1;
                continue;
            }
            self.total_tracks += 1;
            self.total_duration += *duration as f64;
            self.total_size += size;
        }
    }

    /// Fixed-width "[ N/M] " counter for the current track, or "" for a
    /// single-track run.
    pub fn counter_prefix(&self) -> String {
        if !self.show_total {
            return String::new();
        }
        let total = self.total_tracks + self.skipped_tracks;
        let width = total.to_string().len();
        format!("[{:>width$}/{total}] ", self.track_idx + 1)
    }

    /// Blank indent matching the counter width (at least two spaces), so
    /// details align under the track.
    pub fn indent(&self) -> String {
        " ".repeat(self.counter_prefix().len().max(2))
    }

    pub fn prepare_track(&mut self, duration: u32, label: &str) {
        self.track_idx += 1;
        self.track_duration = duration as f64;
        self.track_done = 0.0;
        self.label = label.to_owned();
        self.set_phase("starting", 0.0);
    }

    pub fn set_phase(&mut self, phase: &str, done: f64) {
        self.phase = phase.to_owned();
        self.phase_done = done;
        self.render();
    }

    /// Downloading counts as the first half of a track, encoding as the second.
    pub fn add_downloaded(&mut self, bytes: u64, downloaded: u64, total: u64) {
        self.downloaded += bytes;
        let done = if total > 0 {
            downloaded as f64 / total as f64
        } else {
            0.0
        };
        self.track_done = done / 2.0;
        self.set_phase("downloading", done);
    }

    pub fn set_encoded(&mut self, position_ms: f64) {
        let done = if self.track_duration > 0.0 {
            (position_ms / self.track_duration).min(1.0)
        } else {
            0.0
        };
        self.track_done = 0.5 + done / 2.0;
        self.set_phase("encoding", done);
    }

    pub fn end_track(&mut self) {
        self.done_duration += self.track_duration;
        self.track_duration = 0.0;
        self.track_done = 0.0;
        self.label.clear();
        self.set_phase("", 0.0);
    }
}
