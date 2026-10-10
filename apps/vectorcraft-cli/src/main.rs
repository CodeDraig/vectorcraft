//! `vectorcraft-cli`: VectorCraft from the command line.
//!
//! ```text
//! vectorcraft-cli mcp [--connect 127.0.0.1:7979 | --headless]
//! vectorcraft-cli run [--in FILE] [--cmd id [--params '{json}']]... [--export out.svg]... [--scale 2]
//! vectorcraft-cli commands
//! vectorcraft-cli convert IN OUT [--scale 2] [--artboard 0 | --range 1-3,5] [--outline-text] [--replace-lossy]
//! vectorcraft-cli info FILE
//! vectorcraft-cli bench FILE [--size 2880x1800] [--iters 5]
//! vectorcraft-cli perf [--paths 50000]
//! ```
// Denied, not forbidden: the font listers shared with the desktop app allow it for their
// DirectWrite (Windows) and CoreText (macOS) calls, and nothing else may.
#![deny(unsafe_code)]

use std::io::Write;
use std::process::ExitCode;

/// `println!` / `print!` that end the program quietly when stdout is closed
/// (`vectorcraft-cli commands | head`) instead of panicking with "failed printing to stdout:
/// Broken pipe (os error 32)".
macro_rules! outln {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        if let Err(e) = writeln!(std::io::stdout(), $($arg)*) {
            $crate::stdout_failed(e);
        }
    }};
}
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        if let Err(e) = write!(std::io::stdout(), $($arg)*) {
            $crate::stdout_failed(e);
        }
    }};
}
mod mcp;
use mcp::mcp;
mod batch;
use batch::run;
mod bench;
use bench::bench;
mod files;
use files::{convert, info};

/// The desktop app's CoreText font lister, shared: exports and MCP find the same fonts.
#[cfg(target_os = "macos")]
#[path = "../../vectorcraft/src/mac_fonts.rs"]
mod mac_fonts;
mod perf;
/// The desktop app's DirectWrite font lister, shared: exports and MCP find the same fonts.
#[cfg(all(windows, not(target_vendor = "win7")))]
#[path = "../../vectorcraft/src/system_fonts.rs"]
mod system_fonts;

use serde_json::{Value, json};
use vectorcraft_mcp::{Backend, DEFAULT_ADDR, Headless, Remote, Server};

/// stdout went away. A reader that stopped early (a closed pipe) ends the program quietly, as
/// ripgrep does; any other write error is reported.
fn stdout_failed(e: std::io::Error) -> ! {
    if e.kind() == std::io::ErrorKind::BrokenPipe {
        std::process::exit(0);
    }
    eprintln!("vectorcraft-cli: can't write to stdout: {e}");
    std::process::exit(1);
}

const USAGE: &str = "\
vectorcraft-cli — VectorCraft automation

USAGE:
  vectorcraft-cli mcp [--connect ADDR | --headless]
      Run the MCP server on stdio. Default: connect to a running app at 127.0.0.1:7979
      (vectorcraft --control 7979), falling back to a headless in-process session.

  vectorcraft-cli run [--in FILE] [--cmd ID [--params JSON]]... [--export FILE]... [--scale N]
      Headless batch: open FILE (any readable format) or start a new document, run commands in
      order, export each FILE in the format its extension picks (see Writable formats). Prints one
      JSON result per step.

  vectorcraft-cli commands
      Print the command catalogue as JSON.

  vectorcraft-cli convert IN OUT [--scale N] [--artboard I | --range R] [--outline-text] [--replace-lossy]
      Open IN (any readable format) and export OUT in the format its extension picks (see Writable
      formats). --artboard is 0-based, --range 1-based (\"1-3,5\"); a PDF gets every artboard
      unless one of them is given, EPS the bounds of the art, the other formats the first artboard.
      Live effects are kept, and hidden layers and objects (written hidden in SVG and PSD), with
      SVG's data-* attributes; --outline-text writes SVG text as paths. OUT may not be IN when reading
      IN left things out (hidden text, art or layers it could not read; the notes say which) unless
      --replace-lossy is given.

  vectorcraft-cli info FILE
      Print a JSON summary: the import warnings (what didn't come in as it was, such as an EPS
      read from its preview and why), title, colour mode, units, artboards, object counts by
      kind, fonts.

  vectorcraft-cli bench FILE [--size WxH] [--iters N]
      Render FILE (any readable format) fitted to WxH (default 2880x1800) and print ms per frame
      (warm), multithreaded and single-threaded.

  vectorcraft-cli perf [--paths N]
      Check the performance budgets (render, pan, hit test, save/load, SVG, Pathfinder) on a
      synthetic N-path document (default 50000). Exits non-zero if a budget is exceeded.
";

/// The usage text plus the formats `document.open` reads and `document.export` writes.
fn usage() -> String {
    use vectorcraft_engine::cmd::fileio::{OPEN_EXTS, export_extensions};
    format!("{USAGE}\nReadable formats: .{}\nWritable formats: .{}\n", OPEN_EXTS.join(", ."), export_extensions().join(", ."))
}

fn main() -> ExitCode {
    // Before any font lookup: the fonts font services load (#579).
    #[cfg(all(windows, not(target_vendor = "win7")))]
    system_fonts::install();
    #[cfg(target_os = "macos")]
    mac_fonts::install();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("mcp") => mcp(&args[1..]),
        Some("run") => run(&args[1..]),
        Some("commands") => commands(),
        Some("convert") => convert(&args[1..]),
        Some("info") => info(&args[1..]),
        Some("bench") => bench(&args[1..]),
        Some("perf") => perf::run(&args[1..]),
        Some("-h" | "--help" | "help") | None => {
            out!("{}", usage());
            Ok(())
        }
        Some("-V" | "--version") => {
            outln!("vectorcraft-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => Err(format!("unknown subcommand `{other}`\n\n{}", usage())),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("vectorcraft-cli: {e}");
            ExitCode::FAILURE
        }
    }
}

fn commands() -> Result<(), String> {
    let mut h = Headless::new();
    let v = h.call("engine.commands", json!({}))?;
    outln!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    Ok(())
}
