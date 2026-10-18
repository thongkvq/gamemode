//! Win32 helpers: liệt kê / tắt process, ép nhả RAM, đọc trạng thái bộ nhớ,
//! quét app đang chạy (session người dùng), lấy command line và khởi chạy lại
//! app bằng quyền user thường.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem::{size_of, transmute, zeroed};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, STILL_ACTIVE, SYSTEMTIME,
    WPARAM,
};
use windows_sys::Win32::Security::{
    DuplicateTokenEx, GetTokenInformation, SecurityImpersonation, TokenElevation, TokenPrimary,
    TOKEN_DUPLICATE, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::ProcessStatus::{
    K32EmptyWorkingSet, K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::SystemInformation::{
    GetLocalTime, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessWithTokenW, GetCurrentProcess, GetExitCodeProcess, OpenProcess, OpenProcessToken,
    QueryFullProcessImageNameW, TerminateProcess, PROCESS_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE, PROCESS_VM_READ,
    STARTUPINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    IsWindowVisible, PostMessageW, GW_OWNER, WM_CLOSE,
};

/// ACCESS bị windows-sys đặt ở module khác -> dùng hằng số gốc của Win32.
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
/// LOGON_WITH_PROFILE không cần: token sao chép từ explorer cùng user hiện tại.
const LOGON_FLAGS: u32 = 0;
/// ProcessCommandLineInformation
const PROCESS_COMMAND_LINE_INFORMATION: u32 = 60;
/// ProcessBasicInformation
const PROCESS_BASIC_INFORMATION_CLASS: u32 = 0;
const STATUS_SUCCESS: i32 = 0;
const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004u32 as i32;

pub struct Proc {
    pub pid: u32,
    pub name: String,
}

/// Một process đang chạy kèm thông tin đủ để hiển thị và khôi phục lại sau này.
#[derive(Clone)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub session: u32,
    pub name: String,
    pub exe_path: Option<String>,
    pub cmdline: Option<String>,
    pub display_name: Option<String>,
    pub ws: u64,
}

pub fn list_processes() -> Vec<Proc> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = zeroed();
        e.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut e) != 0 {
            loop {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                out.push(Proc {
                    pid: e.th32ProcessID,
                    name: String::from_utf16_lossy(&e.szExeFile[..len]),
                });
                if Process32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    out
}

/// Quét toàn bộ process kèm session, exe, command line, tên hiển thị và RAM.
/// Tốn vài chục ms -> chỉ gọi trong thread nền.
pub fn snapshot() -> Vec<ProcInfo> {
    let mut out: Vec<ProcInfo> = Vec::new();
    let mut desc_cache: HashMap<String, Option<String>> = HashMap::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = zeroed();
        e.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut e) != 0 {
            loop {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                let name = String::from_utf16_lossy(&e.szExeFile[..len]);
                let pid = e.th32ProcessID;

                let mut info = ProcInfo {
                    pid,
                    ppid: e.th32ParentProcessID,
                    session: session_of(pid),
                    name,
                    exe_path: None,
                    cmdline: None,
                    display_name: None,
                    ws: 0,
                };

                if pid > 4 {
                    if let Some(h) = open_query(pid) {
                        info.ws = working_set(h);
                        info.exe_path = exe_path_of(h);
                        CloseHandle(h);
                    }
                    info.cmdline = command_line_of(pid);
                    if let Some(p) = info.exe_path.as_ref() {
                        let desc = desc_cache
                            .entry(p.to_lowercase())
                            .or_insert_with(|| file_description(p))
                            .clone();
                        info.display_name = desc;
                    }
                }
                out.push(info);

                if Process32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    out
}

/// Session ID (Terminal Services) của một process; 0 nếu không tra được.
pub fn session_of(pid: u32) -> u32 {
    unsafe {
        let mut s: u32 = 0;
        if ProcessIdToSessionId(pid, &mut s) != 0 {
            s
        } else {
            0
        }
    }
}

pub fn current_session() -> u32 {
    session_of(std::process::id())
}

/// Mở process với quyền tối thiểu cần cho việc đọc thông tin.
/// Ưu tiên có PROCESS_VM_READ (đọc working dir), thiếu thì lùi về LIMITED_INFORMATION.
fn open_query(pid: u32) -> Option<HANDLE> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, 0, pid);
        if !h.is_null() {
            return Some(h);
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        (!h.is_null()).then_some(h)
    }
}

unsafe fn working_set(h: HANDLE) -> u64 {
    let mut c: PROCESS_MEMORY_COUNTERS = zeroed();
    c.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    if K32GetProcessMemoryInfo(h, &mut c, c.cb) != 0 {
        c.WorkingSetSize as u64
    } else {
        0
    }
}

unsafe fn exe_path_of(h: HANDLE) -> Option<String> {
    let mut buf = vec![0u16; 32768];
    let mut size = buf.len() as u32;
    if QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut size) == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..size as usize]))
}

/// Command line đầy đủ của process qua NtQueryInformationProcess(ProcessCommandLineInformation).
/// Không mở được process (process hệ thống / đã thoát) -> None.
pub fn command_line_of(pid: u32) -> Option<String> {
    type NtQueryInformationProcessFn =
        unsafe extern "system" fn(HANDLE, u32, *mut c_void, u32, *mut u32) -> i32;

    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum_length: u16,
        buffer: *mut u16,
    }

    unsafe {
        let ntdll = GetModuleHandleW(to_wide("ntdll.dll").as_ptr());
        if ntdll.is_null() {
            return None;
        }
        let addr = GetProcAddress(ntdll, c"NtQueryInformationProcess".as_ptr() as *const u8)?;
        let query: NtQueryInformationProcessFn = transmute(addr);

        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }

        let mut cap: usize = 64 * 1024;
        let mut result = None;
        for _ in 0..3 {
            let mut buf = vec![0u8; cap];
            let mut ret_len: u32 = 0;
            let status = query(
                h,
                PROCESS_COMMAND_LINE_INFORMATION,
                buf.as_mut_ptr() as *mut c_void,
                cap as u32,
                &mut ret_len,
            );
            if status == STATUS_SUCCESS {
                let us = &*(buf.as_ptr() as *const UnicodeString);
                if !us.buffer.is_null() && us.length > 0 {
                    let chars = std::slice::from_raw_parts(us.buffer, (us.length / 2) as usize);
                    result = Some(String::from_utf16_lossy(chars));
                }
                break;
            } else if status == STATUS_INFO_LENGTH_MISMATCH && cap < 1024 * 1024 {
                cap *= 4;
                continue;
            } else {
                break;
            }
        }
        CloseHandle(h);
        result.filter(|s| !s.trim().is_empty())
    }
}

/// Thư mục làm việc hiện tại của process (best-effort, đọc PEB).
/// Chỉ hỗ trợ tiến trình 64-bit; cần quyền đọc bộ nhớ của process đó.
pub fn working_dir_of(pid: u32) -> Option<String> {
    if size_of::<usize>() != 8 {
        return None;
    }

    #[repr(C)]
    struct ProcessBasicInformation {
        exit_status: i32,
        peb_base_address: *mut c_void,
        affinity_mask: usize,
        base_priority: i32,
        unique_process_id: usize,
        inherited_from_unique_process_id: usize,
    }

    type NtQueryInformationProcessFn =
        unsafe extern "system" fn(HANDLE, u32, *mut c_void, u32, *mut u32) -> i32;

    unsafe fn read<T>(h: HANDLE, addr: *const c_void, out: &mut T) -> bool {
        let mut read = 0usize;
        ReadProcessMemory(
            h,
            addr,
            out as *mut T as *mut c_void,
            size_of::<T>(),
            &mut read,
        ) != 0
            && read == size_of::<T>()
    }

    unsafe {
        let ntdll = GetModuleHandleW(to_wide("ntdll.dll").as_ptr());
        if ntdll.is_null() {
            return None;
        }
        let addr = GetProcAddress(ntdll, c"NtQueryInformationProcess".as_ptr() as *const u8)?;
        let query: NtQueryInformationProcessFn = transmute(addr);

        let h = open_query(pid)?;
        let mut pbi: ProcessBasicInformation = zeroed();
        let status = query(
            h,
            PROCESS_BASIC_INFORMATION_CLASS,
            &mut pbi as *mut _ as *mut c_void,
            size_of::<ProcessBasicInformation>() as u32,
            std::ptr::null_mut(),
        );
        let mut out = None;
        if status == STATUS_SUCCESS && !pbi.peb_base_address.is_null() {
            // 64-bit PEB: ProcessParameters nằm ở offset 0x20.
            let mut params: *mut c_void = std::ptr::null_mut();
            let params_slot = (pbi.peb_base_address as usize + 0x20) as *const c_void;
            if read(h, params_slot, &mut params) && !params.is_null() {
                // RTL_USER_PROCESS_PARAMETERS: CurrentDirectory (CURDIR) ở offset 0x38.
                #[repr(C)]
                struct CurDirHead {
                    length: u16,
                    maximum_length: u16,
                    _pad: u32,
                    buffer: *mut u16,
                }
                let mut cur: CurDirHead = zeroed();
                let cur_slot = (params as usize + 0x38) as *const c_void;
                if read(h, cur_slot, &mut cur)
                    && !cur.buffer.is_null()
                    && cur.length > 2
                    && cur.length <= 4096
                {
                    let mut buf = vec![0u16; (cur.length / 2) as usize];
                    let bytes = buf.len() * 2;
                    let mut got = 0usize;
                    // Đọc vào vùng dữ liệu của Vec (không phải vào chính struct Vec).
                    if ReadProcessMemory(
                        h,
                        cur.buffer as *const c_void,
                        buf.as_mut_ptr() as *mut c_void,
                        bytes,
                        &mut got,
                    ) != 0
                        && got == bytes
                    {
                        let s = String::from_utf16_lossy(&buf);
                        let s = s.trim_end_matches('\0').trim().to_string();
                        if !s.is_empty() {
                            out = Some(s);
                        }
                    }
                }
            }
        }
        CloseHandle(h);
        out
    }
}

/// Tên hiển thị (FileDescription) trong version info của file exe.
pub fn file_description(path: &str) -> Option<String> {
    unsafe {
        let w = to_wide(path);
        let size = GetFileVersionInfoSizeW(w.as_ptr(), std::ptr::null_mut());
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if GetFileVersionInfoW(w.as_ptr(), 0, size, buf.as_mut_ptr() as *mut c_void) == 0 {
            return None;
        }
        let block = buf.as_ptr() as *const c_void;

        // Bảng Translation -> cặp (language, codepage) đầu tiên.
        let mut lang: u16 = 0x0409;
        let mut cp: u16 = 0x04B0;
        let mut ptr: *mut c_void = std::ptr::null_mut();
        let mut len: u32 = 0;
        let sub = to_wide(r"\VarFileInfo\Translation");
        if VerQueryValueW(block, sub.as_ptr(), &mut ptr, &mut len) != 0 && len >= 4 && !ptr.is_null() {
            let p = ptr as *const u16;
            lang = *p;
            cp = *p.add(1);
        }

        for (l, c) in [(lang, cp), (0x0409, 0x04B0), (0x0409, 0x04E4)] {
            let query = to_wide(&format!(r"\StringFileInfo\{:04x}{:04x}\FileDescription", l, c));
            let mut vptr: *mut c_void = std::ptr::null_mut();
            let mut vlen: u32 = 0;
            if VerQueryValueW(block, query.as_ptr(), &mut vptr, &mut vlen) != 0
                && vlen > 1
                && !vptr.is_null()
            {
                let s = String::from_utf16_lossy(std::slice::from_raw_parts(
                    vptr as *const u16,
                    (vlen - 1) as usize,
                ));
                let s = s.trim().to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
        None
    }
}

/// Tắt process, trả về số byte RAM nó đang dùng (None nếu không tắt được).
pub fn kill(pid: u32) -> Option<u64> {
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let ws = working_set(h);
        let ok = TerminateProcess(h, 1) != 0;
        CloseHandle(h);
        ok.then_some(ws)
    }
}

pub fn is_alive(pid: u32) -> bool {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(h, &mut code) != 0;
        CloseHandle(h);
        ok && code == STILL_ACTIVE as u32
    }
}

unsafe extern "system" fn close_proc_windows(hwnd: HWND, lparam: LPARAM) -> i32 {
    let target = lparam as u32;
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == target {
        // WM_CLOSE = yêu cầu đóng "lịch sự" để app kịp lưu phiên làm việc.
        PostMessageW(hwnd, WM_CLOSE, 0 as WPARAM, 0);
    }
    1
}

/// Gửi WM_CLOSE tới mọi cửa sổ top-level của các process, chờ tối đa `timeout`,
/// trả về số process vẫn còn sống sau khi chờ.
pub fn close_gracefully(pids: &[u32], timeout: Duration) -> usize {
    if pids.is_empty() {
        return 0;
    }
    unsafe {
        for &pid in pids {
            EnumWindows(Some(close_proc_windows), pid as LPARAM);
        }
    }
    let deadline = Instant::now() + timeout;
    loop {
        let alive: Vec<u32> = pids.iter().copied().filter(|&p| is_alive(p)).collect();
        if alive.is_empty() || Instant::now() >= deadline {
            return alive.len();
        }
        std::thread::sleep(Duration::from_millis(80));
    }
}

/// Cửa sổ top-level "thật" của app người dùng: đang hiện, không có owner (không phải hộp thoại /
/// cửa sổ công cụ của app khác) và tiêu đề khác rỗng. Dùng để biết một exe là app người dùng thật
/// thay vì process nền / helper.
unsafe fn is_user_window(hwnd: HWND) -> bool {
    if IsWindowVisible(hwnd) == 0 {
        return false;
    }
    if !GetWindow(hwnd, GW_OWNER).is_null() {
        return false;
    }
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return false;
    }
    let mut buf = vec![0u16; len as usize + 1];
    let got = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    got > 0 && !String::from_utf16_lossy(&buf[..got as usize]).trim().is_empty()
}

/// Một lượt EnumWindows: tập PID đang sở hữu ít nhất một cửa sổ top-level của app người dùng.
/// Gọi TRƯỚC khi gửi WM_CLOSE, vì sau đó app đã đóng và không còn cửa sổ để đọc.
pub fn pids_with_user_window() -> HashSet<u32> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
        let set = &mut *(lparam as *mut HashSet<u32>);
        if is_user_window(hwnd) {
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid != 0 {
                set.insert(pid);
            }
        }
        1
    }

    let mut set: HashSet<u32> = HashSet::new();
    unsafe {
        EnumWindows(Some(collect), &mut set as *mut HashSet<u32> as LPARAM);
    }
    set
}

/// Lệnh tự khởi động của user/máy: `HKCU\...\CurrentVersion\Run` và `HKLM\...\CurrentVersion\Run`.
/// App chạy ẩn dưới khay (Zalo, OneDrive...) không có cửa sổ top-level nên nhận diện qua đây.
pub fn startup_commands() -> Vec<String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let mut out: Vec<String> = Vec::new();
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let Ok(key) = RegKey::predef(root).open_subkey_with_flags(RUN, KEY_READ) else {
            continue;
        };
        for (_, value) in key.enum_values().flatten() {
            let s = value.to_string();
            if !s.trim().is_empty() {
                out.push(s);
            }
        }
    }
    out
}


pub fn trim(pid: u32) -> u64 {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA, 0, pid);
        if h.is_null() {
            return 0;
        }
        let before = working_set(h);
        K32EmptyWorkingSet(h);
        let after = working_set(h);
        CloseHandle(h);
        before.saturating_sub(after)
    }
}

#[derive(Clone, Copy, Default)]
pub struct Mem {
    pub total: u64,
    pub avail: u64,
    pub commit_used: u64,
    pub commit_limit: u64,
}

pub fn memory() -> Mem {
    unsafe {
        let mut m: MEMORYSTATUSEX = zeroed();
        m.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut m) == 0 {
            return Mem::default();
        }
        Mem {
            total: m.ullTotalPhys,
            avail: m.ullAvailPhys,
            commit_used: m.ullTotalPageFile - m.ullAvailPageFile,
            commit_limit: m.ullTotalPageFile,
        }
    }
}

/// Giờ hệ thống dạng "YYYY-MM-DD HH:MM:SS" (giờ địa phương).
pub fn local_time_string() -> String {
    unsafe {
        let mut st: SYSTEMTIME = zeroed();
        GetLocalTime(&mut st);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
        )
    }
}

pub fn is_admin() -> bool {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation: TOKEN_ELEVATION = zeroed();
        let mut ret_len = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Token của shell (explorer.exe) đang chạy cùng session: token này KHÔNG elevated,
/// dùng để mở app con bằng quyền người dùng thường dù tool đang chạy admin.
unsafe fn shell_token() -> Option<HANDLE> {
    let my_session = current_session();
    let mut source: Option<u32> = None;
    for p in list_processes() {
        if p.name.eq_ignore_ascii_case("explorer.exe") && session_of(p.pid) == my_session {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, p.pid);
            if h.is_null() {
                continue;
            }
            let mut tok: HANDLE = std::ptr::null_mut();
            let mut elevated = false;
            if OpenProcessToken(h, TOKEN_QUERY, &mut tok) != 0 {
                let mut e: TOKEN_ELEVATION = zeroed();
                let mut ret = 0u32;
                if GetTokenInformation(
                    tok,
                    TokenElevation,
                    &mut e as *mut _ as *mut _,
                    size_of::<TOKEN_ELEVATION>() as u32,
                    &mut ret,
                ) != 0
                {
                    elevated = e.TokenIsElevated != 0;
                }
                CloseHandle(tok);
            }
            CloseHandle(h);
            if !elevated {
                source = Some(p.pid);
                break;
            }
        }
    }

    let pid = source?;
    let ph = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if ph.is_null() {
        return None;
    }
    let mut tok: HANDLE = std::ptr::null_mut();
    if OpenProcessToken(ph, TOKEN_DUPLICATE | TOKEN_QUERY, &mut tok) == 0 {
        CloseHandle(ph);
        return None;
    }
    let mut dup: HANDLE = std::ptr::null_mut();
    let ok = DuplicateTokenEx(
        tok,
        MAXIMUM_ALLOWED,
        std::ptr::null(),
        SecurityImpersonation,
        TokenPrimary,
        &mut dup,
    );
    CloseHandle(tok);
    CloseHandle(ph);
    if ok == 0 {
        None
    } else {
        Some(dup)
    }
}

/// Khởi chạy app bằng quyền user thường (de-elevate) với command line + thư mục làm việc.
/// Ưu tiên CreateProcessWithTokenW bằng token của explorer (giữ được tham số dòng lệnh và cwd);
/// thất bại thì lùi về `explorer.exe "<path>"` - chỉ khi file exe còn tồn tại.
/// `Err` chứa mã lỗi / lý do cụ thể để Restore ghi vào log.
pub fn launch_as_standard_user(
    cmdline: &str,
    exe_path: &str,
    cwd: Option<&str>,
) -> Result<(), String> {
    // Lý do thất bại của cách 1, ghép vào thông báo lỗi nếu cách 2 cũng hỏng.
    let mut reason = "không lấy được token của explorer.exe".to_string();

    if let Some(token) = unsafe { shell_token() } {
        unsafe {
            let mut app = to_wide(exe_path);
            // lpCommandLine phải là buffer ghi được (API có thể sửa nội dung).
            let mut cmd = to_wide(cmdline);
            let cwd_w = cwd.map(|d| to_wide(d));
            let mut si: STARTUPINFOW = zeroed();
            si.cb = size_of::<STARTUPINFOW>() as u32;
            let mut pi: PROCESS_INFORMATION = zeroed();
            let ok = CreateProcessWithTokenW(
                token,
                LOGON_FLAGS,
                app.as_mut_ptr(),
                cmd.as_mut_ptr(),
                0,
                std::ptr::null(),
                cwd_w.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
                &si,
                &mut pi,
            );
            let err = if ok != 0 { 0 } else { GetLastError() };
            CloseHandle(token);
            if ok != 0 {
                CloseHandle(pi.hProcess);
                CloseHandle(pi.hThread);
                return Ok(());
            }
            reason = format!("CreateProcessWithTokenW lỗi {}", err);
        }
    }

    // Không còn file exe (app đã cập nhật sang thư mục phiên bản mới, ví dụ Zalo-26.10.11)
    // -> báo rõ, không thử explorer.exe vì chắc chắn cũng hỏng.
    if !std::path::Path::new(exe_path).is_file() {
        return Err(format!("không còn file exe ({})", reason));
    }

    use std::os::windows::process::CommandExt;
    match std::process::Command::new("explorer")
        .arg(exe_path)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn()
    {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("explorer.exe lỗi: {} ({})", e, reason)),
    }
}

pub fn is_service_running(name: &str) -> bool {
    unsafe {
        use windows_sys::Win32::System::Services::{
            CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatus,
            SC_MANAGER_CONNECT, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_STATUS,
        };

        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
        if scm.is_null() {
            return false;
        }
        let svc_name = to_wide(name);
        let svc = OpenServiceW(scm, svc_name.as_ptr(), SERVICE_QUERY_STATUS);
        if svc.is_null() {
            CloseServiceHandle(scm);
            return false;
        }
        let mut status: SERVICE_STATUS = zeroed();
        let ok = QueryServiceStatus(svc, &mut status);
        CloseServiceHandle(svc);
        CloseServiceHandle(scm);
        ok != 0 && status.dwCurrentState == SERVICE_RUNNING
    }
}

fn stop_service_blocking(name: &str) -> Result<(), u32> {
    unsafe {
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::System::Services::{
            CloseServiceHandle, ControlService, OpenSCManagerW, OpenServiceW,
            SC_MANAGER_CONNECT, SERVICE_CONTROL_STOP, SERVICE_STATUS, SERVICE_STOP,
        };

        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
        if scm.is_null() {
            return Err(GetLastError());
        }
        let svc_name = to_wide(name);
        let svc = OpenServiceW(scm, svc_name.as_ptr(), SERVICE_STOP);
        if svc.is_null() {
            let err = GetLastError();
            CloseServiceHandle(scm);
            return Err(err);
        }
        let mut status: SERVICE_STATUS = zeroed();
        let ok = ControlService(svc, SERVICE_CONTROL_STOP, &mut status);
        let err = if ok != 0 { 0 } else { GetLastError() };
        CloseServiceHandle(svc);
        CloseServiceHandle(scm);
        if ok != 0 {
            Ok(())
        } else {
            Err(err)
        }
    }
}

fn start_service_blocking(name: &str) -> Result<(), u32> {
    unsafe {
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::System::Services::{
            CloseServiceHandle, OpenSCManagerW, OpenServiceW, StartServiceW,
            SC_MANAGER_CONNECT, SERVICE_START,
        };

        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
        if scm.is_null() {
            return Err(GetLastError());
        }
        let svc_name = to_wide(name);
        let svc = OpenServiceW(scm, svc_name.as_ptr(), SERVICE_START);
        if svc.is_null() {
            let err = GetLastError();
            CloseServiceHandle(scm);
            return Err(err);
        }
        let ok = StartServiceW(svc, 0, std::ptr::null());
        let err = if ok != 0 { 0 } else { GetLastError() };
        CloseServiceHandle(svc);
        CloseServiceHandle(scm);
        if ok != 0 {
            Ok(())
        } else {
            Err(err)
        }
    }
}

/// ERROR_SERVICE_REQUEST_TIMEOUT: service không phản hồi lệnh trong thời gian chờ.
pub const SERVICE_TIMEOUT: u32 = 1053;

/// Chạy lệnh service trên thread riêng, chờ tối đa `SERVICE_WAIT`. Một số service
/// (ví dụ ClickToRunSvc) phản hồi rất chậm và làm ControlService/StartService treo lâu;
/// quá hạn thì trả SERVICE_TIMEOUT, lệnh vẫn tiếp tục chạy ngầm.
fn with_timeout(name: &str, f: fn(&str) -> Result<(), u32>) -> Result<(), u32> {
    const SERVICE_WAIT: Duration = Duration::from_secs(5);
    let (tx, rx) = std::sync::mpsc::channel();
    let owned = name.to_string();
    std::thread::spawn(move || {
        let _ = tx.send(f(&owned));
    });
    rx.recv_timeout(SERVICE_WAIT).unwrap_or(Err(SERVICE_TIMEOUT))
}

pub fn stop_service(name: &str) -> Result<(), u32> {
    with_timeout(name, stop_service_blocking)
}

pub fn start_service(name: &str) -> Result<(), u32> {
    with_timeout(name, start_service_blocking)
}
