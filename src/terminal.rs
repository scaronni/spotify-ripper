// Keyboard and signal handling while ripping: Esc skips the current track,
// Ctrl-C aborts (a second Ctrl-C exits immediately), terminal resizes redraw
// the status block.

use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM, SIGWINCH};
use signal_hook::iterator::Signals;
use std::io::{IsTerminal, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};

use crate::output::{RED, RESET};
use crate::outln;
use crate::progress::Progress;

pub static ABORT: AtomicBool = AtomicBool::new(false);
pub static SKIP: AtomicBool = AtomicBool::new(false);
pub static RIPPING: AtomicBool = AtomicBool::new(false);

static PROGRESS: LazyLock<Mutex<Progress>> = LazyLock::new(|| Mutex::new(Progress::new()));
static SAVED_TERMIOS: Mutex<Option<libc::termios>> = Mutex::new(None);

pub fn progress() -> MutexGuard<'static, Progress> {
    PROGRESS.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn aborted() -> bool {
    ABORT.load(Ordering::Relaxed)
}

pub fn skipped() -> bool {
    SKIP.load(Ordering::Relaxed)
}

/// Put stdin in cbreak mode (no line buffering, no echo) to catch Esc.
fn enter_cbreak() {
    if !std::io::stdin().is_terminal() {
        return;
    }
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(libc::STDIN_FILENO, &mut t) != 0 {
            return;
        }
        *SAVED_TERMIOS.lock().unwrap() = Some(t);
        t.c_lflag &= !(libc::ICANON | libc::ECHO);
        t.c_cc[libc::VMIN] = 1;
        t.c_cc[libc::VTIME] = 0;
        libc::tcsetattr(libc::STDIN_FILENO, libc::TCSADRAIN, &t);
    }
}

/// Restore the terminal: stdin mode and the full-screen scroll region.
pub fn restore() {
    progress().teardown();
    if let Some(t) = SAVED_TERMIOS.lock().unwrap_or_else(|e| e.into_inner()).take() {
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSADRAIN, &t);
        }
    }
}

pub fn exit(code: i32) -> ! {
    restore();
    std::process::exit(code);
}

/// Start watching the keyboard (unless logging) and signals.
pub fn start(watch_keys: bool) {
    if watch_keys && std::io::stdin().is_terminal() {
        enter_cbreak();
        std::thread::spawn(|| {
            let mut stdin = std::io::stdin();
            let mut byte = [0u8; 1];
            while let Ok(1) = stdin.read(&mut byte) {
                if byte[0] == 0x1b && RIPPING.load(Ordering::Relaxed) {
                    SKIP.store(true, Ordering::Relaxed);
                }
            }
        });
    }

    let mut signals =
        Signals::new([SIGINT, SIGTERM, SIGHUP, SIGWINCH]).expect("registering signal handlers");
    std::thread::spawn(move || {
        for signal in signals.forever() {
            match signal {
                SIGWINCH => progress().handle_resize(),
                SIGINT if !ABORT.swap(true, Ordering::Relaxed) => {
                    outln!("\n{RED}Aborting...{RESET}");
                }
                _ => exit(1),
            }
        }
    });
}
