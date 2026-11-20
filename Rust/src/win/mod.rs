//! Thin Win32 translation layer.
//!
//! This is the only module tree allowed to contain `unsafe` code. Every wrapper
//! is a small, documented translation between Win32 conventions and the safe
//! types used by the rest of the crate; no business logic lives here.
//!
//! # Safety model
//!
//! * Handles are stored as integers, so they are `Send` and cannot be
//!   dereferenced by accident.
//! * Every `unsafe` block carries a `// SAFETY:` comment naming the Win32
//!   contract it discharges.
//! * The unhandled-exception filter formats into a fixed stack buffer and only
//!   writes a pre-opened file: it does not allocate, lock or panic.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetFullPathNameW};
use windows_sys::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AllocConsole, AttachConsole};
use windows_sys::Win32::System::Diagnostics::Debug::{
    EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS, SetUnhandledExceptionFilter,
};
use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GetVersionExW, OSVERSIONINFOW};
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentThreadId};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MessageBoxW, PostThreadMessageW,
    SW_SHOWNORMAL, WM_APP,
};

/// Convert a string into a NUL-terminated UTF-16 buffer for Win32 calls.
#[must_use]
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Convert a path into a NUL-terminated UTF-16 buffer for Win32 calls.
#[must_use]
pub fn wide_path(path: &Path) -> Vec<u16> {
    OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// A named Win32 mutex held for the lifetime of the value.
///
/// The mutex is used as a presence latch: it is created, never waited on, and
/// closed to let the next instance take over. `CloseHandle` is valid from any
/// thread, so the wrapper is `Send`.
#[derive(Debug)]
pub struct HeldMutex {
    handle: usize,
}

impl HeldMutex {
    /// Create the named mutex, or return `None` when another instance owns it.
    #[must_use]
    pub fn acquire(name: &str) -> Option<Self> {
        let name = wide(name);
        // SAFETY: `name` is a valid NUL-terminated wide string used only for the
        // duration of the call; a null security attributes pointer means
        // "default security", which is what a per-user mutex needs.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return None;
        }
        // SAFETY: `GetLastError` reads the calling thread's last error, which is
        // only meaningful immediately after a failing Win32 call.
        let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if already_exists {
            // SAFETY: the handle was created by the call above and is not used
            // afterwards, so closing it here cannot cause a double close.
            unsafe { CloseHandle(handle) };
            return None;
        }
        Some(Self {
            handle: handle as usize,
        })
    }

    /// Try to acquire the mutex for up to `timeout`, sleeping between attempts.
    #[must_use]
    pub fn acquire_waiting(name: &str, timeout: Duration) -> Option<Self> {
        let start = Instant::now();
        loop {
            if let Some(held) = Self::acquire(name) {
                return Some(held);
            }
            if start.elapsed() >= timeout {
                return None;
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    }
}

impl Drop for HeldMutex {
    fn drop(&mut self) {
        // SAFETY: the handle came from `CreateMutexW` and is closed exactly once,
        // because `Drop` runs once per `HeldMutex` value.
        unsafe { CloseHandle(self.handle as HANDLE) };
    }
}

/// Show a modal message box. `error` selects the error icon.
pub fn message_box(title: &str, body: &str, error: bool) {
    let title = wide(title);
    let body = wide(body);
    let flags = MB_OK
        | MB_SETFOREGROUND
        | if error {
            MB_ICONERROR
        } else {
            MB_ICONINFORMATION
        };
    // SAFETY: both strings are valid NUL-terminated buffers alive during the
    // call; a null owner window is explicitly allowed by `MessageBoxW`.
    unsafe { MessageBoxW(std::ptr::null_mut(), body.as_ptr(), title.as_ptr(), flags) };
}

/// Open a URL or folder with the shell.
#[must_use]
pub fn open(target: &str) -> bool {
    let operation = wide("open");
    let target = wide(target);
    // SAFETY: all strings are valid NUL-terminated buffers; the remaining
    // parameters are documented as optional and passed as null.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    result as usize > 32
}

/// Current local time formatted as `YYYY-MM-DD HH:MM:SS`.
#[must_use]
pub fn local_timestamp() -> String {
    // SAFETY: `GetLocalTime` fills a plain C struct that is fully initialized on
    // return; `zeroed` is valid because every bit pattern is a valid SYSTEMTIME.
    let mut time: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
    // SAFETY: `time` is a valid, writable SYSTEMTIME for the duration of the call.
    unsafe { GetLocalTime(&mut time) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute, time.wSecond
    )
}

/// Free bytes available on the volume that contains `path`.
#[must_use]
pub fn free_space(path: &Path) -> Option<u64> {
    let path = wide_path(path);
    let mut free = 0_u64;
    // SAFETY: the path is a valid wide string and `free` is a valid out pointer;
    // the two optional outputs are intentionally null.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            path.as_ptr(),
            &raw mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

/// Expand `%VARIABLE%` references in `text`, leaving unknown names untouched.
#[must_use]
pub fn expand_env(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            out.push('%');
            out.push_str(after);
            return out;
        };
        let name = &after[..end];
        match std::env::var(name) {
            Ok(value) if !name.is_empty() => out.push_str(&value),
            _ => {
                out.push('%');
                out.push_str(name);
                out.push('%');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Normalize a registry path the way Windows compares them: quotes and
/// whitespace removed, environment variables expanded, absolute, lowercase.
#[must_use]
pub fn normalize_path(raw: &str) -> String {
    let expanded = expand_env(raw.trim().trim_matches('"'));
    if expanded.is_empty() {
        return String::new();
    }
    let wide = wide(&expanded);
    let mut buffer = vec![0_u16; 32_768];
    // SAFETY: the input is a valid wide string and `buffer` is a valid, writable
    // array whose length is passed in characters.
    let len = unsafe {
        GetFullPathNameW(
            wide.as_ptr(),
            buffer.len() as u32,
            buffer.as_mut_ptr(),
            std::ptr::null_mut(),
        )
    };
    let normalized = if len > 0 && (len as usize) < buffer.len() {
        String::from_utf16_lossy(&buffer[..len as usize])
    } else {
        expanded
    };
    normalized.to_ascii_lowercase()
}

/// Windows build number (for example 22631 on Windows 11 23H2).
#[must_use]
pub fn build_number() -> u32 {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: u32::try_from(std::mem::size_of::<OSVERSIONINFOW>()).unwrap_or(0),
        dwMajorVersion: 0,
        dwMinorVersion: 0,
        dwBuildNumber: 0,
        dwPlatformId: 0,
        szCSDVersion: [0; 128],
    };
    // SAFETY: `info` is a valid, writable OSVERSIONINFOW whose size field is set;
    // the embedded manifest declares Windows 10 support so the answer is exact.
    let ok = unsafe { GetVersionExW(&mut info) };
    if ok == 0 { 0 } else { info.dwBuildNumber }
}

/// Best-effort Windows UI language detection: Russian or English.
#[must_use]
pub fn system_language_is_russian() -> bool {
    // SAFETY: `GetUserDefaultUILanguage` has no parameters and no preconditions.
    let language = unsafe { GetUserDefaultUILanguage() };
    (language & 0x3FF) == 0x19
}

/// Initialize COM for the calling thread as a multithreaded apartment.
///
/// Returns `false` only when COM reported an unexpected error; an already
/// initialized thread (`S_FALSE`) and a different apartment model are accepted.
#[must_use]
pub fn init_com_mta() -> bool {
    // SAFETY: a null reserved pointer and a documented apartment constant are the
    // only inputs; the call is thread-local and idempotent.
    let hr = unsafe { CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED as u32) };
    matches!(hr, 0 | 1 | -0x7FFE_FEFA)
}

/// Attach to the console of the parent process, if one exists.
#[must_use]
pub fn attach_parent_console() -> bool {
    // SAFETY: `ATTACH_PARENT_PROCESS` is a documented sentinel value.
    (unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }) != 0
}

/// Allocate a new console window for this process.
#[must_use]
pub fn alloc_console() -> bool {
    // SAFETY: the function takes no parameters and has no preconditions.
    (unsafe { AllocConsole() }) != 0
}

/// Block until the user presses Enter (used by diagnostics without a console).
pub fn wait_for_enter() {
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
}

/// Identifier of the calling thread, used by `PostThreadMessageW`.
#[must_use]
pub fn current_thread_id() -> u32 {
    // SAFETY: the function takes no parameters and cannot fail.
    unsafe { GetCurrentThreadId() }
}

/// Wake the UI thread so that it drains its command channel.
pub fn wake_thread(thread_id: u32) {
    // SAFETY: the message is a private `WM_APP` value and carries no pointers, so
    // it is safe for any handler to ignore or process.
    unsafe {
        PostThreadMessageW(thread_id, WM_APP, 0, 0);
    }
}

/// Create a fixed buffer that formats without allocating.
struct LineBuf {
    bytes: [u8; 512],
    len: usize,
}

impl LineBuf {
    const fn new() -> Self {
        Self {
            bytes: [0; 512],
            len: 0,
        }
    }

    fn push_str(&mut self, text: &str) {
        let bytes = text.as_bytes();
        let room = self.bytes.len().saturating_sub(self.len);
        let take = bytes.len().min(room);
        self.bytes[self.len..self.len + take].copy_from_slice(&bytes[..take]);
        self.len += take;
    }

    fn push_hex(&mut self, value: usize) {
        self.push_str("0x");
        let mut started = false;
        for shift in (0..std::mem::size_of::<usize>() * 8).step_by(4).rev() {
            let digit = (value >> shift) & 0xF;
            if digit == 0 && !started && shift != 0 {
                continue;
            }
            started = true;
            let ch = char::from_digit(u32::try_from(digit).unwrap_or(0), 16).unwrap_or('0');
            let mut buf = [0_u8; 4];
            self.push_str(ch.encode_utf8(&mut buf));
        }
    }

    fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.bytes[..self.len]).ok()
    }
}

/// Receives a formatted native-fault report. Must not panic, allocate or block.
type FaultReporter = fn(&str);

static FAULT_REPORTER: std::sync::OnceLock<FaultReporter> = std::sync::OnceLock::new();

unsafe extern "system" fn unhandled_exception_filter(info: *const EXCEPTION_POINTERS) -> i32 {
    let mut buffer = LineBuf::new();
    buffer.push_str("[fault]");
    if !info.is_null() {
        // SAFETY: the OS passes a valid EXCEPTION_POINTERS pointer; non-null was
        // just checked.
        let record = unsafe { (*info).ExceptionRecord };
        if !record.is_null() {
            // SAFETY: a non-null record pointer is owned by the OS for the
            // duration of the filter callback.
            let code = unsafe { (*record).ExceptionCode };
            // SAFETY: same record, read-only access.
            let address = unsafe { (*record).ExceptionAddress };
            buffer.push_str(" exception=");
            buffer.push_hex(code.unsigned_abs() as usize);
            buffer.push_str(" address=");
            buffer.push_hex(address as usize);
        }
    }
    buffer.push_str("\n");
    if let Some(text) = buffer.as_str()
        && let Some(reporter) = FAULT_REPORTER.get()
    {
        reporter(text);
    }
    EXCEPTION_CONTINUE_SEARCH
}

/// Install a reporter for native faults (access violations and similar).
///
/// The reporter runs on the faulting thread inside the OS exception dispatcher,
/// so it must only use the pre-opened crash file.
pub fn install_fault_reporter(reporter: FaultReporter) {
    let _ = FAULT_REPORTER.set(reporter);
    // SAFETY: the filter is an `extern "system"` function with the exact
    // signature Win32 requires and no captured state.
    unsafe { SetUnhandledExceptionFilter(Some(unhandled_exception_filter)) };
}

/// Whether the message loop should continue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopControl {
    /// Keep pumping messages.
    Continue,
    /// Stop the loop.
    Quit,
}

/// Create the calling thread's message queue if it does not exist yet.
///
/// A queue is created implicitly by the first message call, so this primes it
/// before worker threads start posting wake-ups.
pub fn ensure_message_queue() {
    // SAFETY: the message structure is only inspected by the call, which uses
    // `PM_NOREMOVE`; a null window means "thread messages".
    let mut message: windows_sys::Win32::UI::WindowsAndMessaging::MSG =
        unsafe { std::mem::zeroed() };
    // SAFETY: same as above; the call has no other preconditions.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PeekMessageW(
            &mut message,
            std::ptr::null_mut(),
            0,
            0,
            windows_sys::Win32::UI::WindowsAndMessaging::PM_NOREMOVE,
        );
    }
}

/// Run the thread's Win32 message loop.
///
/// `on_iteration` runs before the loop blocks and may drain a command channel;
/// `after_dispatch` runs after every dispatched message and may drain menu
/// events. Both return [`LoopControl::Quit`] to stop.
///
/// Returns `true` when the loop was stopped by a callback.
pub fn run_message_loop(
    mut on_iteration: impl FnMut() -> LoopControl,
    mut after_dispatch: impl FnMut() -> LoopControl,
) -> bool {
    ensure_message_queue();
    // SAFETY: `MSG` is a plain C structure, zero is a valid initial value.
    let mut message: windows_sys::Win32::UI::WindowsAndMessaging::MSG =
        unsafe { std::mem::zeroed() };
    loop {
        if on_iteration() == LoopControl::Quit {
            return true;
        }
        // SAFETY: `message` is a valid writable MSG and a null window requests
        // messages for the calling thread only.
        let result = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetMessageW(
                &mut message,
                std::ptr::null_mut(),
                0,
                0,
            )
        };
        if result <= 0 {
            return result == 0;
        }
        // SAFETY: `message` was filled by `GetMessageW`.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::TranslateMessage(&message);
            windows_sys::Win32::UI::WindowsAndMessaging::DispatchMessageW(&message);
        }
        if after_dispatch() == LoopControl::Quit {
            return true;
        }
    }
}
