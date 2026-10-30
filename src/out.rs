//! Best-effort stdout for `out!` and `outln!`.
//!
//! `println!` panics when the reader goes away (`jimtime review | head`).
//! These macros notice the broken pipe, drop the rest of the output, and
//! let the command finish, so a save, Harvest push or data-repo commit
//! that follows a print still happens.

use std::fmt;
use std::io::{self, ErrorKind, Write};
use std::sync::atomic::{AtomicBool, Ordering};

static CLOSED: AtomicBool = AtomicBool::new(false);

macro_rules! out {
    ($($arg:tt)*) => {
        $crate::out::write(format_args!($($arg)*))
    };
}

macro_rules! outln {
    () => {
        $crate::out::write(format_args!("\n"))
    };
    ($($arg:tt)*) => {
        $crate::out::write(format_args!("{}\n", format_args!($($arg)*)))
    };
}

pub fn write(args: fmt::Arguments) {
    if CLOSED.load(Ordering::Relaxed) {
        return;
    }
    if let Err(e) = io::stdout().lock().write_fmt(args) {
        settle(e);
    }
}

/// Flush a prompt written with `out!` before reading the answer.
pub fn flush() {
    if CLOSED.load(Ordering::Relaxed) {
        return;
    }
    if let Err(e) = io::stdout().flush() {
        settle(e);
    }
}

// A closed pipe means nobody is reading; anything else (a full disk) is as
// fatal as it is for `println!`.
fn settle(e: io::Error) {
    if e.kind() == ErrorKind::BrokenPipe {
        CLOSED.store(true, Ordering::Relaxed);
    } else {
        panic!("failed printing to stdout: {e}");
    }
}
