//! Tinh chỉnh Windows lâu dài cho Paroda GameMode (tab "Tối ưu Win").
//!
//! Khác tab Game Boost (chỉ đổi trong lúc chơi rồi Restore trả lại), các tinh chỉnh ở đây
//! đổi Windows **lâu dài** theo lựa chọn của người dùng. Vì vậy yêu cầu bắt buộc là:
//! **mọi tinh chỉnh hoàn tác được về đúng giá trị trước đó** — kể cả trường hợp giá trị
//! hoặc cả khoá registry chưa từng tồn tại (lúc đó hoàn tác = xoá giá trị đi).
//!
//! Module tự đứng được (không `use crate::…`): chỗ cần chạy chương trình ngoài thì tự có
//! bản `run_hidden` nhỏ (CREATE_NO_WINDOW) và tự giải mã output UTF-16 LE/UTF-8 giống
//! `sys.rs`. Mọi thứ đụng registry / tiến trình / WinAPI nằm sau `#[cfg(windows)]`;
//! phần thuần (parse output, so sánh trạng thái, sao lưu JSON) có unit test chạy cả trên Linux.
//!
//! `main.rs` chỉ cần: `all_tweaks(&game_dirs)` để dựng danh sách, `state()` để vẽ trạng thái,
//! `apply()` / `revert()` để đổi và trả lại, `Backup::load()` / `save()` để giữ file JSON
//! (`wintweaks-backup.json`) cạnh exe và `summary()` cho dòng đếm trên tab.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Tên file sao lưu (main ghép với thư mục cạnh exe rồi truyền đường dẫn đầy đủ vào).
pub const FILE_NAME: &str = "wintweaks-backup.json";

/// Phiên bản định dạng file sao lưu.
pub const BACKUP_VERSION: u32 = 1;

/// Giá trị mặc định cho `Backup::version` khi file JSON thiếu trường này.
fn default_version() -> u32 {
    BACKUP_VERSION
}

/// Thông báo dùng chung khi gọi phần chỉ có trên Windows.
#[cfg(not(windows))]
const NOT_WINDOWS: &str = "Tinh chỉnh Windows chỉ chạy được trên Windows";

// =====================================================================================
// Kiểu dữ liệu
// =====================================================================================

/// Mức rủi ro khi bật một tinh chỉnh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    /// Đổi qua lại vô hại, gần như không có tác dụng phụ.
    Safe,
    /// Có tác dụng phụ đã biết (tốn pin, chậm tìm kiếm, đổi cảm giác chuột…).
    Consider,
    /// Có thể làm hỏng tính năng khác của Windows cho tới khi hoàn tác.
    Advanced,
}

impl Risk {
    pub fn label(self) -> &'static str {
        match self {
            Risk::Safe => "An toàn",
            Risk::Consider => "Cân nhắc",
            Risk::Advanced => "Nâng cao",
        }
    }
}

/// Nhóm tinh chỉnh (mỗi nhóm là một khối trên tab "Tối ưu Win").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Game,
    Background,
    Memory,
    System,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::Game => "Hiệu năng game",
            Group::Background => "Giảm tác vụ nền",
            Group::Memory => "Bộ nhớ",
            Group::System => "Hệ thống",
        }
    }
}

/// Registry hive mà module này đụng tới (đủ dùng, không ôm cả `winreg::HKEY` vào API).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Hive {
    Hkcu,
    Hklm,
}

impl Hive {
    pub fn label(self) -> &'static str {
        match self {
            Hive::Hkcu => "HKCU",
            Hive::Hklm => "HKLM",
        }
    }
}

/// Kiểu khởi động của service, đúng theo giá trị `Start` trong registry (2/3/4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartType {
    Auto,
    Manual,
    Disabled,
}

impl StartType {
    /// Giá trị `Start` ghi vào registry (KHÔNG dừng/khởi động service).
    pub fn value(self) -> u32 {
        match self {
            StartType::Auto => 2,
            StartType::Manual => 3,
            StartType::Disabled => 4,
        }
    }

    pub fn from_value(v: u32) -> Option<StartType> {
        match v {
            2 => Some(StartType::Auto),
            3 => Some(StartType::Manual),
            4 => Some(StartType::Disabled),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            StartType::Auto => "Tự động",
            StartType::Manual => "Thủ công",
            StartType::Disabled => "Tắt",
        }
    }
}

/// Một thao tác cụ thể của tinh chỉnh. Mỗi tinh chỉnh là một danh sách thao tác.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Ghi DWORD vào registry (tạo khoá nếu chưa có).
    RegDword {
        hive: Hive,
        path: &'static str,
        name: &'static str,
        value: u32,
    },
    /// Ghi REG_SZ vào registry (tạo khoá nếu chưa có).
    RegString {
        hive: Hive,
        path: &'static str,
        name: &'static str,
        value: &'static str,
    },
    /// Ghi REG_MULTI_SZ vào registry (tạo khoá nếu chưa có).
    RegMultiString {
        hive: Hive,
        path: &'static str,
        name: &'static str,
        value: Vec<String>,
    },
    /// Đổi Start type của service qua `HKLM\SYSTEM\CurrentControlSet\Services\<name>\Start`.
    /// Service không tồn tại thì bỏ qua (không lỗi). KHÔNG dừng/khởi động service.
    ServiceStart { name: &'static str, start: StartType },
    /// Đổi một thiết lập nguồn khi cắm sạc bằng `powercfg /setacvalueindex` rồi `/setactive`.
    PowerAc {
        subgroup_guid: &'static str,
        setting_guid: &'static str,
        value: u32,
    },
    /// `bcdedit /set {current} <name> <value>`.
    Bcd { name: &'static str, value: &'static str },
    /// Bật/tắt nén bộ nhớ (`Enable-MMAgent` / `Disable-MMAgent -MemoryCompression`).
    MemoryCompression(bool),
    /// Thêm thư mục loại trừ cho Defender (chỉ gỡ lại đúng thư mục module này đã thêm).
    DefenderExclusions(Vec<String>),
}

impl Action {
    /// Mô tả ngắn để hiện trong lỗi / nhật ký.
    pub fn target(&self) -> String {
        match self {
            Action::RegDword {
                hive, path, name, ..
            }
            | Action::RegString {
                hive, path, name, ..
            }
            | Action::RegMultiString {
                hive, path, name, ..
            } => format!("{}\\{}\\{}", hive.label(), path, name),
            Action::ServiceStart { name, .. } => format!("service {}", name),
            Action::PowerAc {
                subgroup_guid,
                setting_guid,
                ..
            } => format!("powercfg {} / {}", subgroup_guid, setting_guid),
            Action::Bcd { name, .. } => format!("bcdedit {}", name),
            Action::MemoryCompression(_) => "nén bộ nhớ (MMAgent)".to_string(),
            Action::DefenderExclusions(v) => format!("Defender loại trừ {} thư mục", v.len()),
        }
    }
}

/// Một tinh chỉnh Windows lâu dài.
#[derive(Debug, Clone)]
pub struct Tweak {
    pub id: &'static str,
    pub group: Group,
    pub risk: Risk,
    pub title: &'static str,
    pub desc: &'static str,
    pub needs_reboot: bool,
    pub recommended: bool,
    actions: Vec<Action>,
}

impl Tweak {
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    pub fn action_count(&self) -> usize {
        self.actions.len()
    }

    /// Dòng phụ trên thẻ: nhóm · mức rủi ro · cần khởi động lại.
    pub fn subtitle(&self) -> String {
        let mut s = format!("{} · {}", self.group.label(), self.risk.label());
        if self.needs_reboot {
            s.push_str(" · cần khởi động lại");
        }
        s
    }
}

/// Trạng thái hiện tại của một tinh chỉnh trên máy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TweakState {
    /// Mọi thao tác đang đúng giá trị đích.
    Applied,
    /// Chưa thao tác nào đúng giá trị đích.
    NotApplied,
    /// Một phần đúng (hoặc phần đọc được đã đúng, phần còn lại chưa rõ).
    Partial,
    /// Không đọc được trạng thái (kèm lý do tiếng Việt).
    Unknown(String),
}

impl TweakState {
    pub fn label(&self) -> String {
        match self {
            TweakState::Applied => "Đã bật".to_string(),
            TweakState::NotApplied => "Chưa bật".to_string(),
            TweakState::Partial => "Một phần".to_string(),
            TweakState::Unknown(why) => format!("Không rõ: {}", why),
        }
    }

    pub fn is_applied(&self) -> bool {
        matches!(self, TweakState::Applied)
    }
}

/// Giá trị đọc được từ máy cho một thao tác.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadValue {
    /// Giá trị (hoặc khoá / service) chưa từng tồn tại.
    Missing,
    /// Không đọc được giá trị (ví dụ `powercfg /q` không in chỉ số AC).
    Unknown,
    Dword(u32),
    Text(String),
    Multi(Vec<String>),
    Flag(bool),
}

/// Kết quả so sánh một thao tác với giá trị đích.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionState {
    /// Đang đúng giá trị đích.
    Matched,
    /// Đang khác giá trị đích.
    NotMatched,
    /// Không áp dụng được trên máy này (ví dụ service không tồn tại) — bỏ qua, không lỗi.
    Skipped,
    /// Không đọc được nên không kết luận.
    Unknown,
}

// =====================================================================================
// Phần thuần (chạy được cả trên Linux — có unit test ở cuối file)
// =====================================================================================

fn cmp_eq(ok: bool) -> ActionState {
    if ok {
        ActionState::Matched
    } else {
        ActionState::NotMatched
    }
}

/// So sánh một thao tác với giá trị vừa đọc được.
pub fn action_state(action: &Action, current: &ReadValue) -> ActionState {
    match action {
        Action::RegDword { value, .. } => match current {
            ReadValue::Dword(v) => cmp_eq(v == value),
            ReadValue::Missing => ActionState::NotMatched,
            _ => ActionState::Unknown,
        },
        Action::RegString { value, .. } => match current {
            ReadValue::Text(s) => cmp_eq(s.eq_ignore_ascii_case(value)),
            ReadValue::Missing => ActionState::NotMatched,
            _ => ActionState::Unknown,
        },
        Action::RegMultiString { value, .. } => match current {
            ReadValue::Multi(v) => cmp_eq(same_multi(v, value)),
            ReadValue::Missing => ActionState::NotMatched,
            _ => ActionState::Unknown,
        },
        Action::ServiceStart { start, .. } => match current {
            ReadValue::Dword(v) => cmp_eq(*v == start.value()),
            // Service không có trên máy này -> không có gì để đổi, coi như xong.
            ReadValue::Missing => ActionState::Skipped,
            _ => ActionState::Unknown,
        },
        Action::PowerAc { value, .. } => match current {
            ReadValue::Dword(v) => cmp_eq(v == value),
            ReadValue::Missing => ActionState::NotMatched,
            _ => ActionState::Unknown,
        },
        Action::Bcd { value, .. } => match current {
            ReadValue::Text(s) => cmp_eq(s.trim().eq_ignore_ascii_case(value.trim())),
            // Chưa có mục này trong BCD -> coi như chưa bật (hoàn tác sẽ là /deletevalue).
            ReadValue::Missing => ActionState::NotMatched,
            _ => ActionState::Unknown,
        },
        Action::MemoryCompression(on) => match current {
            ReadValue::Flag(b) => cmp_eq(b == on),
            _ => ActionState::Unknown,
        },
        Action::DefenderExclusions(dirs) => match current {
            ReadValue::Multi(v) => cmp_eq(dirs.iter().all(|d| contains_path(v, d))),
            ReadValue::Missing => ActionState::NotMatched,
            _ => ActionState::Unknown,
        },
    }
}

/// So sánh hai danh sách REG_MULTI_SZ, không phân biệt hoa thường.
fn same_multi(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.eq_ignore_ascii_case(y))
}

/// Danh sách loại trừ của Defender chứa đường dẫn này chưa (bỏ `\` cuối, không phân biệt hoa thường).
fn contains_path(list: &[String], dir: &str) -> bool {
    let want = dir.trim().trim_end_matches('\\');
    list.iter()
        .any(|x| x.trim().trim_end_matches('\\').eq_ignore_ascii_case(want))
}

/// Gộp trạng thái của mọi thao tác thành trạng thái của cả tinh chỉnh.
pub fn combine_states(states: &[ActionState]) -> TweakState {
    if states.is_empty() {
        return TweakState::Unknown("Tinh chỉnh không có thao tác nào".to_string());
    }
    let matched = states.iter().filter(|s| **s == ActionState::Matched).count();
    let not = states.iter().filter(|s| **s == ActionState::NotMatched).count();
    let unknown = states.iter().filter(|s| **s == ActionState::Unknown).count();
    if unknown > 0 && not == 0 && matched == 0 {
        return TweakState::Unknown("Không đọc được trạng thái của thao tác này".to_string());
    }
    if unknown > 0 {
        // Biết chắc có phần chưa đạt, phần còn lại chưa đọc được -> "một phần" là mô tả đúng nhất.
        return TweakState::Partial;
    }
    if not == 0 {
        if matched == 0 {
            // Mọi thao tác đều bị bỏ qua: máy này không có gì để đổi.
            return TweakState::Unknown("Không có thao tác nào áp dụng được trên máy này".to_string());
        }
        return TweakState::Applied;
    }
    if matched == 0 {
        TweakState::NotApplied
    } else {
        TweakState::Partial
    }
}

/// Đổi giá trị vừa đọc thành giá trị cần lưu vào bản sao lưu.
fn saved_from_read(action: &Action, v: ReadValue) -> SavedValue {
    match (action, v) {
        // Service không tồn tại: không ghi gì thì cũng không có gì để trả lại.
        (Action::ServiceStart { .. }, ReadValue::Missing) => SavedValue::Skipped,
        (_, ReadValue::Missing) => SavedValue::Missing,
        // Không đọc được thì không dám ghi (xem `apply`) — giữ nhánh này để file cũ vẫn đọc được.
        (_, ReadValue::Unknown) => SavedValue::Unreadable,
        (_, ReadValue::Dword(x)) => SavedValue::Dword(x),
        (_, ReadValue::Text(s)) => SavedValue::Text(s),
        (_, ReadValue::Multi(m)) => SavedValue::Multi(m),
        (_, ReadValue::Flag(b)) => SavedValue::Flag(b),
    }
}

/// Lấy chỉ số AC ("Current AC Power Setting Index") từ output `powercfg /q`.
///
/// Output có thể là tiếng Anh hoặc đã bản địa hoá (ví dụ "Chỉ số cài đặt nguồn AC hiện tại"),
/// nên nhận dòng theo hai dấu hiệu: có `0x` và có " AC" (dòng DC — nguồn pin — bị loại).
pub fn parse_powercfg_ac(out: &str) -> Option<u32> {
    for line in out.lines() {
        let upper = line.to_uppercase();
        if !upper.contains("0X") || !upper.contains(" AC") {
            continue;
        }
        let tail = match upper.rfind("0X") {
            Some(i) => &upper[i + 2..],
            None => continue,
        };
        let hex: String = tail.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        if hex.is_empty() {
            continue;
        }
        if let Ok(v) = u32::from_str_radix(&hex, 16) {
            return Some(v);
        }
    }
    None
}

/// Lấy giá trị của một mục trong output `bcdedit /enum {current}` (theo tên, không phân biệt hoa thường).
///
/// `None` nghĩa là mục đó không có trong BCD (hoàn tác sẽ dùng `bcdedit /deletevalue`).
pub fn parse_bcdedit_value(out: &str, name: &str) -> Option<String> {
    for line in out.lines() {
        let mut it = line.split_whitespace();
        let key = match it.next() {
            Some(k) => k,
            None => continue,
        };
        if !key.eq_ignore_ascii_case(name) {
            continue;
        }
        let rest: Vec<&str> = it.collect();
        if rest.is_empty() {
            return None;
        }
        return Some(rest.join(" "));
    }
    None
}

/// Giải mã output của chương trình Windows: UTF-16 LE (có BOM) hoặc UTF-16 LE không BOM
/// (nhận diện qua byte 0 xen kẽ — `bcdedit`, `powercfg` tuỳ phiên bản/locale), còn lại là UTF-8.
pub fn decode_win_output(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        return utf16le_to_string(&bytes[2..]);
    }
    let sample = &bytes[..bytes.len().min(512)];
    let half = sample.len() / 2;
    if half >= 4 {
        let zeros = sample.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
        if zeros * 4 >= half * 3 {
            return utf16le_to_string(bytes);
        }
    }
    String::from_utf8_lossy(bytes).to_string()
}

/// Giải mã UTF-16 LE (bỏ `\0` cuối) thành String.
fn utf16le_to_string(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
        .trim_end_matches('\0')
        .to_string()
}

/// Thời điểm hiện tại (giây kể từ 1970, UTC) — không panic, lỗi thì trả 0.
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// =====================================================================================
// Sao lưu
// =====================================================================================

/// Giá trị gốc đã lưu cho một thao tác (đúng thứ tự thao tác của tinh chỉnh).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum SavedValue {
    /// Giá trị (hoặc khoá/mục BCD) chưa từng tồn tại -> hoàn tác bằng cách xoá.
    Missing,
    /// Thao tác không áp dụng được (service không có trên máy) -> hoàn tác bỏ qua.
    Skipped,
    /// Không đọc được giá trị gốc -> module KHÔNG ghi (xem `apply`), giữ nhánh để file cũ vẫn đọc được.
    Unreadable,
    Dword(u32),
    Text(String),
    Multi(Vec<String>),
    Flag(bool),
}

/// Bản sao lưu giá trị gốc của một tinh chỉnh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupEntry {
    pub tweak_id: String,
    /// Giây kể từ 1970 (UTC) lúc sao lưu; 0 nếu không rõ.
    #[serde(default)]
    pub saved_at: u64,
    /// Giá trị gốc của từng thao tác, đúng thứ tự `Tweak::actions()`.
    pub originals: Vec<SavedValue>,
}

/// Toàn bộ bản sao lưu, lưu ra `wintweaks-backup.json` cạnh exe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backup {
    #[serde(default = "default_version")]
    pub version: u32,
    /// Giá trị gốc theo id tinh chỉnh (BTreeMap để file JSON ổn định thứ tự).
    #[serde(default)]
    pub entries: BTreeMap<String, BackupEntry>,
}

impl Default for Backup {
    fn default() -> Self {
        Backup {
            version: default_version(),
            entries: BTreeMap::new(),
        }
    }
}

impl Backup {
    pub fn new() -> Backup {
        Backup::default()
    }

    pub fn has(&self, tweak_id: &str) -> bool {
        self.entries.contains_key(tweak_id)
    }

    pub fn ids(&self) -> Vec<&str> {
        self.entries.keys().map(|s| s.as_str()).collect()
    }

    /// Xoá một mục khỏi bản sao lưu (API của module; tab "Tối ưu Win" hiện chỉ dùng `revert`,
    /// vốn tự xoá mục sau khi hoàn tác xong — giữ lại cho công cụ dòng lệnh sau này).
    #[allow(dead_code)]
    pub fn remove(&mut self, tweak_id: &str) -> bool {
        self.entries.remove(tweak_id).is_some()
    }

    /// Đọc file sao lưu. Không có file / file hỏng thì trả bản rỗng (không panic, không xoá file).
    pub fn load(path: &Path) -> Backup {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice::<Backup>(&bytes).unwrap_or_default(),
            Err(_) => Backup::default(),
        }
    }

    /// Ghi file sao lưu (tạo cả thư mục cha nếu thiếu).
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() && !dir.exists() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("tạo thư mục {} lỗi: {}", dir.display(), e))?;
            }
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| format!("đóng gói bản sao lưu lỗi: {}", e))?;
        std::fs::write(path, text).map_err(|e| format!("ghi {} lỗi: {}", path.display(), e))
    }
}

// =====================================================================================
// Đọc / ghi trên Windows
// =====================================================================================

#[cfg(windows)]
mod winapi_io {
    //! Phần thật sự đụng registry, `powercfg`, `bcdedit`, PowerShell, Defender.
    //! Chỉ được biên dịch trên Windows.

    use super::*;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// Chạy chương trình ẩn cửa sổ console, gom stdout+stderr (bản nhỏ của `sys::run_hidden`).
    fn run_hidden(program: &str, args: &[&str]) -> Result<String, String> {
        use std::os::windows::process::CommandExt;

        let out = std::process::Command::new(program)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("chạy {} lỗi: {}", program, e))?;
        let text = format!(
            "{}{}",
            decode_win_output(&out.stdout),
            decode_win_output(&out.stderr)
        );
        if out.status.success() {
            Ok(text)
        } else {
            let msg = text.trim().to_string();
            Err(if msg.is_empty() {
                format!("{} trả mã {}", program, out.status)
            } else {
                format!("{}: {}", program, msg)
            })
        }
    }

    /// Chạy PowerShell ẩn cửa sổ; `$ErrorActionPreference='Stop'` để lỗi thành exit code khác 0.
    fn ps_run(body: &str) -> Result<String, String> {
        let script = format!(
            "$ErrorActionPreference='Stop'; {}; if ($?) {{ exit 0 }} else {{ exit 1 }}",
            body
        );
        run_hidden(
            "powershell",
            &["-NoProfile", "-NonInteractive", "-Command", &script],
        )
    }

    /// Bọc một chuỗi thành literal PowerShell (nháy đơn, nháy đơn bên trong nhân đôi).
    fn ps_quote(s: &str) -> String {
        format!("'{}'", s.replace('\'', "''"))
    }

    fn hive_raw(h: Hive) -> winreg::HKEY {
        match h {
            Hive::Hkcu => winreg::enums::HKEY_CURRENT_USER,
            Hive::Hklm => winreg::enums::HKEY_LOCAL_MACHINE,
        }
    }

    /// Khoá `Services\<name>` của một service (None nếu không có trên máy này).
    fn open_service_key(name: &str) -> Result<Option<winreg::RegKey>, String> {
        let root = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE);
        let path = format!("SYSTEM\\CurrentControlSet\\Services\\{}", name);
        match root.open_subkey_with_flags(&path, winreg::enums::KEY_READ) {
            Ok(k) => Ok(Some(k)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("mở {} lỗi: {}", path, e)),
        }
    }

    /// Mở khoá registry để đọc; `Ok(None)` khi khoá chưa tồn tại.
    fn open_read(hive: Hive, path: &str) -> Result<Option<winreg::RegKey>, String> {
        let root = winreg::RegKey::predef(hive_raw(hive));
        match root.open_subkey_with_flags(path, winreg::enums::KEY_READ) {
            Ok(k) => Ok(Some(k)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("mở {}\\{} lỗi: {}", hive.label(), path, e)),
        }
    }

    /// Mở (tạo nếu thiếu) khoá registry để ghi.
    fn open_write(hive: Hive, path: &str) -> Result<winreg::RegKey, String> {
        let root = winreg::RegKey::predef(hive_raw(hive));
        root.create_subkey(path)
            .map(|(k, _)| k)
            .map_err(|e| format!("tạo {}\\{} lỗi: {}", hive.label(), path, e))
    }

    fn read_dword(hive: Hive, path: &str, name: &str) -> Result<ReadValue, String> {
        let key = match open_read(hive, path)? {
            Some(k) => k,
            None => return Ok(ReadValue::Missing),
        };
        match key.get_value::<u32, _>(name) {
            Ok(v) => Ok(ReadValue::Dword(v)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ReadValue::Missing),
            Err(e) => Err(format!("đọc {}\\{} [{}] lỗi: {}", hive.label(), path, name, e)),
        }
    }

    fn read_string(hive: Hive, path: &str, name: &str) -> Result<ReadValue, String> {
        let key = match open_read(hive, path)? {
            Some(k) => k,
            None => return Ok(ReadValue::Missing),
        };
        match key.get_value::<String, _>(name) {
            Ok(v) => Ok(ReadValue::Text(v)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ReadValue::Missing),
            Err(e) => Err(format!("đọc {}\\{} [{}] lỗi: {}", hive.label(), path, name, e)),
        }
    }

    fn read_multi(hive: Hive, path: &str, name: &str) -> Result<ReadValue, String> {
        let key = match open_read(hive, path)? {
            Some(k) => k,
            None => return Ok(ReadValue::Missing),
        };
        match key.get_value::<Vec<String>, _>(name) {
            Ok(v) => Ok(ReadValue::Multi(v)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ReadValue::Missing),
            Err(_) => {
                // Một số máy để giá trị này ở dạng REG_SZ -> đọc kiểu chuỗi rồi tự tách dòng.
                match key.get_value::<String, _>(name) {
                    Ok(s) => Ok(ReadValue::Multi(
                        s.split(['\n', '\0'])
                            .map(|x| x.trim().to_string())
                            .filter(|x| !x.is_empty())
                            .collect(),
                    )),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ReadValue::Missing),
                    Err(e) => Err(format!(
                        "đọc {}\\{} [{}] lỗi: {}",
                        hive.label(),
                        path,
                        name,
                        e
                    )),
                }
            }
        }
    }

    fn read_service_start(name: &str) -> Result<ReadValue, String> {
        let key = match open_service_key(name)? {
            Some(k) => k,
            None => return Ok(ReadValue::Missing),
        };
        match key.get_value::<u32, _>("Start") {
            Ok(v) => Ok(ReadValue::Dword(v)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ReadValue::Missing),
            Err(e) => Err(format!("đọc Start của service {} lỗi: {}", name, e)),
        }
    }

    fn read_power_ac(sub: &str, setting: &str) -> Result<ReadValue, String> {
        // `/q` không in chỉ số khi setting bị Windows ẩn (vd Processor performance boost mode):
        // bỏ cờ ẩn (`-ATTRIB_HIDE`, chỉ làm setting hiện trong Power Options) rồi đọc lại một lần.
        // Vẫn không có => Unknown, KHÔNG coi là lỗi.
        let query = || {
            run_hidden("powercfg", &["/q", "SCHEME_CURRENT", sub, setting])
                .ok()
                .and_then(|out| parse_powercfg_ac(&out))
        };
        let v = query().or_else(|| {
            let _ = run_hidden("powercfg", &["-attributes", sub, setting, "-ATTRIB_HIDE"]);
            query()
        });
        Ok(match v {
            Some(v) => ReadValue::Dword(v),
            None => ReadValue::Unknown,
        })
    }

    fn read_bcd(name: &str) -> Result<ReadValue, String> {
        // bcdedit cần quyền admin; lỗi ở đây thì `apply` dừng luôn (không dám đổi khi chưa sao lưu được).
        let out = run_hidden("bcdedit", &["/enum", "{current}"])?;
        Ok(match parse_bcdedit_value(&out, name) {
            Some(v) => ReadValue::Text(v),
            None => ReadValue::Missing,
        })
    }

    fn read_mem_compression() -> Result<ReadValue, String> {
        let out = ps_run("(Get-MMAgent).MemoryCompression")?;
        match out.trim() {
            v if v.eq_ignore_ascii_case("true") => Ok(ReadValue::Flag(true)),
            v if v.eq_ignore_ascii_case("false") => Ok(ReadValue::Flag(false)),
            _ => Ok(ReadValue::Unknown),
        }
    }

    fn read_exclusions() -> Result<ReadValue, String> {
        let out = ps_run("(Get-MpPreference).ExclusionPath")?;
        let list: Vec<String> = out
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        Ok(if list.is_empty() {
            ReadValue::Missing
        } else {
            ReadValue::Multi(list)
        })
    }

    pub fn read_action(a: &Action) -> Result<ReadValue, String> {
        match a {
            Action::RegDword { hive, path, name, .. } => read_dword(*hive, path, name),
            Action::RegString { hive, path, name, .. } => read_string(*hive, path, name),
            Action::RegMultiString { hive, path, name, .. } => read_multi(*hive, path, name),
            Action::ServiceStart { name, .. } => read_service_start(name),
            Action::PowerAc {
                subgroup_guid,
                setting_guid,
                ..
            } => read_power_ac(subgroup_guid, setting_guid),
            Action::Bcd { name, .. } => read_bcd(name),
            Action::MemoryCompression(_) => read_mem_compression(),
            Action::DefenderExclusions(_) => read_exclusions(),
        }
    }

    fn write_dword(hive: Hive, path: &str, name: &str, v: u32) -> Result<(), String> {
        let key = open_write(hive, path)?;
        key.set_value(name, &v)
            .map_err(|e| format!("ghi {}\\{} [{}] lỗi: {}", hive.label(), path, name, e))
    }

    fn write_string(hive: Hive, path: &str, name: &str, v: &str) -> Result<(), String> {
        let key = open_write(hive, path)?;
        key.set_value(name, &v)
            .map_err(|e| format!("ghi {}\\{} [{}] lỗi: {}", hive.label(), path, name, e))
    }

    fn write_multi(hive: Hive, path: &str, name: &str, v: &[String]) -> Result<(), String> {
        let key = open_write(hive, path)?;
        let list: Vec<String> = v.to_vec();
        key.set_value(name, &list)
            .map_err(|e| format!("ghi {}\\{} [{}] lỗi: {}", hive.label(), path, name, e))
    }

    /// Xoá một giá trị (khoá hoặc giá trị không có thì coi như xong).
    fn delete_value(hive: Hive, path: &str, name: &str) -> Result<(), String> {
        let root = winreg::RegKey::predef(hive_raw(hive));
        let key = match root.open_subkey_with_flags(path, winreg::enums::KEY_READ | winreg::enums::KEY_WRITE)
        {
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("mở {}\\{} lỗi: {}", hive.label(), path, e)),
        };
        match key.delete_value(name) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("xoá {}\\{} [{}] lỗi: {}", hive.label(), path, name, e)),
        }
    }

    /// Đổi kiểu khởi động qua Service Control Manager (`ChangeServiceConfigW`, như `sc config`):
    /// SCM cập nhật ngay và không đụng cờ "Delayed Start". 1.5.0–1.5.1 mở khoá registry của
    /// service chỉ với KEY_READ rồi `set_value` nên luôn "Access is denied".
    fn write_service_start(name: &str, start: StartType) -> Result<(), String> {
        use windows_sys::Win32::System::Services::{
            ChangeServiceConfigW, CloseServiceHandle, OpenSCManagerW, OpenServiceW,
            SC_MANAGER_CONNECT, SERVICE_CHANGE_CONFIG, SERVICE_NO_CHANGE,
        };
        if open_service_key(name)?.is_none() {
            // Service không có trên máy này -> bỏ qua, không lỗi.
            return Ok(());
        }
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: chuỗi UTF-16 kết thúc 0 sống tới hết khối; handle đóng ở mọi nhánh.
        unsafe {
            let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
            if scm.is_null() {
                return Err(format!(
                    "mở Service Control Manager lỗi: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let svc = OpenServiceW(scm, wide.as_ptr(), SERVICE_CHANGE_CONFIG);
            if svc.is_null() {
                let e = std::io::Error::last_os_error();
                CloseServiceHandle(scm);
                return Err(format!("mở service {} để đổi cấu hình lỗi: {}", name, e));
            }
            let ok = ChangeServiceConfigW(
                svc,
                SERVICE_NO_CHANGE,
                start.value(),
                SERVICE_NO_CHANGE,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            );
            let e = std::io::Error::last_os_error();
            CloseServiceHandle(svc);
            CloseServiceHandle(scm);
            if ok == 0 {
                return Err(format!(
                    "đổi kiểu khởi động service {} sang {} lỗi: {}",
                    name,
                    start.label(),
                    e
                ));
            }
        }
        Ok(())
    }

    fn write_power_ac(sub: &str, setting: &str, v: u32) -> Result<(), String> {
        let val = v.to_string();
        run_hidden(
            "powercfg",
            &["/setacvalueindex", "SCHEME_CURRENT", sub, setting, &val],
        )?;
        run_hidden("powercfg", &["/setactive", "SCHEME_CURRENT"])?;
        Ok(())
    }

    fn write_bcd(name: &str, value: &str) -> Result<(), String> {
        run_hidden("bcdedit", &["/set", "{current}", name, value]).map(|_| ())
    }

    fn delete_bcd(name: &str) -> Result<(), String> {
        run_hidden("bcdedit", &["/deletevalue", "{current}", name]).map(|_| ())
    }

    fn write_mem_compression(on: bool) -> Result<(), String> {
        let body = if on {
            "Enable-MMAgent -MemoryCompression"
        } else {
            "Disable-MMAgent -MemoryCompression"
        };
        ps_run(body).map(|_| ())
    }

    /// Thêm thư mục loại trừ (chỉ thêm cái chưa có, tránh ghi trùng).
    fn write_exclusions(dirs: &[String]) -> Result<(), String> {
        if dirs.is_empty() {
            return Ok(());
        }
        let current = match read_exclusions()? {
            ReadValue::Multi(v) => v,
            _ => Vec::new(),
        };
        for d in dirs {
            let d = d.trim();
            if d.is_empty() || contains_path(&current, d) {
                continue;
            }
            ps_run(&format!(
                "Add-MpPreference -ExclusionPath {}",
                ps_quote(d)
            ))?;
        }
        Ok(())
    }

    /// Gỡ đúng những thư mục mà module này đã thêm (so với danh sách gốc).
    fn remove_exclusions(dirs: &[String], original: &[String]) -> Result<(), String> {
        for d in dirs {
            let d = d.trim();
            if d.is_empty() || contains_path(original, d) {
                continue;
            }
            ps_run(&format!(
                "Remove-MpPreference -ExclusionPath {}",
                ps_quote(d)
            ))?;
        }
        Ok(())
    }

    pub fn write_action(a: &Action) -> Result<(), String> {
        match a {
            Action::RegDword {
                hive,
                path,
                name,
                value,
            } => write_dword(*hive, path, name, *value),
            Action::RegString {
                hive,
                path,
                name,
                value,
            } => write_string(*hive, path, name, value),
            Action::RegMultiString {
                hive,
                path,
                name,
                value,
            } => write_multi(*hive, path, name, value),
            Action::ServiceStart { name, start } => write_service_start(name, *start),
            Action::PowerAc {
                subgroup_guid,
                setting_guid,
                value,
            } => write_power_ac(subgroup_guid, setting_guid, *value),
            Action::Bcd { name, value } => write_bcd(name, value),
            Action::MemoryCompression(on) => write_mem_compression(*on),
            Action::DefenderExclusions(dirs) => write_exclusions(dirs),
        }
    }

    /// Ghi lại giá trị gốc đã lưu.
    pub fn restore_action(a: &Action, saved: &SavedValue) -> Result<(), String> {
        match (a, saved) {
            // Không áp dụng được lúc bật thì cũng không có gì để trả lại.
            (_, SavedValue::Skipped) => Ok(()),
            (Action::RegDword { hive, path, name, .. }, SavedValue::Dword(v)) => {
                write_dword(*hive, path, name, *v)
            }
            (Action::RegDword { hive, path, name, .. }, SavedValue::Missing) => {
                delete_value(*hive, path, name)
            }
            (Action::RegString { hive, path, name, .. }, SavedValue::Text(v)) => {
                write_string(*hive, path, name, v)
            }
            (Action::RegString { hive, path, name, .. }, SavedValue::Missing) => {
                delete_value(*hive, path, name)
            }
            (Action::RegMultiString { hive, path, name, .. }, SavedValue::Multi(v)) => {
                write_multi(*hive, path, name, v)
            }
            (Action::RegMultiString { hive, path, name, .. }, SavedValue::Missing) => {
                delete_value(*hive, path, name)
            }
            (Action::ServiceStart { name, .. }, SavedValue::Dword(v)) => match StartType::from_value(*v)
            {
                Some(st) => write_service_start(name, st),
                // Giá trị Start lạ (0/1 = boot/system driver) thì ghi thẳng số gốc.
                None => match winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
                    .open_subkey_with_flags(
                        format!("SYSTEM\\CurrentControlSet\\Services\\{}", name),
                        winreg::enums::KEY_READ | winreg::enums::KEY_SET_VALUE,
                    )
                    .ok()
                {
                    Some(k) => k
                        .set_value("Start", v)
                        .map_err(|e| format!("trả Start của service {} lỗi: {}", name, e)),
                    None => Ok(()),
                },
            },
            // Service không tồn tại lúc bật nhưng giờ có: không biết giá trị gốc -> bỏ qua.
            (Action::ServiceStart { .. }, SavedValue::Missing) => Ok(()),
            (
                Action::PowerAc {
                    subgroup_guid,
                    setting_guid,
                    ..
                },
                SavedValue::Dword(v),
            ) => write_power_ac(subgroup_guid, setting_guid, *v),
            (Action::PowerAc { .. }, SavedValue::Missing) => Ok(()),
            (Action::Bcd { name, .. }, SavedValue::Text(v)) => write_bcd(name, v),
            (Action::Bcd { name, .. }, SavedValue::Missing) => delete_bcd(name),
            (Action::MemoryCompression(_), SavedValue::Flag(b)) => write_mem_compression(*b),
            (Action::DefenderExclusions(dirs), SavedValue::Multi(original)) => {
                remove_exclusions(dirs, original)
            }
            // Lúc bật chưa có thư mục loại trừ nào: gỡ hết những thư mục ta đã thêm.
            (Action::DefenderExclusions(dirs), SavedValue::Missing) => remove_exclusions(dirs, &[]),
            // Không đọc được giá trị gốc thì `apply` đã từ chối ghi — tới đây là file lạ, báo rõ.
            (a, SavedValue::Unreadable) => Err(format!(
                "Không có giá trị gốc cho {} nên không hoàn tác được",
                a.target()
            )),
            (a, s) => Err(format!(
                "Bản sao lưu của {} không khớp kiểu giá trị ({:?})",
                a.target(),
                s
            )),
        }
    }

}

#[cfg(windows)]
use winapi_io as backend;

#[cfg(not(windows))]
mod backend {
    //! Trên hệ khác Windows thì mọi thao tác đều không làm được: trả lỗi tiếng Việt, không panic.
    use super::*;

    pub fn read_action(_a: &Action) -> Result<ReadValue, String> {
        Err(NOT_WINDOWS.to_string())
    }

    pub fn write_action(_a: &Action) -> Result<(), String> {
        Err(NOT_WINDOWS.to_string())
    }

    pub fn restore_action(_a: &Action, _saved: &SavedValue) -> Result<(), String> {
        Err(NOT_WINDOWS.to_string())
    }
}

// =====================================================================================
// Trạng thái / bật / hoàn tác
// =====================================================================================

/// Trạng thái hiện tại của tinh chỉnh trên máy này.
///
/// `Applied` khi MỌI thao tác đang đúng giá trị đích (thao tác không áp dụng được thì bỏ qua).
pub fn state(t: &Tweak) -> TweakState {
    let mut states: Vec<ActionState> = Vec::with_capacity(t.actions.len());
    let mut first_error: Option<String> = None;
    for a in &t.actions {
        match backend::read_action(a) {
            Ok(v) => {
                let st = action_state(a, &v);
                if st == ActionState::Unknown && first_error.is_none() {
                    first_error = Some(format!("không đọc được trạng thái của {}", a.target()));
                }
                states.push(st);
            }
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
                states.push(ActionState::Unknown);
            }
        }
    }
    match (combine_states(&states), first_error) {
        (TweakState::Unknown(_), Some(msg)) => TweakState::Unknown(msg),
        (s, _) => s,
    }
}

/// Bật một tinh chỉnh: đọc và lưu giá trị gốc TRƯỚC khi ghi, lỗi giữa chừng thì trả lại
/// những thao tác đã ghi của lần này.
///
/// Không đọc được giá trị gốc thì trả lỗi và **không ghi gì** — yêu cầu bắt buộc của module
/// là mọi tinh chỉnh phải hoàn tác được về đúng giá trị trước đó.
pub fn apply(t: &Tweak, backup: &mut Backup) -> Result<(), String> {
    if t.actions.is_empty() {
        return Err(format!("Tinh chỉnh “{}” không có thao tác nào.", t.title));
    }
    let already = backup.has(t.id);
    let originals: Vec<SavedValue> = if already {
        let entry = match backup.entries.get(t.id) {
            Some(e) => e,
            None => return Err(format!("Không đọc được bản sao lưu của “{}”.", t.title)),
        };
        if entry.originals.len() != t.actions.len() {
            return Err(format!(
                "Bản sao lưu của “{}” không khớp số thao tác ({} so với {}) — bấm Hoàn tác rồi bật lại.",
                t.title,
                entry.originals.len(),
                t.actions.len()
            ));
        }
        entry.originals.clone()
    } else {
        let mut list = Vec::with_capacity(t.actions.len());
        for a in &t.actions {
            let cur = backend::read_action(a)?;
            let sv = saved_from_read(a, cur);
            if sv == SavedValue::Unreadable {
                return Err(format!(
                    "Không đọc được giá trị gốc của {} nên không đổi (phải chắc chắn hoàn tác được).",
                    a.target()
                ));
            }
            list.push(sv);
        }
        list
    };

    let mut done: Vec<usize> = Vec::new();
    for (i, a) in t.actions.iter().enumerate() {
        if originals[i] == SavedValue::Skipped {
            continue;
        }
        if let Err(e) = backend::write_action(a) {
            // Trả lại đúng những thao tác đã ghi trong lần này (thứ tự ngược).
            for j in done.iter().rev() {
                let _ = backend::restore_action(&t.actions[*j], &originals[*j]);
            }
            return Err(if done.is_empty() {
                e
            } else {
                format!("{} (đã trả lại {} thao tác vừa đổi)", e, done.len())
            });
        }
        done.push(i);
    }

    if !already {
        backup.entries.insert(
            t.id.to_string(),
            BackupEntry {
                tweak_id: t.id.to_string(),
                saved_at: now_unix(),
                originals,
            },
        );
    }
    Ok(())
}

/// Hoàn tác một tinh chỉnh về đúng giá trị gốc, xong thì xoá nó khỏi bản sao lưu.
///
/// Giá trị gốc là "không tồn tại" thì xoá giá trị (`bcdedit /deletevalue` với BCD);
/// thư mục loại trừ của Defender chỉ gỡ đúng những thư mục module này đã thêm.
pub fn revert(t: &Tweak, backup: &mut Backup) -> Result<(), String> {
    let entry = match backup.entries.get(t.id) {
        Some(e) => e.clone(),
        None => {
            return Err(format!(
                "Không có bản sao lưu cho “{}” nên không hoàn tác được.",
                t.title
            ))
        }
    };
    if entry.originals.len() != t.actions.len() {
        return Err(format!(
            "Bản sao lưu của “{}” không khớp số thao tác ({} so với {}) — giữ nguyên file để kiểm tra tay.",
            t.title,
            entry.originals.len(),
            t.actions.len()
        ));
    }

    let mut errors: Vec<String> = Vec::new();
    for (a, saved) in t.actions.iter().zip(entry.originals.iter()) {
        if *saved == SavedValue::Skipped {
            continue;
        }
        if let Err(e) = backend::restore_action(a, saved) {
            errors.push(e);
        }
    }
    if !errors.is_empty() {
        // Giữ lại bản sao lưu để người dùng còn bấm Hoàn tác lần nữa sau khi sửa nguyên nhân.
        return Err(format!(
            "Hoàn tác “{}” chưa xong: {}",
            t.title,
            errors.join(" · ")
        ));
    }
    backup.entries.remove(t.id);
    Ok(())
}

// =====================================================================================
// Danh sách tinh chỉnh
// =====================================================================================

fn dword(hive: Hive, path: &'static str, name: &'static str, value: u32) -> Action {
    Action::RegDword {
        hive,
        path,
        name,
        value,
    }
}

fn text(hive: Hive, path: &'static str, name: &'static str, value: &'static str) -> Action {
    Action::RegString {
        hive,
        path,
        name,
        value,
    }
}

fn multi(hive: Hive, path: &'static str, name: &'static str, value: &[&str]) -> Action {
    Action::RegMultiString {
        hive,
        path,
        name,
        value: value.iter().map(|s| s.to_string()).collect(),
    }
}

fn service(name: &'static str, start: StartType) -> Action {
    Action::ServiceStart { name, start }
}

fn power_ac(subgroup_guid: &'static str, setting_guid: &'static str, value: u32) -> Action {
    Action::PowerAc {
        subgroup_guid,
        setting_guid,
        value,
    }
}

// Đường dẫn registry dùng nhiều lần.
const CDM: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\ContentDeliveryManager";
const GAME_DVR_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\GameDVR";
const ADVANCED: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced";
const SYSTEM_PROFILE: &str =
    "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Multimedia\\SystemProfile";
const MEM_MGMT: &str = "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Memory Management";
const SESSION_POWER: &str = "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Power";
const GRAPHICS: &str = "SYSTEM\\CurrentControlSet\\Control\\GraphicsDrivers";
const POWER_THROTTLING: &str = "SYSTEM\\CurrentControlSet\\Control\\Power\\PowerThrottling";

fn tweak(
    id: &'static str,
    group: Group,
    risk: Risk,
    title: &'static str,
    desc: &'static str,
    needs_reboot: bool,
    recommended: bool,
    actions: Vec<Action>,
) -> Tweak {
    Tweak {
        id,
        group,
        risk,
        title,
        desc,
        needs_reboot,
        recommended,
        actions,
    }
}

/// Toàn bộ tinh chỉnh. `game_dirs` là thư mục chứa exe game (để thêm loại trừ Defender);
/// rỗng thì không tạo tinh chỉnh `defender_games`.
pub fn all_tweaks(game_dirs: &[String]) -> Vec<Tweak> {
    let mut list = Vec::new();

    // ----- Hiệu năng game -----
    list.push(tweak(
        "game_mode",
        Group::Game,
        Risk::Safe,
        "Chế độ game của Windows",
        "Bật Game Mode để Windows ưu tiên CPU/GPU cho game đang mở và tạm hoãn Windows Update. \
         Không ảnh hưởng gì tới app khác khi không chơi.",
        false,
        true,
        vec![
            dword(Hive::Hkcu, "Software\\Microsoft\\GameBar", "AutoGameModeEnabled", 1),
            dword(Hive::Hkcu, "Software\\Microsoft\\GameBar", "AllowAutoGameMode", 1),
        ],
    ));

    list.push(tweak(
        "hags",
        Group::Game,
        Risk::Safe,
        "Lập lịch GPU bằng phần cứng (HAGS)",
        "Cho GPU tự quản lý hàng đợi lệnh vẽ, thường giảm độ trễ khung hình vài ms khi chơi. \
         Có hiệu lực sau khi khởi động lại; driver cũ có thể chậm hơn chút.",
        true,
        true,
        vec![dword(Hive::Hklm, GRAPHICS, "HwSchMode", 2)],
    ));

    list.push(tweak(
        "windowed_opt",
        Group::Game,
        Risk::Safe,
        "Tối ưu game chạy cửa sổ (SwapEffectUpgrade)",
        "Bật tối ưu DirectX cho game chạy dạng cửa sổ không viền, giúp đường vẽ thẳng ra màn hình \
         nên ít khựng hơn. Không đổi gì khi game chạy toàn màn hình.",
        false,
        true,
        vec![text(
            Hive::Hkcu,
            "Software\\Microsoft\\DirectX\\UserGpuPreferences",
            "DirectXUserGlobalSettings",
            "SwapEffectUpgradeEnable=1;",
        )],
    ));

    list.push(tweak(
        "mmcss_games",
        Group::Game,
        Risk::Safe,
        "Ưu tiên đa phương tiện cho game",
        "Windows dành nhiều CPU hơn cho luồng game (giảm ngưỡng dành riêng cho tác vụ nền) và \
         bỏ giới hạn băng thông mạng 10 Mbps vốn bóp ping tải nội dung. Không hại app thường.",
        false,
        true,
        vec![
            dword(Hive::Hklm, SYSTEM_PROFILE, "SystemResponsiveness", 10),
            dword(Hive::Hklm, SYSTEM_PROFILE, "NetworkThrottlingIndex", 0xFFFF_FFFF),
            dword(Hive::Hklm, "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Multimedia\\SystemProfile\\Tasks\\Games", "GPU Priority", 8),
            dword(Hive::Hklm, "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Multimedia\\SystemProfile\\Tasks\\Games", "Priority", 6),
            text(Hive::Hklm, "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Multimedia\\SystemProfile\\Tasks\\Games", "Scheduling Category", "High"),
            text(Hive::Hklm, "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Multimedia\\SystemProfile\\Tasks\\Games", "SFIO Priority", "High"),
        ],
    ));

    list.push(tweak(
        "no_gamedvr",
        Group::Game,
        Risk::Safe,
        "Tắt Game DVR / ghi lại trận đấu",
        "Tắt phần ghi lại nền của Xbox Game Bar — thứ vẫn chạy song song khi chơi và ăn cả CPU \
         lẫn GPU. Mất: không dùng được nút Win+Alt+R để quay clip.",
        false,
        true,
        vec![
            dword(Hive::Hkcu, "System\\GameConfigStore", "GameDVR_Enabled", 0),
            dword(Hive::Hkcu, GAME_DVR_KEY, "AppCaptureEnabled", 0),
            dword(Hive::Hklm, "SOFTWARE\\Policies\\Microsoft\\Windows\\GameDVR", "AllowGameDVR", 0),
        ],
    ));

    list.push(tweak(
        "mouse_raw",
        Group::Game,
        Risk::Consider,
        "Chuột thô: tắt tăng tốc con trỏ",
        "Bỏ \"Enhance pointer precision\" để chuột đi thẳng theo số đếm của cảm biến, ngắm dễ đều tay \
         hơn. Mất: con trỏ di chậm hơn khi kéo xa; có hiệu lực sau khi ĐĂNG NHẬP LẠI.",
        false,
        false,
        vec![
            text(Hive::Hkcu, "Control Panel\\Mouse", "MouseSpeed", "0"),
            text(Hive::Hkcu, "Control Panel\\Mouse", "MouseThreshold1", "0"),
            text(Hive::Hkcu, "Control Panel\\Mouse", "MouseThreshold2", "0"),
        ],
    ));

    let dirs: Vec<String> = game_dirs
        .iter()
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .collect();
    if !dirs.is_empty() {
        list.push(tweak(
            "defender_games",
            Group::Game,
            Risk::Consider,
            "Defender bỏ quét thư mục game",
            "Thêm thư mục chứa game vào danh sách loại trừ của Windows Defender để bớt khựng lúc \
             nạp bản đồ và ít tốn CPU khi vá gói. Mất: vùng đó không còn được quét virus.",
            false,
            false,
            vec![Action::DefenderExclusions(dirs)],
        ));
    }

    list.push(tweak(
        "power_throttling_off",
        Group::Game,
        Risk::Consider,
        "Tắt Power Throttling",
        "Windows thôi tự hạ xung các tiến trình nó cho là \"nền\", nhờ vậy ít tụt FPS đột ngột. \
         Mất: máy chạy pin sẽ tốn pin hơn và ấm hơn.",
        false,
        false,
        vec![dword(Hive::Hklm, POWER_THROTTLING, "PowerThrottlingOff", 1)],
    ));

    // ----- Giảm tác vụ nền -----
    list.push(tweak(
        "bg_apps_off",
        Group::Background,
        Risk::Safe,
        "Chặn app chạy nền",
        "Windows thôi cho app Store/UWP tự chạy nền và tắt công tắc tìm kiếm nền — bớt đĩa và CPU \
         rảnh rỗi. Mất: app phải tự mở lại mới cập nhật thông báo.",
        false,
        true,
        vec![
            dword(Hive::Hkcu, "Software\\Microsoft\\Windows\\CurrentVersion\\BackgroundAccessApplications", "GlobalUserDisabled", 1),
            dword(Hive::Hkcu, "Software\\Microsoft\\Windows\\CurrentVersion\\Search", "BackgroundAppGlobalToggle", 0),
        ],
    ));

    list.push(tweak(
        "no_suggestions",
        Group::Background,
        Risk::Safe,
        "Tắt gợi ý và app tự cài",
        "Bỏ mọi thẻ gợi ý trong Start / Settings, ảnh \"mẹo dùng Windows\" và việc tự cài app \
         quảng cáo. Mất: không còn mục gợi ý nào của Microsoft.",
        false,
        true,
        vec![
            dword(Hive::Hkcu, CDM, "SystemPaneSuggestionsEnabled", 0),
            dword(Hive::Hkcu, CDM, "SubscribedContent-338388Enabled", 0),
            dword(Hive::Hkcu, CDM, "SubscribedContent-338389Enabled", 0),
            dword(Hive::Hkcu, CDM, "SubscribedContent-353694Enabled", 0),
            dword(Hive::Hkcu, CDM, "SubscribedContent-353696Enabled", 0),
            dword(Hive::Hkcu, CDM, "SilentInstalledAppsEnabled", 0),
            dword(Hive::Hkcu, CDM, "SoftLandingEnabled", 0),
        ],
    ));

    // Không có "Tắt Widget" (`HKLM\SOFTWARE\Policies\Microsoft\Dsh` `AllowNewsAndInterests`):
    // Windows 11 chặn ghi value này bằng driver UCPD (kể cả reg.exe quyền admin) — đã bỏ ở 1.5.1,
    // không lách UCPD. Ẩn nút Widgets thì Windows tự làm qua Settings > Personalization > Taskbar.

    list.push(tweak(
        "telemetry_min",
        Group::Background,
        Risk::Safe,
        "Giảm dữ liệu chẩn đoán về mức thấp nhất",
        "Đặt AllowTelemetry=0 và tắt service DiagTrack + dmwappushservice — bớt ghi log và gửi dữ liệu \
         dùng máy về Microsoft. Mất: Windows Update chậm nhận bản vá hơn một chút.",
        false,
        true,
        vec![
            dword(Hive::Hklm, "SOFTWARE\\Policies\\Microsoft\\Windows\\DataCollection", "AllowTelemetry", 0),
            service("DiagTrack", StartType::Disabled),
            service("dmwappushservice", StartType::Disabled),
        ],
    ));

    list.push(tweak(
        "delivery_opt_p2p_off",
        Group::Background,
        Risk::Safe,
        "Tắt chia sẻ bản cập nhật ngang hàng",
        "Windows không còn lấy/gửi bản cập nhật qua máy khác trong mạng, đỡ tốn băng thông và đĩa. \
         Mất: tải bản vá chỉ còn lấy từ server Microsoft nên có thể chậm hơn.",
        false,
        true,
        vec![dword(Hive::Hklm, "SOFTWARE\\Policies\\Microsoft\\Windows\\DeliveryOptimization", "DODownloadMode", 0)],
    ));

    list.push(tweak(
        "edge_bg_off",
        Group::Background,
        Risk::Safe,
        "Tắt Edge chạy nền và khởi động sẵn",
        "Edge thôi tự chạy nền sau khi đóng và thôi nạp sẵn khi mở máy — nhẹ RAM, nhanh khởi động. \
         Mất: lần đầu mở Edge sẽ lâu hơn một chút.",
        false,
        true,
        vec![
            dword(Hive::Hklm, "SOFTWARE\\Policies\\Microsoft\\Edge", "StartupBoostEnabled", 0),
            dword(Hive::Hklm, "SOFTWARE\\Policies\\Microsoft\\Edge", "BackgroundModeEnabled", 0),
        ],
    ));

    list.push(tweak(
        "startup_delay_0",
        Group::Background,
        Risk::Safe,
        "Bỏ trễ 10 giây của app khởi động cùng Windows",
        "Explorer không còn chờ 10 giây mới chạy các mục trong Startup, nên app quen dùng \
         (Discord, Tailscale…) sẵn sàng sớm hơn. Mất: lúc vừa đăng nhập máy bận hơn trong vài giây.",
        false,
        true,
        vec![dword(
            Hive::Hkcu,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Serialize",
            "StartupDelayInMSec",
            0,
        )],
    ));

    list.push(tweak(
        "animations_off",
        Group::Background,
        Risk::Consider,
        "Tắt hiệu ứng hình ảnh của Windows",
        "Tắt hiệu ứng thu/phóng cửa sổ, animation taskbar và nền trong suốt — cảm giác máy nhanh, \
         nhẹ GPU hơn. Mất: giao diện trông khô hơn và không còn trong suốt.",
        false,
        false,
        vec![
            text(Hive::Hkcu, "Control Panel\\Desktop\\WindowMetrics", "MinAnimate", "0"),
            dword(Hive::Hkcu, ADVANCED, "TaskbarAnimations", 0),
            dword(Hive::Hkcu, "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize", "EnableTransparency", 0),
        ],
    ));

    list.push(tweak(
        "services_trim",
        Group::Background,
        Risk::Consider,
        "Tắt vài service ít dùng",
        "Tắt service bản đồ, chế độ trình diễn, fax, báo lỗi Windows, giám sát ứng dụng và chuyển \
         vị trí sang Thủ công — bớt vài chục MB RAM và tác vụ nền. Mất: tính năng tương ứng \
         (báo lỗi, fax, định vị, ảnh bản đồ) không chạy cho tới khi hoàn tác.",
        false,
        false,
        vec![
            service("MapsBroker", StartType::Disabled),
            service("RetailDemo", StartType::Disabled),
            service("Fax", StartType::Disabled),
            service("WerSvc", StartType::Disabled),
            service("PcaSvc", StartType::Disabled),
            service("lfsvc", StartType::Manual),
        ],
    ));

    list.push(tweak(
        "sysmain_off",
        Group::Background,
        Risk::Consider,
        "Tắt SysMain (Superfetch)",
        "Tắt dịch vụ nạp trước ứng dụng, đỡ ghi đọc liên tục trên ổ cứng nên máy ít giật khi vừa \
         mở máy. Mất: app khởi động chậm hơn chút, nhất là khi ổ cứng cơ.",
        false,
        false,
        vec![service("SysMain", StartType::Disabled)],
    ));

    list.push(tweak(
        "search_index_off",
        Group::Background,
        Risk::Consider,
        "Tắt dịch vụ lập chỉ mục tìm kiếm",
        "Tắt Windows Search nên ổ đĩa không còn bị quét lập chỉ mục lúc rảnh — bớt khựng khi chơi. \
         Mất: tìm file trong Start / File Explorer CHẬM HƠN hẳn (chỉ tìm được theo tên trong \
         thư mục đang mở).",
        false,
        false,
        vec![service("WSearch", StartType::Disabled)],
    ));

    // ----- Bộ nhớ -----
    list.push(tweak(
        "mem_compression_on",
        Group::Memory,
        Risk::Safe,
        "Bật nén bộ nhớ",
        "Windows nén các trang RAM ít dùng thay vì đẩy ra pagefile — với máy RAM ≤ 16 GB sẽ bớt \
         khựng khi chơi game nặng. Đánh đổi: tốn thêm chút CPU. Có hiệu lực sau khi KHỞI ĐỘNG LẠI.",
        true,
        true,
        vec![Action::MemoryCompression(true)],
    ));

    list.push(tweak(
        "pagefile_fixed",
        Group::Memory,
        Risk::Consider,
        "Cố định pagefile 16–24 GB",
        "Đặt pagefile trên C: cố định trong khoảng 16–24 GB để Windows không tự co giãn gây khựng \
         lúc commit tăng vọt. Mất: tốn chừng đó dung lượng đĩa C: và cần khởi động lại.",
        true,
        false,
        vec![multi(
            Hive::Hklm,
            MEM_MGMT,
            "PagingFiles",
            &["C:\\pagefile.sys 16384 24576"],
        )],
    ));

    // ----- Hệ thống -----
    list.push(tweak(
        "fast_startup_off",
        Group::System,
        Risk::Consider,
        "Tắt Fast Startup",
        "Mỗi lần tắt máy là tắt thật, driver và nhân được nạp lại sạch nên ít lỗi vặt và game ổn \
         định hơn. Mất: mở máy chậm hơn vài giây so với trước.",
        false,
        false,
        vec![dword(Hive::Hklm, SESSION_POWER, "HiberbootEnabled", 0)],
    ));

    list.push(tweak(
        "pcie_aspm_off_ac",
        Group::System,
        Risk::Safe,
        "Tắt tiết kiệm điện PCIe khi cắm sạc",
        "Khi cắm sạc, khe PCIe (ổ NVMe, card rời) không còn tự hạ tốc độ để tiết kiệm điện — \
         giảm khựng lúc đọc đĩa. Khi dùng pin vẫn tiết kiệm như cũ.",
        false,
        true,
        vec![power_ac(
            "501a4d13-42af-4429-9fd1-a8218c268e20",
            "ee12f906-d277-404b-b6da-e5fa1a576df5",
            0,
        )],
    ));

    list.push(tweak(
        "cpu_boost_aggressive_ac",
        Group::System,
        Risk::Consider,
        "CPU luôn đẩy xung mạnh khi cắm sạc",
        "Đặt Processor performance boost mode = Aggressive: CPU vọt lên xung cao nhất ngay khi có \
         việc, ít tụt khung hình lúc đông người. Mất: nóng và tốn điện hơn khi cắm sạc; dùng pin \
         không đổi.",
        false,
        false,
        vec![power_ac(
            "54533251-82be-4824-96c1-47b60b740d00",
            "be337238-0d82-4146-a960-4f3749d470c7",
            2,
        )],
    ));

    list.push(tweak(
        "usb_suspend_off_ac",
        Group::System,
        Risk::Safe,
        "Không cho USB ngủ khi cắm sạc",
        "Chuột, bàn phím và tai nghe USB không bị Windows tắt điện để tiết kiệm nên hết kiểu \
         \"giật đầu tiên mới nhận\". Khi dùng pin vẫn tiết kiệm như cũ.",
        false,
        true,
        vec![power_ac(
            "2a737441-1930-4402-8d77-b2bebba308a3",
            "48e6b7a6-50f5-4782-a5d4-53bb8f07e226",
            0,
        )],
    ));

    list.push(tweak(
        "hypervisor_off",
        Group::System,
        Risk::Advanced,
        "Tắt lớp ảo hoá (VBS/Hyper-V)",
        "Tắt hypervisor để Windows không chạy lớp ảo hoá bảo vệ VBS, lấy lại vài % hiệu năng CPU \
         khi chơi. CẢNH BÁO: WSL2, Android/BlueStacks, Docker, Windows Sandbox và mọi app cần ảo \
         hoá sẽ KHÔNG chạy cho tới khi hoàn tác và khởi động lại.",
        true,
        false,
        vec![Action::Bcd {
            name: "hypervisorlaunchtype",
            value: "off",
        }],
    ));

    list
}

/// Số tinh chỉnh đang bật trên tổng số (cho dòng "Đã bật N/M").
pub fn summary(tweaks: &[Tweak], states: &[TweakState]) -> (usize, usize) {
    let applied = states.iter().filter(|s| s.is_applied()).count();
    (applied, tweaks.len())
}

/// Id của những tinh chỉnh được đánh dấu "nên bật" (cho nút "Bật mục đề xuất").
pub fn recommended_ids(tweaks: &[Tweak]) -> Vec<&'static str> {
    tweaks
        .iter()
        .filter(|t| t.recommended)
        .map(|t| t.id)
        .collect()
}

// =====================================================================================
// Unit test phần thuần (chạy được trên Linux)
// =====================================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Output `powercfg /q SCHEME_CURRENT <sub> <setting>` thật (tiếng Anh), đã cắt bớt.
    const POWERCFG_EN: &str = "\
Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced)
  Subgroup GUID: 54533251-82be-4824-96c1-47b60b740d00  (Processor power management)
    Power Setting GUID: be337238-0d82-4146-a960-4f3749d470c7  (Processor performance boost mode)
      Possible Setting Index: 0000000000000000
      Possible Setting Friendly Name: Disabled
      Possible Setting Index: 0000000000000001
      Possible Setting Friendly Name: Enabled
      Possible Setting Index: 0000000000000002
      Possible Setting Friendly Name: Aggressive
      Current AC Power Setting Index: 0x0000000000000002
      Current DC Power Setting Index: 0x0000000000000000
";

    /// Cùng nội dung nhưng giao diện Windows tiếng Việt.
    const POWERCFG_VI: &str = "\
Sơ đồ nguồn điện GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Cân bằng)
    GUID cài đặt nguồn điện: be337238-0d82-4146-a960-4f3749d470c7
      Chỉ số cài đặt nguồn điện AC hiện tại: 0x0000000000000002
      Chỉ số cài đặt nguồn điện DC hiện tại: 0x0000000000000000
";

    /// Output `bcdedit /enum {current}` thật, đã cắt bớt.
    const BCDEDIT: &str = "\
Windows Boot Loader
-------------------
identifier              {current}
device                  partition=C:
path                    \\Windows\\system32\\winload.efi
description             Windows 11
locale                  en-US
hypervisorlaunchtype    Off
nx                      OptIn
";

    #[test]
    fn doc_chi_so_ac_tu_powercfg() {
        assert_eq!(parse_powercfg_ac(POWERCFG_EN), Some(2));
        assert_eq!(parse_powercfg_ac(POWERCFG_VI), Some(2));
        // Không có dòng AC (đúng trường hợp setting bị ẩn) -> Unknown, không phải 0.
        let only_dc = "      Current DC Power Setting Index: 0x0000000000000000\n";
        assert_eq!(parse_powercfg_ac(only_dc), None);
        assert_eq!(parse_powercfg_ac(""), None);
        // Giá trị vượt u32 -> bỏ qua chứ không cắt bừa.
        let big = "      Current AC Power Setting Index: 0x1FFFFFFFF\n";
        assert_eq!(parse_powercfg_ac(big), None);
    }

    #[test]
    fn doc_gia_tri_bcdedit() {
        assert_eq!(
            parse_bcdedit_value(BCDEDIT, "hypervisorlaunchtype"),
            Some("Off".to_string())
        );
        // Tên mục không phân biệt hoa thường.
        assert_eq!(
            parse_bcdedit_value(BCDEDIT, "HYPERVISORLAUNCHTYPE"),
            Some("Off".to_string())
        );
        // Mục chưa từng có -> None (hoàn tác sẽ là /deletevalue).
        assert_eq!(parse_bcdedit_value(BCDEDIT, "hypervisorlaunchtype2"), None);
        // Giá trị nhiều từ vẫn lấy đủ.
        assert_eq!(
            parse_bcdedit_value(BCDEDIT, "description"),
            Some("Windows 11".to_string())
        );
        // Có tên nhưng không có giá trị -> None.
        assert_eq!(parse_bcdedit_value("hypervisorlaunchtype\n", "hypervisorlaunchtype"), None);
    }

    fn hags() -> Action {
        dword(Hive::Hklm, GRAPHICS, "HwSchMode", 2)
    }

    #[test]
    fn so_sanh_trang_thai_tung_thao_tac() {
        assert_eq!(
            action_state(&hags(), &ReadValue::Dword(2)),
            ActionState::Matched
        );
        assert_eq!(
            action_state(&hags(), &ReadValue::Dword(1)),
            ActionState::NotMatched
        );
        // Chưa từng có giá trị -> chưa bật (hoàn tác sẽ là xoá).
        assert_eq!(
            action_state(&hags(), &ReadValue::Missing),
            ActionState::NotMatched
        );
        assert_eq!(
            action_state(&hags(), &ReadValue::Text("2".into())),
            ActionState::Unknown
        );

        // Chuỗi: so sánh không phân biệt hoa thường.
        let s = text(Hive::Hkcu, "Software\\Microsoft\\GameBar", "X", "High");
        assert_eq!(
            action_state(&s, &ReadValue::Text("high".into())),
            ActionState::Matched
        );
        assert_eq!(
            action_state(&s, &ReadValue::Missing),
            ActionState::NotMatched
        );

        // Service không tồn tại -> bỏ qua, không lỗi.
        let svc = service("WerSvc", StartType::Disabled);
        assert_eq!(
            action_state(&svc, &ReadValue::Missing),
            ActionState::Skipped
        );
        assert_eq!(
            action_state(&svc, &ReadValue::Dword(4)),
            ActionState::Matched
        );
        assert_eq!(
            action_state(&svc, &ReadValue::Dword(3)),
            ActionState::NotMatched
        );

        // PowerAc: không đọc được chỉ số -> Unknown chứ không coi là chưa bật.
        let p = power_ac("sub", "set", 0);
        assert_eq!(
            action_state(&p, &ReadValue::Dword(0)),
            ActionState::Matched
        );
        assert_eq!(
            action_state(&p, &ReadValue::Unknown),
            ActionState::Unknown
        );

        // BCD: chưa có mục -> chưa bật.
        let b = Action::Bcd {
            name: "hypervisorlaunchtype",
            value: "off",
        };
        assert_eq!(
            action_state(&b, &ReadValue::Text("Off".into())),
            ActionState::Matched
        );
        assert_eq!(
            action_state(&b, &ReadValue::Missing),
            ActionState::NotMatched
        );

        // Nén bộ nhớ.
        let m = Action::MemoryCompression(true);
        assert_eq!(
            action_state(&m, &ReadValue::Flag(true)),
            ActionState::Matched
        );
        assert_eq!(
            action_state(&m, &ReadValue::Flag(false)),
            ActionState::NotMatched
        );

        // Loại trừ Defender: đủ thư mục mới coi là đã bật (bỏ `\` cuối).
        let d = Action::DefenderExclusions(vec!["C:\\Games".into(), "D:\\Game 2".into()]);
        let cur = ReadValue::Multi(vec!["c:\\games\\".into(), "D:\\Game 2".into()]);
        assert_eq!(action_state(&d, &cur), ActionState::Matched);
        let thieu = ReadValue::Multi(vec!["C:\\Games".into()]);
        assert_eq!(action_state(&d, &thieu), ActionState::NotMatched);
        assert_eq!(
            action_state(&d, &ReadValue::Missing),
            ActionState::NotMatched
        );
    }

    #[test]
    fn gop_trang_thai() {
        use ActionState::*;
        assert_eq!(combine_states(&[Matched, Matched]), TweakState::Applied);
        assert_eq!(combine_states(&[NotMatched, NotMatched]), TweakState::NotApplied);
        assert_eq!(combine_states(&[Matched, NotMatched]), TweakState::Partial);
        // Service không có trên máy -> vẫn coi là đã bật nếu phần còn lại đúng.
        assert_eq!(combine_states(&[Matched, Skipped]), TweakState::Applied);
        assert_eq!(combine_states(&[NotMatched, Skipped]), TweakState::NotApplied);
        assert_eq!(combine_states(&[Unknown, Matched]), TweakState::Partial);
        assert_eq!(combine_states(&[Unknown, NotMatched]), TweakState::Partial);
        assert!(matches!(
            combine_states(&[Unknown]),
            TweakState::Unknown(_)
        ));
        assert!(matches!(
            combine_states(&[Skipped]),
            TweakState::Unknown(_)
        ));
        assert!(matches!(combine_states(&[]), TweakState::Unknown(_)));
    }

    #[test]
    fn luu_gia_tri_goc_kem_khong_ton_tai() {
        // Giá trị chưa từng có -> Missing; service thiếu -> Skipped.
        assert_eq!(
            saved_from_read(&hags(), ReadValue::Missing),
            SavedValue::Missing
        );
        assert_eq!(
            saved_from_read(&hags(), ReadValue::Dword(1)),
            SavedValue::Dword(1)
        );
        assert_eq!(
            saved_from_read(&service("Fax", StartType::Disabled), ReadValue::Missing),
            SavedValue::Skipped
        );
        assert_eq!(
            saved_from_read(&hags(), ReadValue::Unknown),
            SavedValue::Unreadable
        );
        assert_eq!(
            saved_from_read(&text(Hive::Hkcu, "a", "b", "c"), ReadValue::Text("x".into())),
            SavedValue::Text("x".into())
        );
    }

    #[test]
    fn backup_serde_round_trip() {
        let mut b = Backup::new();
        b.entries.insert(
            "hags".to_string(),
            BackupEntry {
                tweak_id: "hags".to_string(),
                saved_at: 1_760_000_000,
                originals: vec![SavedValue::Missing],
            },
        );
        b.entries.insert(
            "mmcss_games".to_string(),
            BackupEntry {
                tweak_id: "mmcss_games".to_string(),
                saved_at: 0,
                originals: vec![
                    SavedValue::Dword(20),
                    SavedValue::Missing,
                    SavedValue::Skipped,
                    SavedValue::Text("High".into()),
                    SavedValue::Multi(vec!["C:\\pagefile.sys 16384 24576".into()]),
                    SavedValue::Flag(false),
                    SavedValue::Unreadable,
                ],
            },
        );
        let json = serde_json::to_string(&b).unwrap();
        let back: Backup = serde_json::from_str(&json).unwrap();
        assert_eq!(b, back);
        // Giá trị "không tồn tại" phải sống sót qua vòng JSON.
        assert_eq!(back.entries["hags"].originals[0], SavedValue::Missing);
        assert!(json.contains("\"Missing\""));
        // File rỗng/hỏng -> bản rỗng, không panic.
        let empty: Backup = serde_json::from_str("{}").unwrap_or_default();
        assert!(empty.entries.is_empty());
    }

    #[test]
    fn danh_sach_tinh_chinh_hop_le() {
        let dirs = vec!["C:\\Games\\Delta Force".to_string()];
        let list = all_tweaks(&dirs);
        // Id không trùng.
        let mut ids: Vec<&str> = list.iter().map(|t| t.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "id tinh chỉnh bị trùng");
        // Mọi tinh chỉnh có ≥ 1 thao tác và có nhãn.
        for t in &list {
            assert!(!t.actions.is_empty(), "{} không có thao tác nào", t.id);
            assert!(!t.title.trim().is_empty(), "{} thiếu tiêu đề", t.id);
            assert!(!t.desc.trim().is_empty(), "{} thiếu mô tả", t.id);
        }
        assert!(list.iter().any(|t| t.id == "defender_games"));
        assert!(list.iter().any(|t| t.id == "hypervisor_off"));

        // Không có thư mục game -> không tạo tinh chỉnh Defender.
        let khong_co = all_tweaks(&[]);
        assert!(!khong_co.iter().any(|t| t.id == "defender_games"));
        assert_eq!(khong_co.len(), list.len() - 1);
        // Chỉ toàn chuỗi rỗng cũng coi như rỗng.
        let rong = all_tweaks(&["   ".to_string(), String::new()]);
        assert!(!rong.iter().any(|t| t.id == "defender_games"));
    }

    #[test]
    fn dem_va_danh_sach_de_xuat() {
        let list = all_tweaks(&[]);
        let states: Vec<TweakState> = list
            .iter()
            .map(|t| {
                if t.id == "hags" {
                    TweakState::Applied
                } else {
                    TweakState::NotApplied
                }
            })
            .collect();
        assert_eq!(summary(&list, &states), (1, list.len()));

        let rec = recommended_ids(&list);
        assert!(rec.contains(&"game_mode"));
        assert!(!rec.contains(&"hypervisor_off"));
        assert!(!rec.contains(&"pagefile_fixed"));
        // Mọi id đề xuất phải có thật trong danh sách.
        for id in &rec {
            assert!(list.iter().any(|t| t.id == *id));
        }
    }

    #[test]
    fn giai_ma_output_windows() {
        // "abc" ở UTF-16 LE kèm BOM.
        let utf16 = [0xFF, 0xFE, b'a', 0, b'b', 0, b'c', 0];
        assert_eq!(decode_win_output(&utf16), "abc");
        // UTF-8 thường.
        assert_eq!(decode_win_output(b"Off\r\n"), "Off\r\n");
    }

    #[test]
    fn hoan_tac_khong_co_backup_thi_bao_loi() {
        let list = all_tweaks(&[]);
        let t = list.iter().find(|t| t.id == "hags").unwrap();
        let mut backup = Backup::new();
        let err = revert(t, &mut backup).unwrap_err();
        assert!(err.contains("Không có bản sao lưu"), "lỗi lạ: {}", err);
    }
}
