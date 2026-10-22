//! App khởi động cùng Windows: liệt kê và bật/tắt giống Task Manager.
//!
//! Cách làm **giống Task Manager**: không xoá mục nào, chỉ ghi cờ trong
//! `...\CurrentVersion\Explorer\StartupApproved\{Run, Run32, StartupFolder}` (REG_BINARY 12 byte,
//! byte đầu chẵn = bật, lẻ = tắt) nên bật lại được bất cứ lúc nào.
//!
//! Nguồn đọc: `Run` của user, `Run` của máy, `Run` của WOW6432Node, thư mục Startup của user
//! và thư mục Startup chung. Nguồn nào lỗi thì bỏ qua nguồn đó, không panic.
//!
//! Ghi chú: file `.lnk` trong thư mục Startup được đọc đích thật bằng `IShellLinkW` +
//! `IPersistFile` (feature `Win32_UI_Shell` + `Win32_System_Com`); lỗi thì giữ cách cũ
//! (`command` = đường dẫn file `.lnk`, `exe_path` = `None`).
//!
//! Phần thuần (`exe_from_command`, `parse_approved`, `approved_bytes`) chạy được cả trên Linux
//! nên có unit test; phần gọi WinAPI nằm sau `#[cfg(windows)]`.

use std::time::{SystemTime, UNIX_EPOCH};

/// Con trỏ `void*` của vtable COM (chỉ dùng ở nhánh Windows).
#[cfg(windows)]
use std::ffi::c_void;

/// Nơi một mục khởi động được đăng ký.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StartupLocation {
    /// `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
    RunUser,
    /// `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run`
    RunMachine,
    /// `HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run`
    RunMachine32,
    /// `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup`
    FolderUser,
    /// `%PROGRAMDATA%\Microsoft\Windows\Start Menu\Programs\StartUp`
    FolderCommon,
}

impl StartupLocation {
    /// Nhãn ngắn tiếng Việt để hiện trên giao diện.
    pub fn label(self) -> &'static str {
        match self {
            StartupLocation::RunUser => "Registry (user)",
            StartupLocation::RunMachine => "Registry (máy)",
            StartupLocation::RunMachine32 => "Registry (32-bit)",
            StartupLocation::FolderUser => "Thư mục Startup",
            StartupLocation::FolderCommon => "Thư mục Startup (chung)",
        }
    }

    /// Mục của mọi người dùng (HKLM) — sửa cần quyền admin.
    pub fn is_machine(self) -> bool {
        matches!(
            self,
            StartupLocation::RunMachine | StartupLocation::RunMachine32 | StartupLocation::FolderCommon
        )
    }

    /// Tên khoá con trong `Explorer\StartupApproved` giữ cờ bật/tắt.
    #[cfg(windows)]
    fn approved_subkey(self) -> &'static str {
        match self {
            StartupLocation::RunUser | StartupLocation::RunMachine => "Run",
            StartupLocation::RunMachine32 => "Run32",
            StartupLocation::FolderUser | StartupLocation::FolderCommon => "StartupFolder",
        }
    }
}

/// Một mục khởi động cùng Windows.
#[derive(Clone, Debug)]
pub struct StartupEntry {
    /// Tên value trong `Run`, hoặc tên file trong thư mục Startup (kèm đuôi).
    pub name: String,
    /// Dòng lệnh đầy đủ (với `.lnk` là đường dẫn file `.lnk`).
    pub command: String,
    /// Đường dẫn exe tách từ `command` (None nếu không tách được, vd file `.lnk`).
    pub exe_path: Option<String>,
    pub location: StartupLocation,
    pub enabled: bool,
}

#[cfg(windows)]
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";
#[cfg(windows)]
const RUN_USER: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_MACHINE: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_MACHINE32: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
/// Số 100 ns từ 1601-01-01 tới 1970-01-01 (chênh lệch FILETIME ↔ UNIX_EPOCH).
const FILETIME_EPOCH_DIFF: u64 = 11_644_473_600;

/// Đọc cờ bật/tắt từ giá trị REG_BINARY của `StartupApproved`.
///
/// Byte đầu: chẵn (0x02, 0x06, 0x00) = bật, lẻ (0x03, 0x07) = tắt.
/// Trả `None` khi giá trị rỗng/không đọc được (coi như bật nếu thiếu hẳn value).
pub fn parse_approved(bytes: &[u8]) -> Option<bool> {
    let first = *bytes.first()?;
    Some(first % 2 == 0)
}

/// 12 byte để ghi vào `StartupApproved`: bật `02 00…`, tắt `03 00 00 00` + FILETIME hiện tại.
pub fn approved_bytes(enabled: bool) -> [u8; 12] {
    let mut b = [0u8; 12];
    if enabled {
        b[0] = 0x02;
    } else {
        b[0] = 0x03;
        b[4..12].copy_from_slice(&now_filetime().to_le_bytes());
    }
    b
}

/// FILETIME hiện tại (100 ns kể từ 1601-01-01), tính thuần nên không cần WinAPI.
fn now_filetime() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() + FILETIME_EPOCH_DIFF) * 10_000_000 + u64::from(d.subsec_nanos() / 100),
        Err(_) => 0,
    }
}

/// Mở rộng biến môi trường dạng `%TÊN%`. Biến không có thì giữ nguyên `%TÊN%`
/// (một lượt, không đệ quy nên không thể lặp vô hạn).
fn expand_env(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) if !v.is_empty() => out.push_str(&v),
                    // Không có biến này (hoặc rỗng): giữ nguyên cả cụm %TÊN%.
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            // Dấu % lẻ: đẩy ra rồi đi tiếp.
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Tách đường dẫn exe từ một dòng lệnh (giá trị `Run` hoặc file trong thư mục Startup).
///
/// Quy tắc:
/// - Có ngoặc kép ở đầu: lấy đúng phần trong cặp ngoặc kép đầu tiên (`"C:\Program Files\Steam\steam.exe" -silent`).
/// - Không ngoặc kép: cắt tới `.exe` **đầu tiên** mà ngay sau nó là hết chuỗi hoặc khoảng trắng
///   (`C:\Program Files\App\app.exe --min` → `C:\Program Files\App\app.exe`). Cách này giữ được
///   đường dẫn có dấu cách mà không ngoặc kép; đổi lại nếu tham số cũng chứa `.exe` thì vẫn đúng
///   vì ta lấy mốc `.exe` đầu tiên.
/// - Không có `.exe` nào: lấy token đầu tiên (vd `rundll32 foo.dll` → `rundll32`).
/// - `%TÊN%` được mở rộng trước khi tách. Chuỗi rỗng/toàn khoảng trắng → `None`.
pub fn exe_from_command(cmd: &str) -> Option<String> {
    let cmd = expand_env(cmd.trim());
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return None;
    }

    if let Some(rest) = cmd.strip_prefix('"') {
        let end = rest.find('"').unwrap_or(rest.len());
        let path = rest[..end].trim();
        return (!path.is_empty()).then(|| path.to_string());
    }

    let lower = cmd.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;
    while let Some(pos) = lower[i..].find(".exe") {
        let end = i + pos + 4;
        let next = bytes.get(end).copied();
        if next.is_none() || next == Some(b' ') || next == Some(b'\t') {
            return Some(cmd[..end].trim().to_string());
        }
        i = end;
    }

    let tok = cmd.split_whitespace().next().unwrap_or("");
    let tok = tok.trim_matches('"');
    (!tok.is_empty()).then(|| tok.to_string())
}

/// Liệt kê mọi mục khởi động, sắp theo tên (không phân biệt hoa thường).
/// Nguồn nào lỗi thì bỏ qua nguồn đó.
#[cfg(windows)]
pub fn list_startup() -> Vec<StartupEntry> {
    let mut out = Vec::new();
    scan_run(RegRoot::User, RUN_USER, StartupLocation::RunUser, &mut out);
    scan_run(RegRoot::Machine, RUN_MACHINE, StartupLocation::RunMachine, &mut out);
    scan_run(
        RegRoot::Machine,
        RUN_MACHINE32,
        StartupLocation::RunMachine32,
        &mut out,
    );
    if let Some(dir) = startup_dir("APPDATA", "Startup") {
        scan_folder(&dir, StartupLocation::FolderUser, &mut out);
    }
    if let Some(dir) = startup_dir("PROGRAMDATA", "StartUp") {
        scan_folder(&dir, StartupLocation::FolderCommon, &mut out);
    }
    out.sort_by_key(|e| e.name.to_lowercase());
    out
}

/// Bật/tắt một mục bằng cách ghi cờ `StartupApproved` (không xoá mục).
/// Mục thuộc HKLM cần quyền admin — thiếu quyền thì trả lỗi tiếng Việt rõ ràng.
#[cfg(windows)]
pub fn set_enabled(entry: &StartupEntry, enabled: bool) -> Result<(), String> {
    use winreg::enums::{KEY_READ, KEY_WRITE, REG_BINARY};
    use winreg::RegValue;

    let root = match entry.location {
        StartupLocation::RunUser | StartupLocation::FolderUser => RegRoot::User,
        _ => RegRoot::Machine,
    };
    let path = format!(r"{}\{}", APPROVED, entry.location.approved_subkey());
    let (key, _) = root
        .key()
        .create_subkey_with_flags(&path, KEY_READ | KEY_WRITE)
        .map_err(|e| format!("Không mở/tạo được khoá HK{}\\{}: {}", root.tag(), path, e))?;
    let bytes = approved_bytes(enabled);
    let value = RegValue {
        vtype: REG_BINARY,
        bytes: bytes.to_vec(),
    };
    key.set_raw_value(&entry.name, &value)
        .map_err(|e| {
            let hint = if entry.location.is_machine() {
                " — mục của mọi người dùng, cần mở GameMode bằng quyền admin"
            } else {
                ""
            };
            format!("Không ghi được trạng thái khởi động cho \"{}\": {}{}", entry.name, e, hint)
        })?;
    Ok(())
}

#[cfg(windows)]
#[derive(Clone, Copy)]
enum RegRoot {
    User,
    Machine,
}

#[cfg(windows)]
impl RegRoot {
    /// Khoá gốc HKCU/HKLM đã mở sẵn.
    fn key(self) -> winreg::RegKey {
        winreg::RegKey::predef(match self {
            RegRoot::User => winreg::enums::HKEY_CURRENT_USER,
            RegRoot::Machine => winreg::enums::HKEY_LOCAL_MACHINE,
        })
    }

    fn tag(self) -> &'static str {
        match self {
            RegRoot::User => "CU",
            RegRoot::Machine => "LM",
        }
    }
}

/// Cờ bật/tắt của một mục; thiếu value (hoặc giá trị lạ) = đang bật.
#[cfg(windows)]
fn approved_state(root: RegRoot, subkey: &str, name: &str) -> bool {
    use winreg::enums::{KEY_READ, REG_BINARY};

    let path = format!(r"{}\{}", APPROVED, subkey);
    let key = match root.key().open_subkey_with_flags(&path, KEY_READ) {
        Ok(k) => k,
        Err(_) => return true,
    };
    match key.get_raw_value(name) {
        Ok(v) if v.vtype == REG_BINARY => parse_approved(&v.bytes).unwrap_or(true),
        _ => true,
    }
}

/// Đọc một khoá `Run`: tên value = tên mục, dữ liệu = dòng lệnh.
#[cfg(windows)]
fn scan_run(root: RegRoot, path: &str, location: StartupLocation, out: &mut Vec<StartupEntry>) {
    use winreg::enums::{KEY_READ, REG_EXPAND_SZ, REG_SZ};
    use winreg::types::FromRegValue;

    let key = match root.key().open_subkey_with_flags(path, KEY_READ) {
        Ok(k) => k,
        Err(_) => return,
    };
    for item in key.enum_values().flatten() {
        let (name, value) = item;
        // Chỉ nhận REG_SZ / REG_EXPAND_SZ; kiểu khác trong khoá Run là rác, bỏ qua.
        if value.vtype != REG_SZ && value.vtype != REG_EXPAND_SZ {
            continue;
        }
        let command = match String::from_reg_value(&value) {
            Ok(s) => s.trim().to_string(),
            Err(_) => continue,
        };
        let name = name.trim().to_string();
        if name.is_empty() || command.is_empty() {
            continue;
        }
        let enabled = approved_state(root, location.approved_subkey(), &name);
        out.push(StartupEntry {
            exe_path: exe_from_command(&command),
            name,
            command,
            location,
            enabled,
        });
    }
}

/// Đọc thư mục Startup; bỏ qua `desktop.ini` và thư mục con.
#[cfg(windows)]
fn scan_folder(dir: &std::path::Path, location: StartupLocation, out: &mut Vec<StartupEntry>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name.eq_ignore_ascii_case("desktop.ini") {
            continue;
        }
        let full = path.to_string_lossy().to_string();
        // File .lnk: lấy đích thật (exe + tham số) bằng IShellLinkW; lỗi thì giữ đường dẫn .lnk.
        let target = if is_lnk(&path) { lnk_target(&path) } else { None };
        let enabled = approved_state(
            match location {
                StartupLocation::FolderUser => RegRoot::User,
                _ => RegRoot::Machine,
            },
            location.approved_subkey(),
            &name,
        );
        match target {
            Some((command, exe)) => out.push(StartupEntry {
                exe_path: Some(exe),
                name,
                command,
                location,
                enabled,
            }),
            None => out.push(StartupEntry {
                exe_path: if is_lnk(&path) { None } else { Some(full.clone()) },
                name,
                command: full,
                location,
                enabled,
            }),
        }
    }
}

/// File có phải shortcut `.lnk` không (không phân biệt hoa thường).
#[cfg(windows)]
fn is_lnk(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
}

/// IID của `IShellLinkW` / `IPersistFile` (windows-sys 0.59 chỉ có GUID coclass `ShellLink`).
#[cfg(windows)]
const IID_ISHELLLINKW: windows_sys::core::GUID =
    windows_sys::core::GUID::from_u128(0x000214F9_0000_0000_C000_000000000046);
#[cfg(windows)]
const IID_IPERSISTFILE: windows_sys::core::GUID =
    windows_sys::core::GUID::from_u128(0x0000010B_0000_0000_C000_000000000046);

/// Ba hàm đầu của mọi vtable COM (`IUnknown`). Chỉ `query_interface`/`release` được gọi.
#[cfg(windows)]
#[allow(dead_code)]
#[repr(C)]
struct IUnknownVtbl {
    query_interface: unsafe extern "system" fn(
        *mut c_void,
        *const windows_sys::core::GUID,
        *mut *mut c_void,
    ) -> windows_sys::core::HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

/// Vtable `IPersistFile` (IUnknown + `GetClassID` + `IsDirty`/`Load`/`Save`/`SaveCompleted`/
/// `GetCurFile`; chỉ cần `Load` nhưng phải khai đủ thứ tự để con trỏ hàm đúng chỗ).
#[cfg(windows)]
#[allow(dead_code)]
#[repr(C)]
struct IPersistFileVtbl {
    base: IUnknownVtbl,
    get_class_id: unsafe extern "system" fn(*mut c_void, *mut windows_sys::core::GUID) -> windows_sys::core::HRESULT,
    is_dirty: unsafe extern "system" fn(*mut c_void) -> windows_sys::core::HRESULT,
    load: unsafe extern "system" fn(
        *mut c_void,
        *const u16,
        u32,
    ) -> windows_sys::core::HRESULT,
    save: *const c_void,
    save_completed: *const c_void,
    get_cur_file: *const c_void,
}

/// Vtable `IShellLinkW`: `IUnknown` + các phương thức theo đúng thứ tự trong `shobjidl_core.h`
/// (chỉ `GetPath`/`GetArguments` được gọi, phần còn lại là chỗ giữ đúng vị trí).
#[cfg(windows)]
#[allow(dead_code)]
#[repr(C)]
struct IShellLinkWVtbl {
    base: IUnknownVtbl,
    get_arguments: unsafe extern "system" fn(*mut c_void, *mut u16, i32) -> windows_sys::core::HRESULT,
    get_description: *const c_void,
    get_working_directory: *const c_void,
    get_hotkey: *const c_void,
    set_arguments: *const c_void,
    set_description: *const c_void,
    set_working_directory: *const c_void,
    set_hotkey: *const c_void,
    set_show_cmd: *const c_void,
    get_show_cmd: *const c_void,
    get_icon_location: *const c_void,
    set_icon_location: *const c_void,
    set_relative_path: *const c_void,
    resolve: *const c_void,
    set_path: *const c_void,
    get_path: unsafe extern "system" fn(
        *mut c_void,
        *mut u16,
        i32,
        *mut c_void,
        u32,
    ) -> windows_sys::core::HRESULT,
    set_id_list: *const c_void,
    get_id_list: *const c_void,
    set_name: *const c_void,
    get_name: *const c_void,
}

/// Đích thật của một file `.lnk`: `(dòng lệnh, đường dẫn exe)`. `None` khi không đọc được.
///
/// `CoInitializeEx` gọi trên **thread đang chạy** (bắt buộc trước khi `CoCreateInstance`) rồi
/// `CoUninitialize` ngay sau khi xong. Thread đã vào COM ở apartment khác
/// (`RPC_E_CHANGED_MODE`) thì vẫn dùng được COM đã có, chỉ KHÔNG gọi `CoUninitialize`
/// (không phải ta khởi tạo).
#[cfg(windows)]
fn lnk_target(path: &std::path::Path) -> Option<(String, String)> {
    use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};

    let hr_init = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    if hr_init < 0 {
        // RPC_E_CHANGED_MODE: thread này đã ở apartment khác — COM vẫn dùng được, nhưng KHÔNG
        // phải ta khởi tạo nên không được gọi CoUninitialize.
        return lnk_read(path);
    }
    let out = lnk_read(path);
    unsafe { CoUninitialize() };
    out
}

/// Phần gọi COM thật của `lnk_target` (đã ở trong COM apartment).
#[cfg(windows)]
fn lnk_read(path: &std::path::Path) -> Option<(String, String)> {
    use windows_sys::Win32::Foundation::S_OK;
    use windows_sys::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, STGM_READ};
    use windows_sys::Win32::UI::Shell::ShellLink;

    let mut link: *mut c_void = std::ptr::null_mut();
    let hr = unsafe {
        CoCreateInstance(
            &ShellLink,
            std::ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_ISHELLLINKW,
            &mut link,
        )
    };
    if hr != S_OK || link.is_null() {
        return None;
    }
    // `link` là con trỏ tới con trỏ vtable: byte đầu là vtable của IShellLinkW.
    let vt = unsafe { *(link as *const *const IShellLinkWVtbl) };
    if vt.is_null() {
        // Không có vtable thì không gọi được gì (kể cả Release) — bỏ qua, coi như thất bại.
        return None;
    }

    // IPersistFile để nạp nội dung file .lnk.
    let mut persist: *mut c_void = std::ptr::null_mut();
    let hr = unsafe { ((*vt).base.query_interface)(link, &IID_IPERSISTFILE, &mut persist) };
    if hr != S_OK || persist.is_null() {
        unsafe { ((*vt).base.release)(link) };
        return None;
    }
    let pvt = unsafe { *(persist as *const *const IPersistFileVtbl) };
    let mut wide: Vec<u16> = path.to_string_lossy().encode_utf16().collect();
    wide.push(0);
    let hr = unsafe { ((*pvt).load)(persist, wide.as_ptr(), STGM_READ) };
    let loaded = hr == S_OK;

    let mut target = String::new();
    let mut args = String::new();
    if loaded {
        let mut buf = vec![0u16; 1024];
        let hr = unsafe { ((*vt).get_path)(link, buf.as_mut_ptr(), buf.len() as i32, std::ptr::null_mut(), 0) };
        if hr == S_OK {
            target = utf16_buf_to_string(&buf);
        }
        let mut abuf = vec![0u16; 1024];
        let hr = unsafe { ((*vt).get_arguments)(link, abuf.as_mut_ptr(), abuf.len() as i32) };
        if hr == S_OK {
            args = utf16_buf_to_string(&abuf);
        }
    }

    unsafe {
        ((*pvt).base.release)(persist);
        ((*vt).base.release)(link);
    }

    if target.is_empty() {
        return None;
    }
    // Dòng lệnh giống định dạng của khoá `Run`: đường dẫn có ngoặc kép nếu chứa dấu cách.
    let command = if args.is_empty() {
        format!("\"{}\"", target)
    } else {
        format!("\"{}\" {}", target, args)
    };
    Some((command, target))
}

/// Chuỗi từ buffer UTF-16 đã được WinAPI ghi (dừng ở ký tự 0).
#[cfg(windows)]
fn utf16_buf_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// `%<var>%\Microsoft\Windows\Start Menu\Programs\<leaf>` nếu biến môi trường có và thư mục tồn tại.
#[cfg(windows)]
fn startup_dir(var: &str, leaf: &str) -> Option<std::path::PathBuf> {
    let base = std::env::var(var).ok()?;
    if base.is_empty() {
        return None;
    }
    let dir = std::path::Path::new(&base)
        .join(r"Microsoft\Windows\Start Menu\Programs")
        .join(leaf);
    dir.is_dir().then_some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_co_ngoac_kep_va_tham_so() {
        assert_eq!(
            exe_from_command(r#""C:\Program Files\Steam\steam.exe" -silent"#).as_deref(),
            Some(r"C:\Program Files\Steam\steam.exe")
        );
    }

    #[test]
    fn exe_khong_ngoac_kep_khong_dau_cach() {
        assert_eq!(
            exe_from_command(r"C:\Windows\system32\SecurityHealthSystray.exe").as_deref(),
            Some(r"C:\Windows\system32\SecurityHealthSystray.exe")
        );
    }

    #[test]
    fn exe_khong_ngoac_kep_co_dau_cach() {
        // Chọn cách xử lý: cắt tới `.exe` đầu tiên (không dừng ở dấu cách đầu).
        assert_eq!(
            exe_from_command(r"C:\Program Files\App\app.exe --min").as_deref(),
            Some(r"C:\Program Files\App\app.exe")
        );
    }

    #[test]
    fn exe_mo_rong_bien_moi_truong() {
        // Đặt biến để test chạy giống nhau trên Linux lẫn Windows.
        std::env::set_var("windir", r"C:\Windows");
        assert_eq!(
            exe_from_command(r"%windir%\system32\x.exe /y").as_deref(),
            Some(r"C:\Windows\system32\x.exe")
        );
        assert_eq!(
            exe_from_command(r#""%windir%\System32\cmd.exe" /k echo hi"#).as_deref(),
            Some(r"C:\Windows\System32\cmd.exe")
        );
    }

    #[test]
    fn exe_bien_moi_truong_khong_co_thi_giu_nguyen() {
        assert_eq!(
            exe_from_command(r"%GameMode_Khong_Co_Bien_Nay%\tool.exe").as_deref(),
            Some(r"%GameMode_Khong_Co_Bien_Nay%\tool.exe")
        );
    }

    #[test]
    fn exe_co_tham_so_chua_duong_dan() {
        assert_eq!(
            exe_from_command(r#"C:\Tools\node.exe --inspect "C:\a b\c.js""#).as_deref(),
            Some(r"C:\Tools\node.exe")
        );
    }

    #[test]
    fn exe_mot_minh_va_chuoi_rong() {
        assert_eq!(exe_from_command("oneshot.exe").as_deref(), Some("oneshot.exe"));
        assert_eq!(exe_from_command(r#""C:\app with space\run.bat""#).as_deref(), Some(r"C:\app with space\run.bat"));
        assert_eq!(exe_from_command("   "), None);
        assert_eq!(exe_from_command(""), None);
    }

    #[test]
    fn exe_khong_co_duoi_exe_thi_lay_token_dau() {
        assert_eq!(
            exe_from_command(r"rundll32.exe foo.dll,EntryPoint").as_deref(),
            Some("rundll32.exe")
        );
        assert_eq!(exe_from_command("pythonw script.py").as_deref(), Some("pythonw"));
    }

    #[test]
    fn parse_approved_doc_byte_dau() {
        assert_eq!(parse_approved(&[0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), Some(true));
        assert_eq!(parse_approved(&[0x06, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]), Some(true));
        assert_eq!(parse_approved(&[0x00; 12]), Some(true));
        assert_eq!(parse_approved(&[0x03, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]), Some(false));
        assert_eq!(parse_approved(&[0x07, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), Some(false));
        assert_eq!(parse_approved(&[0x01]), Some(false));
        assert_eq!(parse_approved(&[]), None);
    }

    #[test]
    fn approved_bytes_dung_khuon_va_doc_lai_duoc() {
        let on = approved_bytes(true);
        assert_eq!(on.len(), 12);
        assert_eq!(&on[..4], &[0x02, 0, 0, 0]);
        assert_eq!(parse_approved(&on), Some(true));

        let off = approved_bytes(false);
        assert_eq!(off.len(), 12);
        assert_eq!(&off[..4], &[0x03, 0, 0, 0]);
        assert_eq!(parse_approved(&off), Some(false));
        assert!(u64::from_le_bytes(off[4..12].try_into().unwrap()) > 0, "FILETIME phải > 0");
    }

    #[test]
    fn nhan_vi_tri_khong_rong() {
        for loc in [
            StartupLocation::RunUser,
            StartupLocation::RunMachine,
            StartupLocation::RunMachine32,
            StartupLocation::FolderUser,
            StartupLocation::FolderCommon,
        ] {
            assert!(!loc.label().is_empty());
        }
        assert!(StartupLocation::RunMachine32.is_machine());
        assert!(!StartupLocation::FolderUser.is_machine());
    }
}
