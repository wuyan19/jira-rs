//! Where CLI output is written and in what encoding.
//!
//! `jira` produces UTF-8. That is right for a terminal (the Windows console
//! takes UTF-16 and `std` converts), Git Bash, redirected files and the MCP
//! transport. PowerShell is the exception: it reads a native program's
//! output through a pipe and decodes it with the console output code page
//! (936/GBK on a stock zh-CN Windows), so in a Windows PowerShell window
//! every Chinese string came out as mojibake — even from a plain
//! `jira get X`.
//!
//! Rather than have users reconfigure their shell, output that PowerShell is
//! reading is encoded in that code page. PowerShell is recognised as the
//! process that started `jira`; everything else keeps UTF-8. Characters the
//! code page cannot represent become `\uXXXX` escapes inside JSON, so no
//! data is lost. The MCP server does not use this module.
//!
//! All CLI printing goes through [`json`], [`error`] and [`progress`]; CI
//! rejects raw `print!`-style macros in the CLI modules.

use std::{borrow::Cow, io::{ErrorKind, Write}};

use serde_json::Value;

#[derive(Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
}

/// Print `value` as indented JSON on stdout.
pub fn json(value: &Value) {
    emit(Stream::Stdout, &encode(&serde_json::to_string_pretty(value).unwrap(), Stream::Stdout, true), b"\n");
}

/// Print a diagnostic line on stderr.
pub fn error(message: &str) {
    emit(Stream::Stderr, &encode(message, Stream::Stderr, false), b"\n");
}

/// Print `text` on stderr without a trailing newline (progress lines).
pub fn progress(text: &str) {
    emit(Stream::Stderr, &encode(text, Stream::Stderr, false), b"");
}

fn emit(stream: Stream, bytes: &[u8], tail: &[u8]) {
    let written = match stream {
        Stream::Stdout => {
            let mut out = std::io::stdout().lock();
            out.write_all(bytes).and_then(|_| out.write_all(tail)).and_then(|_| out.flush())
        }
        Stream::Stderr => {
            let mut err = std::io::stderr().lock();
            err.write_all(bytes).and_then(|_| err.write_all(tail))
        }
    };
    match written {
        // The reader went away (`jira list | head`); nothing left to report to.
        Err(e) if e.kind() == ErrorKind::BrokenPipe => {}
        Err(e) => panic!("failed printing: {e}"),
        Ok(()) => {}
    }
}

/// `text` as the bytes to write to `stream`. With `escape` set, a JSON
/// document stays lossless when the code page lacks a character.
fn encode<'a>(text: &'a str, stream: Stream, escape: bool) -> Cow<'a, [u8]> {
    #[cfg(not(windows))]
    {
        let _ = (stream, escape);
        return Cow::Borrowed(text.as_bytes());
    }

    #[cfg(windows)]
    match platform::pipe_code_page(stream) {
        None => Cow::Borrowed(text.as_bytes()),
        Some(code_page) => match platform::encode(text, code_page) {
            // A code page that cannot be produced this way: stay UTF-8.
            None => Cow::Borrowed(text.as_bytes()),
            Some((bytes, true)) => Cow::Owned(bytes),
            // Diagnostics tolerate `?` for what the code page lacks.
            Some((bytes, false)) if !escape => Cow::Owned(bytes),
            Some((_, false)) => {
                let escaped = escape_unencodable(text, |c| platform::is_encodable(c, code_page));
                Cow::Owned(platform::encode(&escaped, code_page).map_or_else(|| escaped.into_bytes(), |(b, _)| b))
            }
        },
    }
}

/// Code page for output PowerShell is reading: PowerShell decodes with the
/// console output code page, so that is what must be produced — unless it is
/// already UTF-8 (65001) or unknown (0: no console attached).
#[cfg(any(windows, test))]
fn code_page(handle_is_pipe: bool, console_code_page: u32) -> Option<u32> {
    (handle_is_pipe && console_code_page != 0 && console_code_page != 65001).then_some(console_code_page)
}

/// Windows PowerShell (`powershell.exe`) and PowerShell 7+ (`pwsh.exe`).
#[cfg(any(windows, test))]
fn is_powershell(image: &str) -> bool {
    image.eq_ignore_ascii_case("powershell.exe") || image.eq_ignore_ascii_case("pwsh.exe")
}

/// `json` with every character `encodable` rejects rewritten as `\uXXXX`
/// (two escapes for an astral character). Outside string literals JSON is
/// ASCII, so only string contents can be affected.
fn escape_unencodable(json: &str, encodable: impl Fn(char) -> bool) -> String {
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        if c.is_ascii() || encodable(c) {
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

#[cfg(windows)]
mod platform {
    use std::{ptr::addr_eq, sync::OnceLock};

    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Globalization::{WC_NO_BEST_FIT_CHARS, WideCharToMultiByte};
    use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType};
    use windows_sys::Win32::System::Console::{GetConsoleOutputCP, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    use super::{code_page, is_powershell, Stream};

    /// Code page to encode `stream` with; `None` means UTF-8.
    pub fn pipe_code_page(stream: Stream) -> Option<u32> {
        if forced_utf8() {
            return None;
        }
        static POWERSHELL_PARENT: OnceLock<bool> = OnceLock::new();
        if !*POWERSHELL_PARENT.get_or_init(|| parent_image().is_some_and(|n| is_powershell(&n))) {
            return None;
        }
        let handle = unsafe {
            GetStdHandle(match stream {
                Stream::Stdout => STD_OUTPUT_HANDLE,
                Stream::Stderr => STD_ERROR_HANDLE,
            })
        };
        // A terminal writes UTF-16 through the console API and a file should
        // stay UTF-8; only a pipe is decoded by PowerShell itself.
        let is_pipe = !handle.is_null() && !addr_eq(handle, INVALID_HANDLE_VALUE)
            && unsafe { GetFileType(handle) } == FILE_TYPE_PIPE;
        code_page(is_pipe, unsafe { GetConsoleOutputCP() })
    }

    fn forced_utf8() -> bool {
        std::env::var("JIRA_OUTPUT_ENCODING")
            .is_ok_and(|v| v.trim().eq_ignore_ascii_case("utf-8") || v.trim().eq_ignore_ascii_case("utf8"))
    }

    /// `text` in `code_page`, and whether every character was representable.
    /// `None` for a code page that cannot be produced this way.
    pub fn encode(text: &str, code_page: u32) -> Option<(Vec<u8>, bool)> {
        if text.is_empty() {
            return Some((Vec::new(), true));
        }
        let wide: Vec<u16> = text.encode_utf16().collect();
        let (ptr, len) = (wide.as_ptr(), wide.len() as i32);
        let mut used_default = 0i32;   // LPBOOL
        let needed = unsafe {
            WideCharToMultiByte(code_page, WC_NO_BEST_FIT_CHARS, ptr, len, std::ptr::null_mut(), 0, std::ptr::null(), &mut used_default)
        };
        if needed <= 0 {
            return None;
        }
        let mut bytes = vec![0u8; needed as usize];
        used_default = 0;
        let written = unsafe {
            WideCharToMultiByte(
                code_page,
                WC_NO_BEST_FIT_CHARS,
                ptr,
                len,
                bytes.as_mut_ptr(),
                needed,
                std::ptr::null(),
                &mut used_default,
            )
        };
        (written == needed).then(|| (bytes, used_default == 0))
    }

    /// Characters with only a "best fit" substitute count as unrepresentable:
    /// a look-alike is still a wrong character.
    pub fn is_encodable(c: char, code_page: u32) -> bool {
        let mut units = [0u16; 2];
        let wide = c.encode_utf16(&mut units);
        let mut byte = [0u8; 4];
        let mut used_default = 0i32;   // LPBOOL
        let written = unsafe {
            WideCharToMultiByte(
                code_page,
                WC_NO_BEST_FIT_CHARS,
                wide.as_ptr(),
                wide.len() as i32,
                byte.as_mut_ptr(),
                byte.len() as i32,
                std::ptr::null(),
                &mut used_default,
            )
        };
        written > 0 && used_default == 0
    }

    /// Image name (e.g. `powershell.exe`) of the process that started this one.
    fn parent_image() -> Option<String> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if addr_eq(snapshot, INVALID_HANDLE_VALUE) {
            return None;
        }
        let me = unsafe { GetCurrentProcessId() };
        let parent_pid = find(snapshot, me).map(|e| e.th32ParentProcessID);
        let parent = parent_pid.and_then(|pid| find(snapshot, pid));
        unsafe { CloseHandle(snapshot) };
        parent.map(|e| String::from_utf16_lossy(&e.szExeFile[..end_of(&e.szExeFile)]))
    }

    fn find(snapshot: windows_sys::Win32::Foundation::HANDLE, pid: u32) -> Option<PROCESSENTRY32W> {
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        unsafe {
            let mut more = Process32FirstW(snapshot, &mut entry);
            while more != 0 {
                if entry.th32ProcessID == pid {
                    return Some(entry);
                }
                more = Process32NextW(snapshot, &mut entry);
            }
        }
        None
    }

    fn end_of(path: &[u16]) -> usize {
        path.iter().position(|&u| u == 0).unwrap_or(path.len())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn powershell_reads_the_console_code_page() {
        assert_eq!(code_page(true, 936), Some(936));
        assert_eq!(code_page(true, 437), Some(437));
    }

    #[test]
    fn no_transcoding_for_utf8_consoles_pipes_read_by_others_or_no_console() {
        assert_eq!(code_page(true, 65001), None, "chcp 65001 / Windows Terminal");
        assert_eq!(code_page(true, 0), None, "no console attached");
        assert_eq!(code_page(false, 936), None, "terminal, file, or a pipe read by something else");
    }

    #[test]
    fn recognizes_both_powershell_hosts_and_nothing_else() {
        for image in ["powershell.exe", "pwsh.exe", "PowerShell.EXE", "PWSH.exe"] {
            assert!(is_powershell(image), "{image}");
        }
        for image in ["powershell_ise.exe", "cmd.exe", "bash.exe", "node.exe", "mypowershell.exe", "powershell"] {
            assert!(!is_powershell(image), "{image}");
        }
    }

    #[test]
    fn escapes_exactly_the_unencodable_characters() {
        let json = r#"{"summary":"café 🚀","n":1}"#;
        assert_eq!(escape_unencodable(json, |c| c != '🚀'), r#"{"summary":"café \ud83d\ude80","n":1}"#);
    }

    #[test]
    fn an_escaped_document_is_still_the_same_json() {
        let original = json!({ "summary": "测试 🚀 é done", "labels": ["中文"], "ok": true });
        let text = serde_json::to_string_pretty(&original).unwrap();
        let escaped = escape_unencodable(&text, |c| c.is_ascii());
        assert!(escaped.is_ascii());
        assert_eq!(serde_json::from_str::<Value>(&escaped).unwrap(), original);
    }
}
