//! Win32 helpers: liệt kê / tắt process, đọc & xả danh sách trang RAM (standby list),
//! quét app đang chạy (session người dùng), lấy command line và khởi chạy lại
//! app bằng quyền user thường, Wi-Fi (WLAN API), registry Game DVR / GPU và Task Scheduler.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem::{size_of, transmute, zeroed};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, FILETIME, GetLastError, HANDLE, HWND, INVALID_HANDLE_VALUE,
    LPARAM, LUID, STILL_ACTIVE, SYSTEMTIME, WPARAM,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetBestRoute, ICMP_ECHO_REPLY, IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho, MIB_IPFORWARDROW,
};
use windows_sys::Win32::NetworkManagement::WiFi::{
    WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory, WlanOpenHandle, WlanQueryInterface,
    WlanSetInterface, wlan_interface_state_connected, wlan_intf_opcode_autoconf_enabled,
    WLAN_INTERFACE_INFO_LIST,
};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, DuplicateTokenEx, GetTokenInformation, LookupPrivilegeValueW,
    SecurityImpersonation, TokenElevation, TokenPrimary, SE_PRIVILEGE_ENABLED,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_DUPLICATE, TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY,
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
    CreateMutexW, CreateProcessWithTokenW, GetCurrentProcess, GetExitCodeProcess, GetSystemTimes,
    OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, TerminateProcess,
    PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    PROCESS_VM_READ, STARTUPINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowW, GetWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowLongPtrW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, PostMessageW,
    SetForegroundWindow, SetWindowLongPtrW, ShowWindow, GWL_EXSTYLE, GW_OWNER, SW_HIDE,
    SW_RESTORE, SW_SHOW, SW_SHOWMINNOACTIVE, WM_CLOSE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
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

/// Một dòng cây process chỉ lấy từ Toolhelp32 (pid, ppid, tên exe viết thường):
/// KHÔNG mở handle process nào, nên dùng được để tính tập PID cần bảo vệ
/// TRƯỚC khi quét sâu.
pub struct ProcNode {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
}

/// Cây process nhẹ (Toolhelp32), không mở quyền gì trên process.
pub fn process_nodes() -> Vec<ProcNode> {
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
                out.push(ProcNode {
                    pid: e.th32ProcessID,
                    ppid: e.th32ParentProcessID,
                    name: String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase(),
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
/// Process có tên trong `skip` (game + anti-cheat ACE) chỉ được mở bằng
/// PROCESS_QUERY_LIMITED_INFORMATION để lấy đường dẫn exe — KHÔNG yêu cầu quyền đọc bộ nhớ,
/// không đọc command line / thư mục làm việc / working set. `skip` là tên exe viết thường
/// (khớp đúng hoặc khớp tiền tố, ví dụ "ace-").
/// `protect` = PID thuộc "họ" process của game (launcher / tổ tiên / con cháu): dù tên exe
/// không nằm trong `skip` thì cũng chỉ được mở bằng PROCESS_QUERY_LIMITED_INFORMATION.
pub fn snapshot_skip(skip: &[String], protect: &HashSet<u32>) -> Vec<ProcInfo> {
    snapshot_inner(skip, protect)
}

fn is_skipped(name_lower: &str, skip: &[String]) -> bool {
    !skip.is_empty()
        && skip
            .iter()
            .any(|s| name_lower == s.as_str() || name_lower.starts_with(s.as_str()))
}

fn snapshot_inner(skip: &[String], protect: &HashSet<u32>) -> Vec<ProcInfo> {
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
                    if protect.contains(&pid) || is_skipped(&info.name.to_lowercase(), skip) {
                        // Game / anti-cheat / họ process game: chỉ đọc đường dẫn exe,
                        // không đụng tới bộ nhớ.
                        if let Some(h) = open_limited(pid) {
                            info.exe_path = exe_path_of(h);
                            CloseHandle(h);
                        }
                    } else {
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

/// Mở process chỉ để đọc thông tin cơ bản (không có quyền đọc bộ nhớ / ghi).
fn open_limited(pid: u32) -> Option<HANDLE> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        (!h.is_null()).then_some(h)
    }
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

/// Nén working set của một process: đẩy các trang ít dùng ra standby list.
/// Trang bị nén KHÔNG mất — nằm lại trong standby làm cache nên Windows vẫn tính vào
/// "RAM khả dụng", và process cần lại thì lấy về ngay (không phải đọc pagefile).
/// Trả về số byte working set giảm được (0 nếu không mở được process).
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

/// Working set hiện tại của một process (0 nếu không đọc được).
pub fn working_set_of(pid: u32) -> u64 {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return 0;
        }
        let ws = working_set(h);
        CloseHandle(h);
        ws
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


/// Thông tin danh sách trang bộ nhớ (`SystemMemoryListInformation` = 80):
/// số byte hoàn toàn trống và số byte đang nằm trong standby list.
#[derive(Clone, Copy, Default)]
pub struct MemLists {
    pub free_bytes: u64,
    pub standby_bytes: u64,
}

/// SYSTEM_MEMORY_LIST_INFORMATION (toàn bộ trường đều là ULONG_PTR).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SystemMemoryListInformation {
    zero_page_count: usize,
    free_page_count: usize,
    modified_page_count: usize,
    modified_no_write_page_count: usize,
    bad_page_count: usize,
    page_count_by_priority: [usize; 8],
    repurposed_pages_by_priority: [usize; 8],
    modified_page_count_page_file: usize,
}

/// SystemMemoryListInformation
const SYSTEM_MEMORY_LIST_INFORMATION: u32 = 80;
/// MemoryPurgeStandbyList (tham số của NtSetSystemInformation class 80).
const MEMORY_PURGE_STANDBY_LIST: u32 = 4;
const PAGE_BYTES: u64 = 4096;
/// ERROR_NOT_ALL_ASSIGNED: token không được cấp quyền vừa yêu cầu.
const ERROR_NOT_ALL_ASSIGNED: u32 = 1300;

type NtQuerySystemInformationFn =
    unsafe extern "system" fn(u32, *mut c_void, u32, *mut u32) -> i32;
type NtSetSystemInformationFn = unsafe extern "system" fn(u32, *mut c_void, u32) -> i32;

/// Lấy hàm trong ntdll.dll theo tên (đều là hàm không export qua import lib).
unsafe fn ntdll_proc(name: *const u8) -> Option<unsafe extern "system" fn() -> isize> {
    let ntdll = GetModuleHandleW(to_wide("ntdll.dll").as_ptr());
    if ntdll.is_null() {
        return None;
    }
    GetProcAddress(ntdll, name)
}

/// Đọc số byte free / standby của hệ thống. `None` nếu không gọi được API.
pub fn memory_lists() -> Option<MemLists> {
    unsafe {
        let p = ntdll_proc(c"NtQuerySystemInformation".as_ptr() as *const u8)?;
        let query: NtQuerySystemInformationFn = transmute(p);

        // Windows trả STATUS_INFO_LENGTH_MISMATCH cho class này dù buffer đã đủ;
        // cấp dư 16 byte và chấp nhận cả hai mã trả về.
        let mut buf = vec![0u8; size_of::<SystemMemoryListInformation>() + 16];
        let mut ret_len: u32 = 0;
        let status = query(
            SYSTEM_MEMORY_LIST_INFORMATION,
            buf.as_mut_ptr() as *mut c_void,
            buf.len() as u32,
            &mut ret_len,
        );
        if status != STATUS_SUCCESS && status != STATUS_INFO_LENGTH_MISMATCH {
            return None;
        }
        let info = &*(buf.as_ptr() as *const SystemMemoryListInformation);

        let free_pages = info.zero_page_count.saturating_add(info.free_page_count);
        let standby_pages: usize = info.page_count_by_priority.iter().sum();
        Some(MemLists {
            free_bytes: free_pages as u64 * PAGE_BYTES,
            standby_bytes: standby_pages as u64 * PAGE_BYTES,
        })
    }
}

/// Bật một quyền của token hiện tại (cần cho purge standby: SeProfileSingleProcessPrivilege).
unsafe fn enable_privilege(name: &str) -> Result<(), String> {
    let mut token: HANDLE = std::ptr::null_mut();
    if OpenProcessToken(
        GetCurrentProcess(),
        TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
        &mut token,
    ) == 0
    {
        return Err(format!("OpenProcessToken lỗi {}", GetLastError()));
    }

    let mut luid: LUID = zeroed();
    let wide = to_wide(name);
    if LookupPrivilegeValueW(std::ptr::null(), wide.as_ptr(), &mut luid) == 0 {
        let err = GetLastError();
        CloseHandle(token);
        return Err(format!("LookupPrivilegeValueW({}) lỗi {}", name, err));
    }

    let mut tp: TOKEN_PRIVILEGES = zeroed();
    tp.PrivilegeCount = 1;
    tp.Privileges[0].Luid = luid;
    tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
    let ok = AdjustTokenPrivileges(
        token,
        0,
        &mut tp,
        0,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
    );
    let err = GetLastError();
    CloseHandle(token);
    if ok == 0 {
        return Err(format!("AdjustTokenPrivileges lỗi {}", err));
    }
    if err == ERROR_NOT_ALL_ASSIGNED {
        return Err(format!("token không được cấp quyền {}", name));
    }
    Ok(())
}

/// Xả standby list: giải phóng các trang RAM đang chờ (cache) — nhẹ nhàng hơn
/// EmptyWorkingSet vì không đẩy working set của app đang chạy ra pagefile.
pub fn purge_standby() -> Result<(), String> {
    unsafe {
        let p = ntdll_proc(c"NtSetSystemInformation".as_ptr() as *const u8)
            .ok_or_else(|| "không tìm thấy NtSetSystemInformation".to_string())?;
        let set: NtSetSystemInformationFn = transmute(p);

        // Quyền chỉ có ý nghĩa khi tool chạy elevated; thiếu thì báo rõ thay vì lỗi khó hiểu.
        enable_privilege("SeProfileSingleProcessPrivilege")?;

        let mut command: u32 = MEMORY_PURGE_STANDBY_LIST;
        let status = set(
            SYSTEM_MEMORY_LIST_INFORMATION,
            &mut command as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        );
        if status == STATUS_SUCCESS {
            Ok(())
        } else {
            Err(format!(
                "NtSetSystemInformation lỗi 0x{:08X}",
                status as u32
            ))
        }
    }
}

/// Một interface Wi-Fi kèm trạng thái quét nền (autoconf).
#[derive(Clone)]
pub struct WlanIface {
    pub guid: String,
    pub description: String,
    pub state: u32,
    pub connected: bool,
    /// Trạng thái "tự động cấu hình" (quét nền) nếu interface đang kết nối.
    pub autoconf: Option<bool>,
}

unsafe fn wlan_open() -> Result<HANDLE, String> {
    let mut negotiated: u32 = 0;
    let mut handle: HANDLE = std::ptr::null_mut();
    // Phiên bản client 2 = Windows Vista trở lên.
    let rc = WlanOpenHandle(2, std::ptr::null(), &mut negotiated, &mut handle);
    if rc != 0 {
        return Err(format!("WlanOpenHandle lỗi {}", rc));
    }
    Ok(handle)
}

fn guid_to_string(g: &windows_sys::core::GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7]
    )
}

fn guid_from_string(s: &str) -> Option<windows_sys::core::GUID> {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 32 {
        return None;
    }
    let n = |from: usize, to: usize| u32::from_str_radix(&hex[from..to], 16).ok();
    let mut data4 = [0u8; 8];
    for (i, item) in data4.iter_mut().enumerate() {
        *item = u8::from_str_radix(&hex[16 + i * 2..18 + i * 2], 16).ok()?;
    }
    Some(windows_sys::core::GUID {
        data1: n(0, 8)?,
        data2: n(8, 12)? as u16,
        data3: n(12, 16)? as u16,
        data4,
    })
}

fn utf16_trim(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Liệt kê interface Wi-Fi qua WLAN API (không phụ thuộc ngôn ngữ như netsh).
pub fn wlan_interfaces() -> Result<Vec<WlanIface>, String> {
    unsafe {
        let handle = wlan_open()?;
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        let rc = WlanEnumInterfaces(handle, std::ptr::null(), &mut list);
        if rc != 0 || list.is_null() {
            WlanCloseHandle(handle, std::ptr::null_mut());
            return Err(format!("WlanEnumInterfaces lỗi {}", rc));
        }

        let count = (*list).dwNumberOfItems as usize;
        let base = (*list).InterfaceInfo.as_ptr();
        let mut out: Vec<WlanIface> = Vec::with_capacity(count);
        for i in 0..count {
            let item = &*base.add(i);
            let connected = item.isState == wlan_interface_state_connected;
            let mut autoconf = None;
            if connected {
                let mut size: u32 = 0;
                let mut data: *mut c_void = std::ptr::null_mut();
                let mut value_type = 0;
                let rc = WlanQueryInterface(
                    handle,
                    &item.InterfaceGuid,
                    wlan_intf_opcode_autoconf_enabled,
                    std::ptr::null(),
                    &mut size,
                    &mut data,
                    &mut value_type,
                );
                if rc == 0 && !data.is_null() {
                    autoconf = Some(*(data as *const i32) != 0);
                    WlanFreeMemory(data as *const c_void);
                }
            }
            out.push(WlanIface {
                guid: guid_to_string(&item.InterfaceGuid),
                description: utf16_trim(&item.strInterfaceDescription),
                state: item.isState as u32,
                connected,
                autoconf,
            });
        }

        WlanFreeMemory(list as *const c_void);
        WlanCloseHandle(handle, std::ptr::null_mut());
        Ok(out)
    }
}

/// Bật/tắt quét nền Wi-Fi (autoconf) cho một interface theo GUID.
pub fn set_wlan_autoconf(guid: &str, enabled: bool) -> Result<(), String> {
    let g = guid_from_string(guid).ok_or_else(|| format!("GUID interface không hợp lệ: {}", guid))?;
    unsafe {
        let handle = wlan_open()?;
        let value: i32 = if enabled { 1 } else { 0 };
        let rc = WlanSetInterface(
            handle,
            &g,
            wlan_intf_opcode_autoconf_enabled,
            size_of::<i32>() as u32,
            &value as *const i32 as *const c_void,
            std::ptr::null(),
        );
        WlanCloseHandle(handle, std::ptr::null_mut());
        if rc != 0 {
            Err(format!("WlanSetInterface lỗi {}", rc))
        } else {
            Ok(())
        }
    }
}

const GAMEDVR_KEY: &str = r"System\GameConfigStore";
const APPCAPTURE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\GameDVR";
const GPU_PREFS_KEY: &str = r"Software\Microsoft\DirectX\UserGpuPreferences";

fn read_dword(path: &str, name: &str) -> Option<u32> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(path, KEY_READ)
        .ok()?
        .get_value(name)
        .ok()
}

/// Ghi DWORD; `None` = xoá value (dùng khi trước đó value không tồn tại).
fn write_dword(path: &str, name: &str, value: Option<u32>) -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(path, KEY_READ | KEY_WRITE)
        .map_err(|e| format!("mở khoá {} lỗi: {}", path, e))?;
    match value {
        Some(v) => key
            .set_value(name, &v)
            .map_err(|e| format!("ghi {} lỗi: {}", name, e)),
        // Value không tồn tại sẵn thì delete_value trả lỗi NotFound — coi như xong.
        None => {
            let _ = key.delete_value(name);
            Ok(())
        }
    }
}

pub fn gamedvr_enabled() -> Option<u32> {
    read_dword(GAMEDVR_KEY, "GameDVR_Enabled")
}

pub fn set_gamedvr_enabled(value: Option<u32>) -> Result<(), String> {
    write_dword(GAMEDVR_KEY, "GameDVR_Enabled", value)
}

pub fn appcapture_enabled() -> Option<u32> {
    read_dword(APPCAPTURE_KEY, "AppCaptureEnabled")
}

pub fn set_appcapture_enabled(value: Option<u32>) -> Result<(), String> {
    write_dword(APPCAPTURE_KEY, "AppCaptureEnabled", value)
}

/// Một value trong UserGpuPreferences (tên = đường dẫn exe).
pub struct GpuPref {
    pub path: String,
    pub value: String,
}

/// Toàn bộ value hiện có của HKCU\Software\Microsoft\DirectX\UserGpuPreferences.
pub fn gpu_prefs() -> Vec<GpuPref> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    let mut out: Vec<GpuPref> = Vec::new();
    let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(GPU_PREFS_KEY, KEY_READ)
    else {
        return out;
    };
    for (name, _) in key.enum_values().flatten() {
        if let Ok(value) = key.get_value::<String, _>(&name) {
            out.push(GpuPref { path: name, value });
        }
    }
    out
}

/// Đặt GPU hiệu năng cao (GPU rời) cho exe: `GpuPreference=2;` trong UserGpuPreferences.
/// Trả `Ok(true)` nếu vừa ghi mới, `Ok(false)` nếu giá trị đã đúng từ trước.
pub fn ensure_gpu_high_perf(exe_path: &str) -> Result<bool, String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    // Khoá có thể chưa tồn tại trên máy chưa từng chỉnh GPU theo app -> tạo mới.
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(GPU_PREFS_KEY)
        .map_err(|e| format!("mở khoá UserGpuPreferences lỗi: {}", e))?;
    // Giá trị dạng "AppStatus=0;GpuPreference=1;": chỉ đổi GpuPreference, giữ các mục khác.
    let current = key.get_value::<String, _>(exe_path).unwrap_or_default();
    let mut parts: Vec<String> = current
        .split(';')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty() && !p.to_lowercase().starts_with("gpupreference="))
        .map(|p| p.to_string())
        .collect();
    parts.push("GpuPreference=2".to_string());
    let wanted = format!("{};", parts.join(";"));
    if current.trim() == wanted {
        return Ok(false);
    }
    key.set_value(exe_path, &wanted)
        .map_err(|e| format!("ghi UserGpuPreferences lỗi: {}", e))?;
    Ok(true)
}

const TASK_NAME: &str = "GameMode";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Chạy chương trình ẩn cửa sổ console, gom stdout+stderr để báo lỗi cho người dùng.
fn run_hidden(program: &str, args: &[&str]) -> Result<String, String> {
    use std::os::windows::process::CommandExt;

    let out = std::process::Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("chạy {} lỗi: {}", program, e))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        Ok(text)
    } else {
        let msg = text.trim().to_string();
        Err(if msg.is_empty() {
            format!("{} trả mã {}", program, out.status)
        } else {
            msg
        })
    }
}

/// Task "GameMode" trong Task Scheduler đã tồn tại chưa (kiểm tra bằng exit code).
pub fn task_exists() -> bool {
    run_hidden("schtasks", &["/Query", "/TN", TASK_NAME]).is_ok()
}

/// Tạo task chạy cùng Windows với quyền cao nhất (không bật UAC mỗi lần đăng nhập).
pub fn task_create() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("không lấy được đường dẫn exe: {}", e))?;
    let tr = format!("\"{}\" --minimized", exe.display());
    run_hidden(
        "schtasks",
        &[
            "/Create",
            "/TN",
            TASK_NAME,
            "/TR",
            &tr,
            "/SC",
            "ONLOGON",
            "/RL",
            "HIGHEST",
            "/F",
        ],
    )
    .map(|_| ())
}

/// Xoá task "GameMode".
pub fn task_delete() -> Result<(), String> {
    run_hidden("schtasks", &["/Delete", "/TN", TASK_NAME, "/F"]).map(|_| ())
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

/// Commit charge của Windows: `(đang dùng, giới hạn)` theo byte.
/// Lấy từ `GlobalMemoryStatusEx`: giới hạn = `ullTotalPageFile`,
/// đang dùng = `ullTotalPageFile - ullAvailPageFile`.
pub fn commit() -> (u64, u64) {
    unsafe {
        let mut m: MEMORYSTATUSEX = zeroed();
        m.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut m) == 0 {
            return (0, 0);
        }
        (m.ullTotalPageFile - m.ullAvailPageFile, m.ullTotalPageFile)
    }
}

/// Đọc `GetSystemTimes` thành mili-giây (đơn vị FILETIME là 100 ns).
fn system_times() -> Option<(u64, u64, u64)> {
    unsafe {
        let mut idle: FILETIME = zeroed();
        let mut kernel: FILETIME = zeroed();
        let mut user: FILETIME = zeroed();
        if GetSystemTimes(&mut idle, &mut kernel, &mut user) == 0 {
            return None;
        }
        let f = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
        Some((f(idle), f(kernel), f(user)))
    }
}

/// CPU toàn hệ thống theo delta giữa hai lần đọc `GetSystemTimes`.
/// `kernel` đã bao gồm thời gian idle nên tổng = kernel + user.
pub struct CpuMeter {
    last: Option<(u64, u64, u64)>,
}

impl Default for CpuMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuMeter {
    pub fn new() -> Self {
        Self { last: None }
    }

    /// % CPU toàn hệ thống kể từ lần gọi trước. Lần gọi đầu trả 0.0
    /// (chưa có mốc so sánh); không đọc được thì cũng trả 0.0.
    pub fn pct(&mut self) -> f32 {
        let Some((idle, kernel, user)) = system_times() else {
            return 0.0;
        };
        let prev = self.last.replace((idle, kernel, user));
        let Some((p_idle, p_kernel, p_user)) = prev else {
            return 0.0;
        };
        let d_idle = idle.saturating_sub(p_idle);
        let total = kernel.saturating_sub(p_kernel) + user.saturating_sub(p_user);
        if total == 0 {
            return 0.0;
        }
        ((total.saturating_sub(d_idle)) as f64 * 100.0 / total as f64) as f32
    }
}

/// 4 octet IPv4 -> u32 đúng như WinAPI mong đợi: byte mạng nằm trong bộ nhớ
/// (giống `in_addr` của `inet_addr`), KHÔNG phải giá trị big-endian.
fn ip_u32(octets: [u8; 4]) -> u32 {
    u32::from_ne_bytes(octets)
}

/// u32 của WinAPI -> chuỗi "a.b.c.d".
pub fn ip_to_string(addr: u32) -> String {
    let b = addr.to_ne_bytes();
    format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
}

/// Phân giải "a.b.c.d" hoặc tên miền thành địa chỉ cho `ping`.
/// Trả `(địa chỉ, tên hiển thị)`.
pub fn resolve_ipv4(host: &str) -> Option<(u32, String)> {
    let host = host.trim();
    if host.is_empty() {
        return None;
    }
    if let Some(octets) = parse_ipv4(host) {
        let a = ip_u32(octets);
        return Some((a, ip_to_string(a)));
    }
    use std::net::ToSocketAddrs;
    let octets = (host, 0u16)
        .to_socket_addrs()
        .ok()?
        .find_map(|s| match s.ip() {
            std::net::IpAddr::V4(v4) => Some(v4.octets()),
            std::net::IpAddr::V6(_) => None,
        })?;
    let a = ip_u32(octets);
    Some((a, format!("{} ({})", ip_to_string(a), host)))
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = s.split('.');
    for slot in out.iter_mut() {
        *slot = parts.next()?.parse::<u8>().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}

/// Gateway mặc định: route tốt nhất tới 1.1.1.1 (`GetBestRoute`).
/// `None` khi không có route hoặc next hop = 0.0.0.0 (chưa có mạng).
pub fn gateway_ip() -> Option<u32> {
    unsafe {
        let mut row: MIB_IPFORWARDROW = zeroed();
        let rc = GetBestRoute(ip_u32([1, 1, 1, 1]), 0, &mut row);
        if rc != 0 || row.dwForwardNextHop == 0 {
            return None;
        }
        Some(row.dwForwardNextHop)
    }
}

/// Một gói ICMP echo tới địa chỉ IPv4 của WinAPI. Trả RTT (ms) khi có hồi âm,
/// `None` khi mất gói / hết thời gian chờ / không tạo được handle ICMP.
pub fn ping(addr: u32, timeout_ms: u32) -> Option<f32> {
    unsafe {
        let handle = IcmpCreateFile();
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let payload = [0u8; 32];
        // Vùng đệm trả về phải chứa ICMP_ECHO_REPLY + dữ liệu gửi + phần dư.
        // Cấp bằng u64 để chắc chắn căn 8 byte: ICMP_ECHO_REPLY có con trỏ bên trong.
        let need = size_of::<ICMP_ECHO_REPLY>() + payload.len() + 32;
        let mut reply = vec![0u64; need.div_ceil(8)];
        let n = IcmpSendEcho(
            handle,
            addr,
            payload.as_ptr() as *const c_void,
            payload.len() as u16,
            std::ptr::null(),
            reply.as_mut_ptr() as *mut c_void,
            (reply.len() * 8) as u32,
            timeout_ms,
        );
        IcmpCloseHandle(handle);
        if n == 0 {
            return None;
        }
        let r = &*(reply.as_ptr() as *const ICMP_ECHO_REPLY);
        // Status 0 = IP_SUCCESS; các mã khác (TTL hết hạn, unreachable...) coi như mất.
        if r.Status != 0 {
            return None;
        }
        Some(r.RoundTripTime as f32)
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

// ==========================================================================
// Cửa sổ chính GameMode: ẩn xuống khay / hiện lại / chặn mở nhiều bản
// ==========================================================================
//
// HWND được truyền dạng `isize` (không phải `HWND` = con trỏ) để gửi được sang
// thread xử lý sự kiện khay: con trỏ thô không `Send`, còn `isize` thì có.

/// Tìm cửa sổ theo đúng tiêu đề (dùng cho bản chạy thứ hai).
pub fn find_window(title: &str) -> isize {
    let t = to_wide(title);
    unsafe { FindWindowW(std::ptr::null(), t.as_ptr()) as isize }
}

/// Ẩn cửa sổ chính (nút X / đóng cửa sổ -> thu xuống khay).
///
/// Không để cửa sổ ở trạng thái SW_HIDE: cửa sổ ẩn không nhận WM_PAINT nên event loop của
/// eframe 0.29 chờ lần vẽ đó mãi và quay 100% một nhân CPU. Thay vào đó thu nhỏ cửa sổ và
/// gắn WS_EX_TOOLWINDOW để bỏ nút taskbar / Alt+Tab; cửa sổ thu nhỏ vẫn vẽ được nên loop ngủ.
pub fn hide_main_window(hwnd: isize) {
    if hwnd == 0 {
        return;
    }
    unsafe {
        let h = hwnd as HWND;
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let tool = WS_EX_TOOLWINDOW as isize;
        if ex & tool != 0 && IsIconic(h) != 0 {
            return; // đã ở khay
        }
        // Đổi ex-style lúc cửa sổ đang ẩn thì taskbar mới gỡ nút.
        ShowWindow(h, SW_HIDE);
        SetWindowLongPtrW(h, GWL_EXSTYLE, (ex | tool) & !(WS_EX_APPWINDOW as isize));
        ShowWindow(h, SW_SHOWMINNOACTIVE);
    }
}

/// Hiện lại cửa sổ chính và đưa lên trước (click trái icon khay, hoặc bản chạy thứ hai).
pub fn show_main_window(hwnd: isize) {
    if hwnd == 0 {
        return;
    }
    unsafe {
        let h = hwnd as HWND;
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let tool = WS_EX_TOOLWINDOW as isize;
        if ex & tool != 0 {
            // Từ khay ra: trả lại nút taskbar (đổi style lúc ẩn như `hide_main_window`).
            ShowWindow(h, SW_HIDE);
            SetWindowLongPtrW(h, GWL_EXSTYLE, (ex & !tool) | WS_EX_APPWINDOW as isize);
        }
        if IsIconic(h) != 0 {
            ShowWindow(h, SW_RESTORE);
        } else {
            ShowWindow(h, SW_SHOW);
        }
        SetForegroundWindow(h);
    }
}

/// Gửi WM_CLOSE vào cửa sổ chính (đánh thức main thread kể cả khi cửa sổ đang ẩn).
pub fn post_close(hwnd: isize) {
    if hwnd == 0 {
        return;
    }
    unsafe {
        PostMessageW(hwnd as HWND, WM_CLOSE, 0, 0);
    }
}

/// Giữ named mutex cho tới khi tiến trình thoát.
/// `Some(handle)` = bản đầu tiên (handle phải còn sống); `None` = đã có bản khác đang chạy.
pub fn acquire_single_instance(name: &str) -> Option<HANDLE> {
    let w = to_wide(name);
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, w.as_ptr());
        if h.is_null() {
            // Không tạo được mutex (hiếm): coi như chưa có bản nào để app vẫn mở được.
            return Some(std::ptr::null_mut());
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(h);
            None
        } else {
            Some(h)
        }
    }
}
