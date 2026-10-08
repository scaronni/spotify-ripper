// Live status block pinned to the bottom rows of the terminal, while normal
// output scrolls in the region above it.

use std::io::IsTerminal;
use std::time::Instant;

use crate::args::args;
use crate::output::write_raw;
use crate::util::{format_size, format_time, format_time_short};

/// Width of the bar labels, so the bars line up at the same column.
const BAR_LABEL_WIDTH: usize = "Downloading:".len();

pub struct Progress {
    // per-song state, in milliseconds
    current_track: Option<u32>,
    song_position: f64,
    song_duration: f64,

    // overall state
    pub show_total: bool,
    pub skipped_tracks: usize,
    pub track_idx: usize,
    pub total_tracks: usize,
    total_position: f64,
    total_duration: f64,
    pub total_size: u64,

    // eta calculation
    ema_rate: Option<f64>,
    stat_prev: Option<(f64, Instant)>,
    song_eta: Option<f64>,
    total_eta: Option<f64>,
    last_eta_calc: Option<Instant>,

    term_width: usize,
    term_height: usize,

    // pinned bottom status block
    active: bool,
    reserved: usize,
    status: Vec<String>,
}

impl Progress {
    pub fn new() -> Self {
        let mut p = Self {
            current_track: None,
            song_position: 0.0,
            song_duration: 0.0,
            show_total: false,
            skipped_tracks: 0,
            track_idx: 0,
            total_tracks: 0,
            total_position: 0.0,
            total_duration: 0.0,
            total_size: 0,
            ema_rate: None,
            stat_prev: None,
            song_eta: None,
            total_eta: None,
            last_eta_calc: None,
            term_width: 120,
            term_height: 24,
            active: false,
            reserved: 0,
            status: Vec::new(),
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

    /// Reserve the bottom rows for the live status block and confine normal
    /// (scrolling) output to the region above.
    pub fn setup(&mut self) {
        if args().has_log || self.active || !std::io::stdout().is_terminal() {
            return;
        }
        self.reserved = if self.show_total { 4 } else { 3 };
        if self.term_height < self.reserved + 2 {
            return; // terminal too short for a pinned block
        }
        self.status = vec![
            "Track download size: -".to_owned(),
            "Downloading:".to_owned(),
            "Progress:".to_owned(),
        ];
        if self.show_total {
            self.status.push("Total:".to_owned());
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
        if !self.active {
            return;
        }
        self.active = false;
        let row = self.term_height - self.reserved + 1;
        write_raw(&format!("\x1b7\x1b[r\x1b[{row};1H\x1b[J\x1b8"));
    }

    fn render(&self) {
        if !self.active {
            return;
        }
        let top = self.term_height - self.reserved;
        let mut s = String::from("\x1b7");
        for (i, line) in self.status.iter().enumerate() {
            let line: String = line.chars().take(self.term_width).collect();
            s.push_str(&format!("\x1b[{};1H\x1b[2K{line}", top + 1 + i));
        }
        s.push_str("\x1b8");
        write_raw(&s);
    }

    /// `tracks`: (duration in ms, estimated size, already ripped or unavailable).
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

    /// Blank indent matching the counter width, so continuation lines align.
    pub fn indent(&self) -> String {
        " ".repeat(self.counter_prefix().len())
    }

    pub fn prepare_track(&mut self, duration: u32) {
        self.song_position = 0.0;
        self.song_duration = duration as f64;
        self.current_track = Some(duration);
        self.track_idx += 1;
        self.stat_prev = None;
        self.song_eta = None;
        self.last_eta_calc = None;
        if self.active {
            self.status[1] = "Downloading:".to_owned();
            self.status[2] = "Progress:".to_owned();
            self.render();
        }
    }

    pub fn set_download_size(&mut self, size: u64) {
        if self.active {
            self.status[0] = format!("Track download size: {}", format_size(size));
            self.render();
        }
    }

    pub fn set_download_progress(&mut self, downloaded: u64, total: u64) {
        if self.active && total > 0 {
            self.status[1] = self.pct_bar("Downloading:", (downloaded * 100 / total) as usize);
            self.render();
        }
    }

    fn prog_width(&self) -> usize {
        if self.term_width < 70 {
            10
        } else if self.term_width < 100 {
            40 - (100 - self.term_width)
        } else {
            40
        }
    }

    fn pct_bar(&self, label: &str, pct: usize) -> String {
        let w = self.prog_width();
        let x = pct * w / 100;
        format!(
            "{label:<BAR_LABEL_WIDTH$} [{}{}] {pct}%",
            "=".repeat(x),
            " ".repeat(w - x)
        )
    }

    fn time_bar(&self, label: &str, pos_ms: f64, dur_ms: f64, eta: Option<f64>) -> String {
        let w = self.prog_width();
        let pct = if dur_ms > 0.0 {
            ((pos_ms * 100.0 / dur_ms) as usize).min(100)
        } else {
            0
        };
        let x = pct * w / 100;
        let mut s = format!(
            "{label:<BAR_LABEL_WIDTH$} [{}{}] {}",
            "=".repeat(x),
            " ".repeat(w - x),
            format_time((pos_ms / 1000.0) as u64, Some((dur_ms / 1000.0) as u64))
        );
        if let Some(eta) = eta {
            s.push_str(&format!(
                "  (~{} remaining)",
                format_time_short(eta.max(0.0) as u64)
            ));
        }
        s
    }

    pub fn update_progress(&mut self, num_frames: usize, sample_rate: u32) {
        if !self.active {
            return;
        }
        if num_frames > 0 && sample_rate > 0 {
            self.song_position += num_frames as f64 * 1000.0 / sample_rate as f64;
        }
        if self
            .last_eta_calc
            .is_none_or(|t| t.elapsed().as_secs_f64() >= 2.0)
        {
            self.last_eta_calc = Some(Instant::now());
            self.eta_calc();
        }
        self.status[2] = self.time_bar(
            "Progress:",
            self.song_position,
            self.song_duration,
            self.song_eta,
        );
        if self.show_total {
            let total_position = self.total_position + self.song_position;
            self.status[3] = self.time_bar(
                "Total:",
                total_position,
                self.total_duration,
                self.total_eta,
            );
        }
        self.render();
    }

    pub fn end_track(&mut self, show_end: bool) {
        if show_end && self.active {
            self.song_position = self.song_duration;
            self.eta_calc();
            self.update_progress(0, 0);
        }
        self.stat_prev = None;
        self.song_eta = None;
        self.total_eta = None;
        if let Some(duration) = self.current_track.take() {
            self.total_position += duration as f64;
        }
    }

    fn eta_calc(&mut self) {
        fn calc(pos: f64, dur: f64, rate: f64, old_eta: Option<f64>) -> Option<f64> {
            let mut new_eta = (dur - pos) / rate;
            if old_eta.is_none_or(|old| (new_eta - old).abs() >= 5.0) {
                let r = new_eta % 5.0;
                new_eta += if r >= 3.0 { 5.0 - r } else { -r };
                Some(new_eta)
            } else {
                old_eta
            }
        }

        if self.current_track.is_none() {
            return;
        }
        if let Some((prev_pos, prev_time)) = self.stat_prev {
            let dt = prev_time.elapsed().as_secs_f64();
            if dt > 0.0 {
                let rate = (self.song_position - prev_pos) / dt;
                if rate > 0.00000001 {
                    let ema = match self.ema_rate {
                        Some(avg) => 0.005 * rate + 0.995 * avg,
                        None => rate,
                    };
                    self.ema_rate = Some(ema);
                    // rates are in ms of audio per second, ETAs in seconds
                    self.song_eta = calc(
                        self.song_position / 1000.0,
                        self.song_duration / 1000.0,
                        ema / 1000.0,
                        self.song_eta,
                    );
                    if self.show_total {
                        let tp = self.total_position + self.song_position;
                        self.total_eta = calc(
                            tp / 1000.0,
                            self.total_duration / 1000.0,
                            ema / 1000.0,
                            self.total_eta,
                        );
                    }
                }
            }
        }
        self.stat_prev = Some((self.song_position, Instant::now()));
    }
}
