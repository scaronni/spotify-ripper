// Console / log output: colors are plain ANSI SGR codes that are stripped when
// not writing to a terminal, with -s, or with -L.

use std::fs::OpenOptions;
use std::io::{self, IsTerminal, Write};
use std::sync::{Mutex, OnceLock};

pub const GREEN: &str = "\x1b[32m";
pub const RED: &str = "\x1b[31m";
pub const ORANGE: &str = "\x1b[38;5;208m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const RESET: &str = "\x1b[0m";

struct Output {
    writer: Box<dyn Write + Send>,
    strip: bool,
}

static OUTPUT: OnceLock<Mutex<Output>> = OnceLock::new();

fn output() -> &'static Mutex<Output> {
    OUTPUT.get_or_init(|| {
        Mutex::new(Output {
            writer: Box::new(io::stdout()),
            strip: !io::stdout().is_terminal(),
        })
    })
}

/// Set up output: `log` is the -L/--log value ("-" logs to stdout).
pub fn init(log: Option<&str>, strip_colors: bool) -> io::Result<()> {
    let out = match log {
        Some("-") => Output {
            writer: Box::new(io::stdout()),
            strip: true,
        },
        Some(path) => Output {
            writer: Box::new(OpenOptions::new().create(true).append(true).open(path)?),
            strip: true,
        },
        None => Output {
            writer: Box::new(io::stdout()),
            strip: strip_colors || !io::stdout().is_terminal(),
        },
    };
    if OUTPUT.set(Mutex::new(out)).is_err() {
        panic!("output initialized twice");
    }
    Ok(())
}

fn strip_sgr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find("\x1b[") {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos + 2..];
        let end = tail.find(|c: char| !(c.is_ascii_digit() || c == ';'));
        match end {
            Some(e) if tail[e..].starts_with('m') => rest = &tail[e + 1..],
            _ => {
                out.push_str("\x1b[");
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Write `s` as-is, apart from the color stripping.
pub fn write(s: &str) {
    let mut out = output().lock().unwrap_or_else(|e| e.into_inner());
    let text = if out.strip {
        strip_sgr(s)
    } else {
        s.to_owned()
    };
    let _ = out.writer.write_all(text.as_bytes());
    let _ = out.writer.flush();
}

/// Write terminal control sequences (cursor movement); never stripped.
pub fn write_raw(s: &str) {
    let mut out = output().lock().unwrap_or_else(|e| e.into_inner());
    let _ = out.writer.write_all(s.as_bytes());
    let _ = out.writer.flush();
}

#[macro_export]
macro_rules! outln {
    () => { $crate::output::write("\n") };
    ($($arg:tt)*) => { $crate::output::write(&format!("{}\n", format!($($arg)*))) };
}

/// A non-fatal problem, in orange.
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        $crate::output::write(&format!(
            "{}warning: {}{}\n",
            $crate::output::ORANGE,
            format!($($arg)*),
            $crate::output::RESET
        ))
    };
}

/// Width of the label column for the --verbose fields: the width of the longest
/// status word ("unavailable") plus a space, so the values align with the tracks.
const LABEL_WIDTH: usize = 12;

/// A '<indent><label:> <value>' line with a dim label, for --verbose details.
pub fn format_field(indent: &str, label: &str, value: impl std::fmt::Display) -> String {
    format!(
        "{indent}{DIM}{:<width$}{RESET}{value}",
        format!("{label}:"),
        width = LABEL_WIDTH
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_only_colors() {
        assert_eq!(strip_sgr("\x1b[32mok\x1b[39m"), "ok");
        assert_eq!(strip_sgr("\x1b[1m\x1b[2mA\x1b[0m"), "A");
        assert_eq!(strip_sgr("\x1b[38;5;208mA\x1b[39m"), "A");
        assert_eq!(strip_sgr("a\x1b[2Kb"), "a\x1b[2Kb");
    }
}
