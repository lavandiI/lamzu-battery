//! Application entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;

use lamzu_battery::cli::Cli;
use lamzu_battery::config::Paths;
use lamzu_battery::hid::{HidPort, NativeHid, NoHid};
use lamzu_battery::i18n::Language;
use lamzu_battery::win::{self, HeldMutex};
use lamzu_battery::{
    APP_NAME, MUTEX_TRAY, VERSION, app, autostart, crash, diagnostics, logging, updater,
};

/// How the process obtained a console, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConsoleMode {
    /// No console at all.
    None,
    /// Attached to the console of the parent process.
    Parent,
    /// A console was allocated for this process, so it has to stay open.
    Allocated,
}

fn console_for_cli(args: &[String]) -> ConsoleMode {
    let wants_output = args.iter().any(|argument| {
        matches!(
            argument.as_str(),
            "--help" | "-h" | "--version" | "-V" | "--diagnose"
        )
    });
    if !wants_output {
        return ConsoleMode::None;
    }
    if win::attach_parent_console() {
        return ConsoleMode::Parent;
    }
    let interactive = args
        .iter()
        .any(|argument| matches!(argument.as_str(), "--help" | "-h" | "--diagnose"));
    if interactive && win::alloc_console() {
        ConsoleMode::Allocated
    } else {
        ConsoleMode::None
    }
}

fn run_diagnostics(cli: &Cli, console: ConsoleMode) -> ExitCode {
    let language = cli.language().unwrap_or_else(Language::detect);
    let port: Box<dyn HidPort> = match NativeHid::new() {
        Ok(hid) => Box::new(hid),
        Err(error) => {
            eprintln!("HID is not available: {error}");
            Box::new(NoHid)
        }
    };
    let exe = std::env::current_exe().unwrap_or_default();
    let code = diagnostics::run(language, cli.raw, port.as_ref(), &exe);
    if console == ConsoleMode::Allocated {
        win::wait_for_enter();
    }
    ExitCode::from(code)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let console = console_for_cli(&args);
    let cli = Cli::parse();

    if cli.diagnose {
        return run_diagnostics(&cli, console);
    }

    let paths = Paths::discover();
    crash::install(&paths.crash);
    let _log = logging::init(&paths, cli.debug);
    log::info!("{APP_NAME} {VERSION} starting from {}", paths.exe.display());
    log::info!("data folder: {}", paths.data_dir.display());
    if let Some(fallback) = &paths.fallback {
        log::warn!(
            "{} is not writable, using {}",
            fallback.display(),
            paths.data_dir.display()
        );
    }
    autostart::migrate_legacy(&paths.exe);
    if let Some(exe) = updater::executable_path() {
        let _ = updater::cleanup_stale_backup(&exe);
        updater::cleanup_download_files(&exe);
    }
    if !lamzu_battery::is_packaged() {
        log::debug!("development build: updates and tray promotion are disabled");
    }

    let mutex = if cli.wait_for_instance {
        log::info!("waiting for the previous instance to release the tray mutex");
        HeldMutex::acquire_waiting(MUTEX_TRAY, Duration::from_secs(45))
    } else {
        HeldMutex::acquire(MUTEX_TRAY)
    };
    let Some(mutex) = mutex else {
        log::info!("another instance is already running");
        return ExitCode::SUCCESS;
    };

    let code = app::run(paths, mutex);
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
