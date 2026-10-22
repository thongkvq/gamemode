//! Chẩn đoán phần cứng + gợi ý nâng cấp cho card "Phần cứng" (tab "Tối ưu Win").
//!
//! Cách dùng từ `main.rs`/`ui.rs`:
//! ```ignore
//! let hw = std::thread::spawn(crate::hwinfo::collect);   // CHẶN vài giây -> phải chạy nền
//! let advices = crate::hwinfo::advice(&hw);
//! let line = crate::hwinfo::summary_line(&hw);
//! ```
//!
//! - `collect()` chạy ĐÚNG MỘT lần `powershell -NoProfile -Command <SCRIPT>` ẩn cửa sổ; script
//!   trả MỘT JSON (`ConvertTo-Json -Depth 4 -Compress`). Mỗi phần trong script bọc try/catch nên
//!   thiếu một phần (ví dụ máy bàn không có pin) không làm hỏng cả JSON.
//! - `parse` / `advice` / `advice_at` / `summary_line` THUẦN: không đọc đĩa, không WinAPI ->
//!   unit test chạy được cả trên Linux.
//! - Module tự đứng: KHÔNG `use crate::...`; phần WinAPI nằm sau `#[cfg(windows)]`.

use serde::{Deserialize, Serialize};

const GIB: u64 = 1024 * 1024 * 1024;

/// Script PowerShell chạy MỘT lần cho mỗi lần `collect()`.
///
/// Chú ý: script chỉ dùng nháy ĐƠN (không có ký tự `"`) để không phải lo việc escape khi
/// truyền qua argv của Windows; chỉ có nháy đơn bọc ngoài do Rust thêm vào.
pub const SCRIPT: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$out = [ordered]@{}

try {
  $cs = Get-CimInstance Win32_ComputerSystem
  $out.machine = [ordered]@{ Manufacturer = [string]$cs.Manufacturer; Model = [string]$cs.Model; HypervisorPresent = [bool]$cs.HypervisorPresent }
} catch { $out.machine = $null }

try {
  $p = Get-CimInstance Win32_Processor | Select-Object -First 1
  $out.cpu = [ordered]@{ Name = [string]$p.Name; NumberOfCores = [int]$p.NumberOfCores; NumberOfLogicalProcessors = [int]$p.NumberOfLogicalProcessors; MaxClockSpeed = [int]$p.MaxClockSpeed }
} catch { $out.cpu = $null }

try {
  $out.memory = @(Get-CimInstance Win32_PhysicalMemory | ForEach-Object {
    [ordered]@{ BankLabel = [string]$_.BankLabel; DeviceLocator = [string]$_.DeviceLocator; Capacity = [uint64]$_.Capacity; Speed = [int]$_.Speed; ConfiguredClockSpeed = [int]$_.ConfiguredClockSpeed; PartNumber = ([string]$_.PartNumber).Trim() }
  })
} catch { $out.memory = @() }

try {
  $arr = Get-CimInstance Win32_PhysicalMemoryArray | Select-Object -First 1
  $out.memorySlots = [int]$arr.MemoryDevices
} catch { $out.memorySlots = $null }

try {
  $out.gpus = @(Get-CimInstance Win32_VideoController | ForEach-Object {
    $d = $null
    if ($_.DriverDate) { $d = ([datetime]$_.DriverDate).ToString('yyyy-MM-dd') }
    [ordered]@{ Name = [string]$_.Name; DriverVersion = [string]$_.DriverVersion; DriverDate = $d; PNPDeviceID = [string]$_.PNPDeviceID; CurrentRefreshRate = [int]$_.CurrentRefreshRate; CurrentHorizontalResolution = [int]$_.CurrentHorizontalResolution; CurrentVerticalResolution = [int]$_.CurrentVerticalResolution }
  })
} catch { $out.gpus = @() }

try {
  $out.disks = @(Get-PhysicalDisk | ForEach-Object {
    [ordered]@{ FriendlyName = [string]$_.FriendlyName; MediaType = [string]$_.MediaType; BusType = [string]$_.BusType; Size = [uint64]$_.Size }
  })
} catch { $out.disks = @() }

try {
  $sys = $env:SystemDrive
  if (-not $sys) { $sys = 'C:' }
  $di = New-Object System.IO.DriveInfo($sys)
  $out.systemDrive = [ordered]@{ Name = [string]$di.Name; FreeBytes = [uint64]$di.AvailableFreeSpace; TotalBytes = [uint64]$di.TotalSize }
} catch { $out.systemDrive = $null }

try {
  $b = Get-CimInstance -Namespace root/wmi -ClassName BatteryStatus | Select-Object -First 1
  if ($b) { $out.battery = [ordered]@{ PowerOnline = [bool]$b.PowerOnline } } else { $out.battery = $null }
} catch { $out.battery = $null }

try {
  $agent = Get-MMAgent
  $out.mmAgent = [ordered]@{ MemoryCompression = [bool]$agent.MemoryCompression }
} catch { $out.mmAgent = $null }

try {
  $dg = Get-CimInstance Win32_DeviceGuard
  $out.deviceGuard = [ordered]@{ VirtualizationBasedSecurityStatus = [int]$dg.VirtualizationBasedSecurityStatus }
} catch { $out.deviceGuard = $null }

try {
  $out.powerScheme = [string]((powercfg /getactivescheme) -join ' ')
} catch { $out.powerScheme = $null }

$out | ConvertTo-Json -Depth 4 -Compress
"#;

// ---------------------------------------------------------------------------
// Bộ chuyển đổi chịu lỗi: PowerShell lúc trả số, lúc trả chuỗi, lúc trả một object
// thay vì mảng (mảng 1 phần tử bị bung ra). Thiếu field nào cũng không được lỗi.
// ---------------------------------------------------------------------------

/// Nhận `"169"`, `169`, `null`, `""` -> `Option<u64>`.
fn de_u64_opt<'de, D>(d: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde_json::Value;
    Ok(match Option::<Value>::deserialize(d)? {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) => n.as_u64().or_else(|| n.as_f64().map(|f| f as u64)),
        Some(Value::String(s)) => {
            let t = s.trim();
            t.parse::<u64>().ok().or_else(|| t.parse::<f64>().ok().map(|f| f as u64))
        }
        Some(Value::Bool(b)) => Some(b as u64),
        _ => None,
    })
}

/// Như `de_u64_opt` nhưng 0 = "không rõ".
fn de_u64<'de, D>(d: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(de_u64_opt(d)?.unwrap_or(0))
}

/// Nhận `"Foo"`, `12`, `true`, `null`, `"  "` -> `Option<String>` (chuỗi rỗng coi như thiếu).
fn de_str_opt<'de, D>(d: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde_json::Value;
    let v = match Option::<Value>::deserialize(d)? {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    };
    Ok(v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
}

/// Nhận `true/false/1/0/"true"/"False"` -> bool; thiếu thì false.
fn de_bool<'de, D>(d: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(de_bool_opt(d)?.unwrap_or(false))
}

/// Như `de_bool` nhưng giữ được "không có dữ liệu" (`None`) — cần cho `PowerOnline`.
fn de_bool_opt<'de, D>(d: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde_json::Value;
    Ok(match Option::<Value>::deserialize(d)? {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(b),
        Some(Value::Number(n)) => Some(n.as_f64().map(|f| f != 0.0).unwrap_or(false)),
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "" => None,
            "true" | "1" | "yes" | "có" => Some(true),
            "false" | "0" | "no" | "không" => Some(false),
            _ => None,
        },
        _ => None,
    })
}

/// Một object hoặc một mảng object đều nhận; `null`/thiếu -> danh sách rỗng.
fn one_or_many<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany<T> {
        Many(Vec<T>),
        One(T),
    }
    Ok(match Option::<OneOrMany<T>>::deserialize(d)? {
        None => Vec::new(),
        Some(OneOrMany::Many(v)) => v,
        Some(OneOrMany::One(x)) => vec![x],
    })
}

// ---------------------------------------------------------------------------
// Kiểu dữ liệu (khớp đúng tên field trong SCRIPT ở trên).
// ---------------------------------------------------------------------------

/// Toàn bộ số liệu phần cứng đọc được. Field nào thiếu cũng không lỗi khi parse.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HwInfo {
    pub machine: Option<Machine>,
    pub cpu: Option<Cpu>,
    #[serde(deserialize_with = "one_or_many")]
    pub memory: Vec<MemModule>,
    /// Số khe RAM (`Win32_PhysicalMemoryArray.MemoryDevices`); 0/None = không rõ.
    #[serde(deserialize_with = "de_u64_opt")]
    pub memory_slots: Option<u64>,
    #[serde(deserialize_with = "one_or_many")]
    pub gpus: Vec<Gpu>,
    #[serde(deserialize_with = "one_or_many")]
    pub disks: Vec<Disk>,
    pub system_drive: Option<Drive>,
    pub battery: Option<Battery>,
    pub mm_agent: Option<MmAgent>,
    pub device_guard: Option<DeviceGuard>,
    /// Dòng `powercfg /getactivescheme` (nguyên văn, có GUID).
    pub power_scheme: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Machine {
    #[serde(deserialize_with = "de_str_opt")]
    pub manufacturer: Option<String>,
    #[serde(deserialize_with = "de_str_opt")]
    pub model: Option<String>,
    #[serde(deserialize_with = "de_bool")]
    pub hypervisor_present: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Cpu {
    #[serde(deserialize_with = "de_str_opt")]
    pub name: Option<String>,
    #[serde(deserialize_with = "de_u64")]
    pub number_of_cores: u64,
    #[serde(deserialize_with = "de_u64")]
    pub number_of_logical_processors: u64,
    #[serde(deserialize_with = "de_u64")]
    pub max_clock_speed: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct MemModule {
    #[serde(deserialize_with = "de_str_opt")]
    pub bank_label: Option<String>,
    #[serde(deserialize_with = "de_str_opt")]
    pub device_locator: Option<String>,
    #[serde(deserialize_with = "de_u64")]
    pub capacity: u64,
    /// Bus định mức của thanh (MT/s).
    #[serde(deserialize_with = "de_u64")]
    pub speed: u64,
    /// Bus thực tế đang chạy (MT/s).
    #[serde(deserialize_with = "de_u64")]
    pub configured_clock_speed: u64,
    #[serde(deserialize_with = "de_str_opt")]
    pub part_number: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Gpu {
    #[serde(deserialize_with = "de_str_opt")]
    pub name: Option<String>,
    #[serde(deserialize_with = "de_str_opt")]
    pub driver_version: Option<String>,
    /// `yyyy-MM-dd` (script đã format lại từ datetime của WMI).
    #[serde(deserialize_with = "de_str_opt")]
    pub driver_date: Option<String>,
    #[serde(rename = "PNPDeviceID", alias = "PnpDeviceId", deserialize_with = "de_str_opt")]
    pub pnp_device_id: Option<String>,
    #[serde(deserialize_with = "de_u64")]
    pub current_refresh_rate: u64,
    #[serde(deserialize_with = "de_u64")]
    pub current_horizontal_resolution: u64,
    #[serde(deserialize_with = "de_u64")]
    pub current_vertical_resolution: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Disk {
    #[serde(deserialize_with = "de_str_opt")]
    pub friendly_name: Option<String>,
    #[serde(deserialize_with = "de_str_opt")]
    pub media_type: Option<String>,
    #[serde(deserialize_with = "de_str_opt")]
    pub bus_type: Option<String>,
    #[serde(deserialize_with = "de_u64")]
    pub size: u64,
}

/// Ổ chứa Windows (`$env:SystemDrive`, thường là C:).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Drive {
    #[serde(deserialize_with = "de_str_opt")]
    pub name: Option<String>,
    #[serde(deserialize_with = "de_u64")]
    pub free_bytes: u64,
    #[serde(deserialize_with = "de_u64")]
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Battery {
    /// true = đang cắm sạc. `None` = máy không có pin / không đọc được.
    #[serde(deserialize_with = "de_bool_opt")]
    pub power_online: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct MmAgent {
    #[serde(deserialize_with = "de_bool_opt")]
    pub memory_compression: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct DeviceGuard {
    /// 0 = tắt, 1 = bật nhưng chưa chạy, 2 = đang chạy (VBS).
    #[serde(deserialize_with = "de_u64")]
    pub virtualization_based_security_status: u64,
}

// ---------------------------------------------------------------------------
// Thu thập
// ---------------------------------------------------------------------------

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Chạy `SCRIPT` bằng PowerShell ẩn cửa sổ rồi parse JSON.
///
/// CHẶN vài giây (WMI/CIM) -> main phải gọi trong thread nền, không gọi trên thread UI.
#[cfg(windows)]
pub fn collect() -> Result<HwInfo, String> {
    use std::os::windows::process::CommandExt;

    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            SCRIPT,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("chạy powershell lỗi: {}", e))?;

    let text = decode_win_output(&out.stdout);
    if text.trim().is_empty() {
        let err = decode_win_output(&out.stderr);
        let err = err.trim();
        return Err(if err.is_empty() {
            format!("powershell không trả dữ liệu (mã {})", out.status)
        } else {
            format!("powershell lỗi: {}", err)
        });
    }
    parse(&text)
}

/// Trên Linux/macOS không có PowerShell/WMI.
#[cfg(not(windows))]
pub fn collect() -> Result<HwInfo, String> {
    Err("chỉ đọc được phần cứng trên Windows".to_string())
}

/// `schtasks` trả UTF-16 LE có BOM, còn `[Console]::OutputEncoding = UTF8` trả UTF-8:
/// nhận cả hai để không sinh chuỗi rác kiểu `C\0:\0\\0`.
#[cfg(windows)]
fn decode_win_output(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// Parse JSON của `SCRIPT` (thuần, test được trên Linux).
///
/// Khoan dung: bỏ BOM, cắt lấy đoạn từ `{` đầu tới `}` cuối (PowerShell có thể in cảnh báo
/// trước JSON), và mọi field đều có mặc định.
pub fn parse(json: &str) -> Result<HwInfo, String> {
    let s = json.trim_start_matches('\u{feff}').trim();
    let (start, end) = match (s.find('{'), s.rfind('}')) {
        (Some(a), Some(b)) if b > a => (a, b + 1),
        _ => return Err("không thấy JSON trong kết quả PowerShell".to_string()),
    };
    serde_json::from_str::<HwInfo>(&s[start..end])
        .map_err(|e| format!("JSON phần cứng không hợp lệ: {}", e))
}

// ---------------------------------------------------------------------------
// Ngày (không dùng crate ngoài; đủ chính xác cho việc so tuổi driver)
// ---------------------------------------------------------------------------

/// Ngày dương lịch tối thiểu. `Date::today()` lấy theo UTC (lệch tối đa 1 ngày, không ảnh
/// hưởng ngưỡng 2 năm của luật driver).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Date {
    pub fn new(year: i32, month: u32, day: u32) -> Self {
        Self { year, month, day }
    }

    /// Hôm nay theo UTC.
    pub fn today() -> Self {
        let days = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| (d.as_secs() / 86_400) as i64)
            .unwrap_or(0);
        Self::from_days(days)
    }

    /// Đọc `2021-08-25`, `2021-08-25T00:00:00.0000000+07:00` hay `2021/08/25`.
    pub fn parse(s: &str) -> Option<Self> {
        // Gom 3 nhóm chữ số đầu tiên: WMI có thể trả kèm giờ/múi giờ ngay sau ngày.
        let mut nums: Vec<String> = Vec::new();
        let mut cur = String::new();
        for c in s.trim().chars() {
            if c.is_ascii_digit() {
                cur.push(c);
            } else if !cur.is_empty() {
                nums.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            nums.push(cur);
        }
        if nums.len() < 3 {
            return None;
        }
        let y: i32 = nums[0].parse().ok()?;
        let m: u32 = nums[1].parse().ok()?;
        let d: u32 = nums[2].parse().ok()?;
        if !(1980..=2200).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        Some(Self::new(y, m, d))
    }

    /// Số ngày kể từ 1970-01-01 (thuật toán days_from_civil của Howard Hinnant).
    pub fn days_since_epoch(self) -> i64 {
        let y = self.year as i64 - if self.month <= 2 { 1 } else { 0 };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let m = self.month as i64;
        let d = self.day as i64;
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    fn from_days(days: i64) -> Self {
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = mp + if mp < 10 { 3 } else { -9 };
        Self::new((y + if m <= 2 { 1 } else { 0 }) as i32, m as u32, d as u32)
    }
}

// ---------------------------------------------------------------------------
// Gợi ý
// ---------------------------------------------------------------------------

/// Mức độ của một gợi ý; thứ tự `Good < Info < Warn` cũng là thứ tự ưu tiên khi sắp xếp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Level {
    Good,
    Info,
    Warn,
}

#[derive(Debug, Clone, Serialize)]
pub struct Advice {
    pub level: Level,
    pub title: String,
    pub detail: String,
}

fn adv(level: Level, title: impl Into<String>, detail: impl Into<String>) -> Advice {
    Advice { level, title: title.into(), detail: detail.into() }
}

/// Tuổi driver tối đa trước khi bị coi là quá cũ (~2 năm).
const DRIVER_MAX_AGE_DAYS: i64 = 730;
/// Ổ C dưới ngưỡng này (hoặc dưới 30 GB) là Warn.
const DISK_MIN_FREE_PCT: u64 = 15;
const DISK_MIN_FREE_BYTES: u64 = 30 * GIB;
/// Trên ngưỡng Warn nhưng vẫn dưới mức này thì nhắc nhẹ bằng Info.
const DISK_TIGHT_PCT: u64 = 25;

/// Gợi ý cho hôm nay.
pub fn advice(hw: &HwInfo) -> Vec<Advice> {
    advice_at(hw, Date::today())
}

/// Gợi ý với mốc ngày cho trước (thuần -> test được luật "driver cũ hơn 2 năm").
pub fn advice_at(hw: &HwInfo, today: Date) -> Vec<Advice> {
    let mut out = Vec::new();
    ram_rules(hw, &mut out);
    gpu_rules(hw, today, &mut out);
    display_rules(hw, &mut out);
    disk_rules(hw, &mut out);
    system_rules(hw, &mut out);
    power_rules(hw, &mut out);
    // Warn lên đầu; sort ổn định nên trong cùng mức vẫn giữ thứ tự luật.
    out.sort_by_key(|a| std::cmp::Reverse(a.level));
    out
}

pub fn total_ram_bytes(hw: &HwInfo) -> u64 {
    hw.memory.iter().map(|m| m.capacity).sum()
}

/// Tổng RAM làm tròn LÊN theo GiB (16 GiB -> 16, 15.5 GiB -> 16).
pub fn total_ram_gib(hw: &HwInfo) -> u64 {
    total_ram_bytes(hw).div_ceil(GIB)
}

/// Kênh của một thanh: lấy theo `BankLabel`, không có thì theo `DeviceLocator`.
/// `"P0 CHANNEL A"` -> `"CHANNEL A"`, `"ChannelA-DIMM0"` -> `"CHANNEL A"`.
fn channel_key(m: &MemModule) -> Option<String> {
    let src = m.bank_label.as_deref().or(m.device_locator.as_deref())?;
    let up = src.to_ascii_uppercase();
    let idx = up.find("CHANNEL")?;
    let rest = &up[idx + "CHANNEL".len()..];
    let ch = rest.chars().find(|c| c.is_ascii_alphanumeric());
    Some(match ch {
        Some(c) => format!("CHANNEL {}", c),
        None => "CHANNEL".to_string(),
    })
}

/// Mọi thanh cùng nằm trên một kênh (dù có 2 thanh).
fn all_same_channel(mods: &[MemModule]) -> bool {
    let keys: Vec<String> = mods.iter().filter_map(channel_key).collect();
    !keys.is_empty() && keys.len() == mods.len() && keys.iter().all(|k| *k == keys[0])
}

fn ram_rules(hw: &HwInfo, out: &mut Vec<Advice>) {
    let n = hw.memory.len();
    let slots = hw.memory_slots.unwrap_or(0);
    let total = total_ram_bytes(hw);

    if n > 0 {
        if (n == 1 && slots >= 2) || (n >= 2 && all_same_channel(&hw.memory)) {
            out.push(adv(Level::Warn, "RAM đang chạy kênh đơn", single_channel_detail(hw, slots)));
        } else if n >= 2 {
            let list: Vec<String> = hw
                .memory
                .iter()
                .map(|m| {
                    let where_ = m
                        .bank_label
                        .clone()
                        .or_else(|| m.device_locator.clone())
                        .unwrap_or_else(|| "?".to_string());
                    format!("{} ({})", gb_text(m.capacity), where_)
                })
                .collect();
            out.push(adv(
                Level::Good,
                "RAM đang chạy 2 kênh",
                format!(
                    "{} thanh: {} — đúng cấu hình dual channel, băng thông RAM gấp đôi nên iGPU và CPU \
                     phát huy hết, FPS 1% low ổn định hơn.",
                    n,
                    list.join(", ")
                ),
            ));
        } else {
            // 1 thanh nhưng không đọc được số khe -> chưa dám kết luận kênh đơn.
            out.push(adv(
                Level::Info,
                "Máy chỉ có 1 thanh RAM",
                format!(
                    "Đang thấy 1 thanh {}{}. Nếu máy còn khe trống, gắn thêm 1 thanh cùng dung lượng và \
                     cùng bus để chạy 2 kênh — kiểm tra số khe trong BIOS hoặc bằng CPU-Z (tab SPD).",
                    gb_text(hw.memory[0].capacity),
                    bus_suffix(hw.memory[0].speed)
                ),
            ));
        }
    }

    for m in &hw.memory {
        if m.speed > 0 && m.configured_clock_speed > 0 && m.configured_clock_speed < m.speed {
            out.push(adv(
                Level::Warn,
                format!(
                    "RAM chạy dưới định mức ({} < {} MT/s)",
                    m.configured_clock_speed, m.speed
                ),
                format!(
                    "Thanh {}{} chỉ đang chạy {} MT/s. Bật XMP/EXPO trong BIOS nếu máy cho phép \
                     (laptop Acer thường khoá mục này), hoặc kiểm tra lại cặp thanh: hai thanh khác bus \
                     sẽ tự hạ xuống mức thấp nhất. Bus thấp làm iGPU và CPU mất băng thông -> FPS tụt.",
                    gb_text(m.capacity),
                    bus_suffix(m.speed),
                    m.configured_clock_speed
                ),
            ));
        }
    }

    let gib = total.div_ceil(GIB);
    if total > 0 && gib <= 16 {
        out.push(adv(
            Level::Info,
            format!("RAM {} GB — cân nhắc lên 32 GB", gib),
            format!(
                "{} GB là đủ cho game nhẹ, nhưng vừa chơi vừa mở Chrome/Discord/OBS thì Windows phải \
                 nén bộ nhớ và ghi page file ra SSD, gây khựng hình. Lên 32 GB (2×16 GB cùng bus) là mức \
                 thoải mái cho game + app nền trong vài năm tới.",
                gib
            ),
        ));
    }
}

fn bus_suffix(speed: u64) -> String {
    if speed > 0 {
        format!(" {}", ddr_label(speed).unwrap_or_else(|| format!("{} MT/s", speed)))
    } else {
        String::new()
    }
}

/// `3200` -> `DDR4-3200`. Suy đoán thế hệ theo bus (Windows không trả SMBIOSMemoryType ở đây):
/// ≤1866 = DDR3, <4400 = DDR4, còn lại DDR5.
fn ddr_label(speed: u64) -> Option<String> {
    if speed == 0 {
        return None;
    }
    let gen = if speed <= 1866 {
        "DDR3"
    } else if speed < 4400 {
        "DDR4"
    } else {
        "DDR5"
    };
    Some(format!("{}-{}", gen, speed))
}

fn single_channel_detail(hw: &HwInfo, slots: u64) -> String {
    let m = &hw.memory[0];
    let where_ = m
        .bank_label
        .clone()
        .or_else(|| m.device_locator.clone())
        .unwrap_or_else(|| "không rõ khe".to_string());
    let part = m.part_number.clone().unwrap_or_else(|| "không rõ".to_string());
    let slot_txt = if slots >= 2 {
        format!("máy có {} khe", slots)
    } else {
        "máy còn khe trống".to_string()
    };
    format!(
        "Đang có {} thanh {}{} cắm ở {} ({}). Gắn thêm 1 thanh cùng dung lượng {} và cùng bus {}{} \
         (PartNumber đang dùng: {}) vào khe còn trống để chạy 2 kênh — mua đúng PartNumber này nếu còn \
         hàng, không thì chọn thanh cùng hãng, cùng dung lượng và cùng bus. Lợi ích: iGPU Radeon lấy RAM \
         làm VRAM nên băng thông tăng gần gấp đôi, CPU có thêm băng thông -> FPS trung bình tăng và nhất \
         là FPS 1% low/min đỡ tụt khi chơi game nặng.",
        hw.memory.len(),
        gb_text(m.capacity),
        bus_suffix(m.speed),
        where_,
        slot_txt,
        gb_text(m.capacity),
        if m.speed > 0 { m.speed.to_string() } else { "?".to_string() },
        if m.speed > 0 { " MT/s" } else { "" },
        part,
    )
}

/// `PCI\VEN_10DE&DEV_2520&...` -> `(Some(0x10DE), Some(0x2520))`.
fn pnp_ids(pnp: &str) -> (Option<u16>, Option<u16>) {
    let up = pnp.to_ascii_uppercase();
    let get = |tag: &str| -> Option<u16> {
        let i = up.find(tag)? + tag.len();
        let hex: String = up[i..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .take(4)
            .collect();
        if hex.len() == 4 {
            u16::from_str_radix(&hex, 16).ok()
        } else {
            None
        }
    };
    (get("VEN_"), get("DEV_"))
}

/// Bảng DEV -> tên card NVIDIA (mở rộng bằng cách thêm dòng vào `match` này).
pub fn nvidia_dev_name(dev: u16) -> Option<&'static str> {
    Some(match dev {
        0x2520 => "GeForce RTX 3060 Laptop GPU",
        0x2560 => "RTX 3060 Laptop GPU",
        0x25A2 => "RTX 3050 Ti Laptop",
        0x2420 => "RTX 3080 Ti Laptop",
        0x249C => "RTX 3080 Laptop",
        0x24DD => "RTX 3070 Ti Laptop",
        0x24E0 => "RTX 3070 Laptop",
        0x25E2 => "RTX 3050 Laptop",
        _ => return None,
    })
}

/// Số model đầu tiên có ≥3 chữ số: `"NVIDIA GeForce GTX 750 Ti"` -> `"750"`.
fn model_number(s: &str) -> Option<String> {
    let mut cur = String::new();
    for c in s.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() {
            cur.push(c);
        } else {
            if cur.len() >= 3 {
                return Some(cur);
            }
            cur.clear();
        }
    }
    None
}

fn family(s: &str) -> Option<&'static str> {
    let up = s.to_ascii_uppercase();
    for f in ["RTX", "GTX", "MX", "QUADRO", "TESLA"] {
        if up.contains(f) {
            return Some(f);
        }
    }
    None
}

fn has_ti(s: &str) -> bool {
    s.to_ascii_uppercase().split(|c: char| !c.is_ascii_alphanumeric()).any(|t| t == "TI")
}

/// Tên driver có khớp model trong bảng không (so số model + họ RTX/GTX + hậu tố Ti).
fn name_matches_model(actual: &str, expected: &str) -> bool {
    match (model_number(actual), model_number(expected)) {
        (Some(a), Some(e)) if a == e => {
            family(actual) == family(expected) && has_ti(actual) == has_ti(expected)
        }
        _ => {
            let a = actual.to_ascii_uppercase();
            let e = expected.to_ascii_uppercase();
            a.contains(&e) || e.contains(&a)
        }
    }
}

fn is_amd_gpu(name: &str) -> bool {
    let up = name.to_ascii_uppercase();
    up.contains("RADEON") || up.contains("AMD")
}

fn is_nvidia_gpu(name: &str) -> bool {
    let up = name.to_ascii_uppercase();
    up.contains("NVIDIA") || up.contains("GEFORCE") || up.contains("RTX") || up.contains("GTX")
}

fn has_amd_gpu(hw: &HwInfo) -> bool {
    hw.gpus
        .iter()
        .any(|g| g.name.as_deref().map(is_amd_gpu).unwrap_or(false))
}

fn gpu_rules(hw: &HwInfo, today: Date, out: &mut Vec<Advice>) {
    let amd_igpu = has_amd_gpu(hw);

    for g in &hw.gpus {
        let actual = g.name.clone().unwrap_or_default();
        let (ven, dev) = pnp_ids(g.pnp_device_id.as_deref().unwrap_or(""));
        if ven == Some(0x10DE) {
            if let Some(expected) = dev.and_then(nvidia_dev_name) {
                if !name_matches_model(&actual, expected) {
                    out.push(adv(
                        Level::Warn,
                        "Driver NVIDIA đang nhận sai card",
                        format!(
                            "Card thật là {} (PCI {:04X}:{:04X}) nhưng driver báo tên \"{}\" (bản {}). \
                             INF của driver đang gán sai model -> sai bảng xung/điện, dễ mất DLSS, bộ mã hoá \
                             NVENC và tụt FPS so với đúng card. Cách sửa: tải driver chính hãng cho {} từ \
                             nvidia.com (hoặc GeForce Experience), khi cài chọn \"Clean install\"; nếu vẫn \
                             sai tên thì gỡ driver cũ bằng DDU trong Safe Mode rồi cài lại.",
                            expected,
                            ven.unwrap_or(0),
                            dev.unwrap_or(0),
                            actual,
                            g.driver_version.clone().unwrap_or_else(|| "?".to_string()),
                            expected
                        ),
                    ));
                }
            }
        }
    }

    let today_days = today.days_since_epoch();
    for g in &hw.gpus {
        let Some(date) = g.driver_date.as_deref().and_then(Date::parse) else {
            continue;
        };
        let age = today_days - date.days_since_epoch();
        if age <= DRIVER_MAX_AGE_DAYS {
            continue;
        }
        let name = g.name.clone().unwrap_or_else(|| "GPU".to_string());
        let years = age as f64 / 365.0;
        let mut how = if is_amd_gpu(&name) {
            "Cài AMD Adrenalin mới nhất từ amd.com (kèm chipset driver cho CPU Ryzen), hoặc bản driver \
             trên trang hỗ trợ của hãng máy cho đúng model."
                .to_string()
        } else if is_nvidia_gpu(&name) {
            "Cài driver NVIDIA mới nhất cho đúng card từ nvidia.com (Game Ready/Studio đều được), chọn \
             \"Clean install\"."
                .to_string()
        } else {
            "Cài driver mới nhất từ trang hỗ trợ của hãng máy/GPU.".to_string()
        };
        if amd_igpu && is_nvidia_gpu(&name) {
            how.push_str(
                " Lưu ý: laptop này có iGPU AMD và card rời NVIDIA, hình ảnh đi qua iGPU trước khi ra màn \
                 hình nên driver iGPU cũ cũng kéo FPS xuống — cập nhật cả driver AMD Adrenalin lẫn driver \
                 NVIDIA.",
            );
        }
        out.push(adv(
            Level::Warn,
            format!("Driver đồ hoạ quá cũ — {} ({})", name, g.driver_date.clone().unwrap_or_default()),
            format!(
                "Driver của {} phát hành {} (cách đây khoảng {:.1} năm), quá cũ so với hôm nay ({:04}-{:02}-{:02}). \
                 Driver cũ thiếu tối ưu cho game mới (shader compile chậm, stutter, mất frame generation). {}",
                name,
                date_str(date),
                years,
                today.year,
                today.month,
                today.day,
                how
            ),
        ));
    }
}

fn display_rules(hw: &HwInfo, out: &mut Vec<Advice>) {
    let Some(g) = hw
        .gpus
        .iter()
        .find(|g| g.current_refresh_rate > 0 && g.current_horizontal_resolution > 0)
    else {
        return;
    };
    let hz = g.current_refresh_rate;
    let res = format!("{}×{}", g.current_horizontal_resolution, g.current_vertical_resolution);
    if hz < 100 {
        out.push(adv(
            Level::Info,
            format!("Màn hình đang chạy {} Hz — thử mức cao hơn", hz),
            format!(
                "Màn {res} hiện ở {hz} Hz. Nếu màn hỗ trợ tần số quét cao hơn (120/144/165 Hz), vào \
                 Settings > System > Display > Advanced display và chọn mức cao nhất, hoặc chỉnh trong \
                 NVIDIA Control Panel / AMD Software. Tần số cao làm hình mượt hơn hẳn khi FPS vượt {hz}."
            ),
        ));
    } else if hz >= 120 {
        out.push(adv(
            Level::Good,
            format!("Màn hình đang chạy {} Hz", hz),
            format!(
                "Màn {res} đang ở {hz} Hz — đã dùng đúng tần số quét cao, không cần chỉnh thêm."
            ),
        ));
    }
}

fn disk_rules(hw: &HwInfo, out: &mut Vec<Advice>) {
    let Some(d) = &hw.system_drive else { return };
    if d.total_bytes == 0 {
        return;
    }
    let pct = d.free_bytes.saturating_mul(100) / d.total_bytes;
    let name = d
        .name
        .clone()
        .map(|n| n.trim_end_matches('\\').to_string())
        .unwrap_or_else(|| "C:".to_string());
    let detail_base = format!(
        "Ổ {name} còn {} trống trên {} ({}%). SSD NVMe gần đầy vừa chậm ghi vừa hết chỗ cho page file và \
         shader cache -> game load lâu, khựng khi vào map mới.",
        gb_text(d.free_bytes),
        gb_text(d.total_bytes),
        pct
    );
    if pct < DISK_MIN_FREE_PCT || d.free_bytes < DISK_MIN_FREE_BYTES {
        out.push(adv(
            Level::Warn,
            format!("Ổ {} gần đầy — chỉ còn {} ({}%)", name, gb_text(d.free_bytes), pct),
            format!(
                "{detail_base} Nên dọn ngay: Disk Cleanup/Storage Sense, xoá file tạm và bộ cài cũ, gỡ game \
                 không chơi (hoặc chuyển sang ổ khác), tắt Hibernate nếu không dùng (powercfg /h off) để lấy \
                 lại vài GB. Mục tiêu: để trống trên {}.",
                DISK_MIN_FREE_PCT
            ),
        ));
    } else if pct < DISK_TIGHT_PCT {
        out.push(adv(
            Level::Info,
            format!("Ổ {} còn {} ({}%)", name, gb_text(d.free_bytes), pct),
            format!(
                "{detail_base} Chưa đến mức báo động nhưng nên dọn bớt để giữ trên {} trống.",
                DISK_TIGHT_PCT
            ),
        ));
    }
}

fn system_rules(hw: &HwInfo, out: &mut Vec<Advice>) {
    let hv = hw.machine.as_ref().map(|m| m.hypervisor_present).unwrap_or(false);
    let vbs = hw
        .device_guard
        .as_ref()
        .map(|d| d.virtualization_based_security_status)
        .unwrap_or(0);
    if hv || vbs == 1 || vbs == 2 {
        let mut detail = String::from(
            "Hypervisor (VirtualMachinePlatform/Hyper-V) hoặc Virtualization Based Security đang chạy: \
             Windows nằm trong lớp ảo hoá nên một số game (nhất là game có chống cheat) mất khoảng 5-15% FPS. \
             Nếu không dùng WSL2, Android subsystem hay Docker thì tắt ở mục \"Hệ thống\" của app rồi khởi \
             động lại máy.",
        );
        if hv {
            detail.push_str(
                " Máy báo HypervisorPresent = True, tức là hypervisor đang thực sự hoạt động.",
            );
        }
        if vbs == 2 {
            detail.push_str(" VBS đang bật và chạy (VirtualizationBasedSecurityStatus = 2).");
        }
        out.push(adv(Level::Info, "Hypervisor/VBS đang chạy", detail));
    }

    let compression = hw.mm_agent.as_ref().and_then(|m| m.memory_compression);
    let gib = total_ram_gib(hw);
    if compression == Some(false) && gib > 0 && gib <= 16 {
        out.push(adv(
            Level::Info,
            "Nén bộ nhớ (Memory Compression) đang tắt",
            format!(
                "RAM chỉ {} GB mà Memory Compression đang tắt, nên khi thiếu RAM Windows phải ghi page file \
                 ra SSD -> giật khi chơi. Bật lại ở mục \"Bộ nhớ\" của app (hoặc chạy \
                 `Enable-MMAgent -MemoryCompression` bằng PowerShell quyền admin) rồi khởi động lại.",
                gib
            ),
        ));
    }
}

fn power_rules(hw: &HwInfo, out: &mut Vec<Advice>) {
    if let Some(b) = &hw.battery {
        if b.power_online == Some(false) {
            out.push(adv(
                Level::Warn,
                "Đang chạy pin: CPU/GPU bị giới hạn công suất",
                "Máy đang không cắm sạc (PowerOnline = False). Ở chế độ pin, CPU và GPU bị hạ xung/giới hạn \
                 TGP (Nitro Sense hạ cả PL1/PL2) nên FPS giảm rất mạnh và có thể tụt xuống dưới 30 FPS. \
                 Cắm sạc trước khi chơi, và đặt sạc ở chế độ hiệu năng cao nếu máy có.",
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Dòng tóm tắt
// ---------------------------------------------------------------------------

fn gb_text(bytes: u64) -> String {
    let g = bytes as f64 / GIB as f64;
    if (g - g.round()).abs() < 0.05 {
        format!("{} GB", g.round() as u64)
    } else {
        format!("{:.1} GB", g)
    }
}

fn date_str(d: Date) -> String {
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

/// Bỏ các nhãn thương hiệu trong tên WMI: `(R)`, `(TM)`, `CPU`, ...
fn clean_marks(raw: &str) -> String {
    let mut s = raw.to_string();
    for m in ["(R)", "(TM)", "(tm)", "(r)", "®", "™", "CPU ", "Corporation "] {
        s = s.replace(m, " ");
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn short_cpu_name(raw: &str) -> String {
    let mut s = clean_marks(raw);
    if let Some(i) = s.find(" with ") {
        s.truncate(i);
    }
    for p in ["AMD ", "Intel ", "Genuine ", "12th Gen ", "13th Gen ", "14th Gen "] {
        if let Some(rest) = s.strip_prefix(p) {
            s = rest.to_string();
        }
    }
    s.trim().to_string()
}

/// `NVIDIA GeForce RTX 3060 Laptop GPU` -> `RTX 3060 Laptop`; `AMD Radeon(TM) Graphics` -> `Radeon`.
fn short_gpu_name(raw: &str) -> String {
    const DROP: [&str; 6] = ["NVIDIA", "AMD", "INTEL", "GEFORCE", "GPU", "GRAPHICS"];
    let cleaned = clean_marks(raw);
    let kept: Vec<&str> = cleaned
        .split_whitespace()
        .filter(|t| !DROP.contains(&t.to_ascii_uppercase().as_str()))
        .collect();
    if kept.is_empty() {
        cleaned
    } else {
        kept.join(" ")
    }
}

/// Một dòng tóm tắt để hiện ở đầu card "Phần cứng".
///
/// Ví dụ: `Acer Nitro AN515-45 · Ryzen 7 5800H 8C/16T · 16 GB DDR4-3200 (1 thanh) ·
/// RTX 3060 Laptop + Radeon`.
pub fn summary_line(hw: &HwInfo) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(m) = &hw.machine {
        let man = m.manufacturer.clone().unwrap_or_default();
        let model = m.model.clone().unwrap_or_default();
        let joined = if model.is_empty() {
            man.trim().to_string()
        } else if man.is_empty() || model.to_ascii_uppercase().starts_with(&man.to_ascii_uppercase()) {
            model.trim().to_string()
        } else {
            format!("{} {}", man.trim(), model.trim())
        };
        if !joined.is_empty() {
            parts.push(joined);
        }
    }

    if let Some(c) = &hw.cpu {
        if let Some(raw) = c.name.as_deref() {
            let name = short_cpu_name(raw);
            let mut s = name;
            if c.number_of_cores > 0 && c.number_of_logical_processors > 0 {
                s.push_str(&format!(" {}C/{}T", c.number_of_cores, c.number_of_logical_processors));
            }
            if !s.trim().is_empty() {
                parts.push(s);
            }
        }
    }

    let total = total_ram_bytes(hw);
    if total > 0 {
        let speed = hw
            .memory
            .iter()
            .map(|m| m.configured_clock_speed.max(m.speed))
            .max()
            .unwrap_or(0);
        parts.push(format!(
            "{} GB{} ({} thanh)",
            total.div_ceil(GIB),
            bus_suffix(speed),
            hw.memory.len()
        ));
    }

    let gpus: Vec<String> = hw
        .gpus
        .iter()
        .filter_map(|g| g.name.as_deref())
        .filter(|n| !n.trim().is_empty())
        .map(short_gpu_name)
        .collect();
    if !gpus.is_empty() {
        parts.push(gpus.join(" + "));
    }

    if parts.is_empty() {
        "Không đọc được thông tin phần cứng".to_string()
    } else {
        parts.join(" · ")
    }
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Máy thật: Acer Nitro AN515-45, Ryzen 7 5800H, RTX 3060 Laptop, 1 thanh 16 GB khe A,
    /// màn 1920×1080 144 Hz, ổ C còn 73 GB, hypervisor bật, memory compression tắt, đang cắm sạc.
    const REAL_JSON: &str = r#"{
      "machine": { "Manufacturer": "Acer", "Model": "Nitro AN515-45", "HypervisorPresent": true },
      "cpu": { "Name": "AMD Ryzen 7 5800H with Radeon Graphics", "NumberOfCores": 8,
               "NumberOfLogicalProcessors": 16, "MaxClockSpeed": 3200 },
      "memory": [ { "BankLabel": "P0 CHANNEL A", "DeviceLocator": "DIMM 0",
                    "Capacity": 17179869184, "Speed": 3200, "ConfiguredClockSpeed": 3200,
                    "PartNumber": "8ATF2G64HZ-3G2E2" } ],
      "memorySlots": 2,
      "gpus": [
        { "Name": "NVIDIA GeForce RTX 3060 Laptop GPU", "DriverVersion": "32.0.15.6094",
          "DriverDate": "2026-01-15",
          "PNPDeviceID": "PCI\\VEN_10DE&DEV_2520&SUBSYS_15431025&REV_A1\\4&1a2b3c4d&0&0008",
          "CurrentRefreshRate": 144, "CurrentHorizontalResolution": 1920, "CurrentVerticalResolution": 1080 },
        { "Name": "AMD Radeon(TM) Graphics", "DriverVersion": "27.20.14048.2",
          "DriverDate": "2021-08-25",
          "PNPDeviceID": "PCI\\VEN_1002&DEV_1638&SUBSYS_15431025&REV_C5\\4&1a2b3c4d&0&0009",
          "CurrentRefreshRate": 144, "CurrentHorizontalResolution": 1920, "CurrentVerticalResolution": 1080 }
      ],
      "disks": [ { "FriendlyName": "Samsung SSD 980 512GB", "MediaType": "SSD",
                   "BusType": "NVMe", "Size": 512110190592 } ],
      "systemDrive": { "Name": "C:\\", "FreeBytes": 78383153152, "TotalBytes": 511000000000 },
      "battery": { "PowerOnline": true },
      "mmAgent": { "MemoryCompression": false },
      "deviceGuard": { "VirtualizationBasedSecurityStatus": 0 },
      "powerScheme": "Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced)"
    }"#;

    const TODAY: Date = Date { year: 2026, month: 10, day: 22 };

    fn find<'a>(v: &'a [Advice], needle: &str) -> Option<&'a Advice> {
        v.iter().find(|a| a.title.contains(needle))
    }

    #[test]
    fn single_channel_la_warn() {
        let hw = parse(REAL_JSON).unwrap();
        assert_eq!(hw.memory.len(), 1);
        assert_eq!(hw.memory_slots, Some(2));
        let v = advice_at(&hw, TODAY);
        let a = find(&v, "kênh đơn").expect("phải có cảnh báo kênh đơn");
        assert_eq!(a.level, Level::Warn);
        // Detail phải in dung lượng, bus và PartNumber để mua đúng loại.
        assert!(a.detail.contains("16 GB"), "{}", a.detail);
        assert!(a.detail.contains("3200"), "{}", a.detail);
        assert!(a.detail.contains("8ATF2G64HZ-3G2E2"), "{}", a.detail);
        assert!(a.detail.contains("CHANNEL A"), "{}", a.detail);
        assert!(v.iter().all(|a| !a.title.contains("2 kênh")), "không được báo Good 2 kênh");
    }

    #[test]
    fn hai_thanh_a_b_la_good() {
        let json = REAL_JSON
            .replace("\"memorySlots\": 2", "\"memorySlots\": 2")
            .replace(
                r#""memory": [ { "BankLabel": "P0 CHANNEL A", "DeviceLocator": "DIMM 0","#,
                r#""memory": [ { "BankLabel": "P0 CHANNEL A", "DeviceLocator": "DIMM 0","#,
            );
        // Thêm thanh thứ hai ở khe B (cùng PartNumber/dung lượng/bus).
        let dual = json.replace(
            r#""PartNumber": "8ATF2G64HZ-3G2E2" } ]"#,
            r#""PartNumber": "8ATF2G64HZ-3G2E2" },
              { "BankLabel": "P0 CHANNEL B", "DeviceLocator": "DIMM 1", "Capacity": 17179869184,
                "Speed": 3200, "ConfiguredClockSpeed": 3200, "PartNumber": "8ATF2G64HZ-3G2E2" } ]"#,
        );
        let hw = parse(&dual).unwrap();
        assert_eq!(hw.memory.len(), 2);
        let v = advice_at(&hw, TODAY);
        assert!(find(&v, "kênh đơn").is_none(), "2 thanh A/B không được báo kênh đơn");
        let g = find(&v, "2 kênh").expect("phải có Good 2 kênh");
        assert_eq!(g.level, Level::Good);
    }

    #[test]
    fn hai_thanh_cung_kenh_a_van_la_warn() {
        let dual = REAL_JSON.replace(
            r#""PartNumber": "8ATF2G64HZ-3G2E2" } ]"#,
            r#""PartNumber": "8ATF2G64HZ-3G2E2" },
              { "BankLabel": "P0 CHANNEL A", "DeviceLocator": "DIMM 1", "Capacity": 17179869184,
                "Speed": 3200, "ConfiguredClockSpeed": 3200, "PartNumber": "8ATF2G64HZ-3G2E2" } ]"#,
        );
        let hw = parse(&dual).unwrap();
        assert!(find(&advice_at(&hw, TODAY), "kênh đơn").is_some());
    }

    #[test]
    fn driver_sai_ten_la_warn() {
        let bad = REAL_JSON.replace("NVIDIA GeForce RTX 3060 Laptop GPU", "NVIDIA GeForce GTX 750 Ti");
        let hw = parse(&bad).unwrap();
        let v = advice_at(&hw, TODAY);
        let a = find(&v, "nhận sai card").expect("driver INF sai phải bị cảnh báo");
        assert_eq!(a.level, Level::Warn);
        assert!(a.detail.contains("RTX 3060 Laptop"), "{}", a.detail);
        assert!(a.detail.contains("GTX 750 Ti"), "{}", a.detail);
        assert!(a.detail.contains("2520"), "{}", a.detail);
        assert!(a.detail.contains("nvidia.com"), "{}", a.detail);
    }

    #[test]
    fn ten_dung_thi_khong_bao_sai_card() {
        let hw = parse(REAL_JSON).unwrap();
        assert!(find(&advice_at(&hw, TODAY), "nhận sai card").is_none());
        // 10DE:2560 cũng là RTX 3060 Laptop.
        assert!(name_matches_model("NVIDIA GeForce RTX 3060 Laptop GPU", "RTX 3060 Laptop GPU"));
        assert!(!name_matches_model("NVIDIA GeForce RTX 3050 Laptop GPU", "RTX 3050 Ti Laptop"));
        assert_eq!(nvidia_dev_name(0x2520), Some("GeForce RTX 3060 Laptop GPU"));
        assert_eq!(nvidia_dev_name(0x2420), Some("RTX 3080 Ti Laptop"));
        assert_eq!(nvidia_dev_name(0x1234), None);
    }

    #[test]
    fn driver_2021_la_qua_cu() {
        let hw = parse(REAL_JSON).unwrap();
        let v = advice_at(&hw, TODAY);
        let a = find(&v, "quá cũ").expect("driver AMD 2021 phải bị cảnh báo");
        assert_eq!(a.level, Level::Warn);
        assert!(a.title.contains("Radeon"), "{}", a.title);
        assert!(a.title.contains("2021-08-25"), "{}", a.title);
        assert!(a.detail.contains("Adrenalin") || a.detail.contains("amd.com"), "{}", a.detail);
        // NVIDIA giả định driver mới (2026-01-15) nên không bị báo cũ.
        assert_eq!(v.iter().filter(|x| x.title.contains("quá cũ")).count(), 1);
        // Cùng dữ liệu nhưng "hôm nay" là 2021-10-01 -> chưa quá 2 năm.
        let early = advice_at(&hw, Date::new(2021, 10, 1));
        assert!(find(&early, "quá cũ").is_none());
    }

    #[test]
    fn laptop_co_igpu_thi_nhac_driver_amd() {
        // Driver NVIDIA cũ + tên sai, máy có iGPU AMD -> nhắc hình đi qua iGPU.
        let bad = REAL_JSON
            .replace("NVIDIA GeForce RTX 3060 Laptop GPU", "NVIDIA GeForce GTX 750 Ti")
            .replace("\"DriverDate\": \"2026-01-15\"", "\"DriverDate\": \"2023-01-10\"");
        let hw = parse(&bad).unwrap();
        let v = advice_at(&hw, TODAY);
        let old = v
            .iter()
            .find(|a| a.title.contains("GTX 750 Ti") && a.title.contains("quá cũ"))
            .expect("driver NVIDIA 2023 phải bị báo cũ");
        assert!(old.detail.contains("iGPU"), "{}", old.detail);
        assert!(old.detail.contains("nvidia.com"), "{}", old.detail);
    }

    #[test]
    fn man_hinh_va_o_cung() {
        let hw = parse(REAL_JSON).unwrap();
        let v = advice_at(&hw, TODAY);
        assert_eq!(find(&v, "144 Hz").map(|a| a.level), Some(Level::Good));
        // 73 GB / 511 GB ~ 15.3% -> chưa tới ngưỡng Warn 15%, chỉ nhắc Info.
        assert!(find(&v, "gần đầy").is_none());
        assert_eq!(find(&v, "Ổ C: còn").map(|a| a.level), Some(Level::Info));

        // Ổ C còn 25 GB -> Warn.
        let tight = REAL_JSON.replace("\"FreeBytes\": 78383153152", "\"FreeBytes\": 26843545600");
        let v2 = advice_at(&parse(&tight).unwrap(), TODAY);
        assert_eq!(find(&v2, "gần đầy").map(|a| a.level), Some(Level::Warn));
    }

    #[test]
    fn hypervisor_va_nen_bo_nho() {
        let hw = parse(REAL_JSON).unwrap();
        let v = advice_at(&hw, TODAY);
        let hv = find(&v, "Hypervisor").expect("hypervisor bật phải có Info");
        assert_eq!(hv.level, Level::Info);
        let mc = find(&v, "Nén bộ nhớ").expect("compression tắt + RAM 16 GB phải có Info");
        assert_eq!(mc.level, Level::Info);
    }

    #[test]
    fn chay_pin_la_warn() {
        let unplugged = REAL_JSON.replace("\"PowerOnline\": true", "\"PowerOnline\": false");
        let v = advice_at(&parse(&unplugged).unwrap(), TODAY);
        let a = find(&v, "chạy pin").expect("không cắm sạc phải Warn");
        assert_eq!(a.level, Level::Warn);
        // Đang cắm sạc thì không có cảnh báo này.
        let hw = parse(REAL_JSON).unwrap();
        assert!(find(&advice_at(&hw, TODAY), "chạy pin").is_none());
    }

    #[test]
    fn ram_chay_duoi_dinh_muc() {
        let slow = REAL_JSON.replace("\"ConfiguredClockSpeed\": 3200", "\"ConfiguredClockSpeed\": 2666");
        let v = advice_at(&parse(&slow).unwrap(), TODAY);
        let a = find(&v, "dưới định mức").expect("2666 < 3200 phải Warn");
        assert_eq!(a.level, Level::Warn);
    }

    #[test]
    fn summary_line_dung_vi_du() {
        let hw = parse(REAL_JSON).unwrap();
        assert_eq!(
            summary_line(&hw),
            "Acer Nitro AN515-45 · Ryzen 7 5800H 8C/16T · 16 GB DDR4-3200 (1 thanh) · \
             RTX 3060 Laptop + Radeon"
        );
    }

    #[test]
    fn json_thieu_field_van_parse() {
        let hw = parse("{}").unwrap();
        assert!(hw.memory.is_empty() && hw.gpus.is_empty());
        assert_eq!(summary_line(&hw), "Không đọc được thông tin phần cứng");
        assert!(advice_at(&hw, TODAY).is_empty());

        let hw2 = parse(r#"{"machine":{"Model":"Nitro AN515-45"}}"#).unwrap();
        assert_eq!(hw2.machine.as_ref().unwrap().model.as_deref(), Some("Nitro AN515-45"));
        assert_eq!(summary_line(&hw2), "Nitro AN515-45");

        // Mảng 1 phần tử bị PowerShell bung thành object + số dạng chuỗi + null.
        let hw3 = parse(
            r#"{"memorySlots":"2","memory":{"Capacity":"17179869184","Speed":"3200","PartNumber":null},
                "systemDrive":null,"battery":null,"gpus":[{"Name":"AMD Radeon(TM) Graphics",
                "CurrentRefreshRate":"60","CurrentHorizontalResolution":1920}]}"#,
        )
        .unwrap();
        assert_eq!(hw3.memory.len(), 1);
        assert_eq!(hw3.memory[0].capacity, 17179869184);
        assert_eq!(hw3.memory_slots, Some(2));
        assert!(hw3.system_drive.is_none() && hw3.battery.is_none());
        let v = advice_at(&hw3, TODAY);
        assert_eq!(find(&v, "kênh đơn").map(|a| a.level), Some(Level::Warn));
        assert_eq!(find(&v, "Hz —").map(|a| a.level), Some(Level::Info));

        // Rác trước JSON (PowerShell in cảnh báo) và BOM vẫn parse được.
        assert!(parse("\u{feff}WARNING: gì đó\n{\"memorySlots\": 4}").is_ok());
        assert!(parse("không phải json").is_err());
    }

    #[test]
    fn ngay_va_so_khe() {
        assert_eq!(Date::parse("2021-08-25").unwrap().days_since_epoch(), 18_864);
        assert_eq!(
            Date::parse("2021-08-25T00:00:00.0000000+07:00").unwrap(),
            Date::new(2021, 8, 25)
        );
        assert!(Date::parse("").is_none());
        assert_eq!(Date::from_days(0), Date::new(1970, 1, 1));
        assert_eq!(Date::new(2026, 10, 22).days_since_epoch() - Date::new(2021, 8, 25).days_since_epoch(), 1884);

        assert_eq!(channel_key(&MemModule { bank_label: Some("P0 CHANNEL A".into()), ..Default::default() }).as_deref(), Some("CHANNEL A"));
        assert_eq!(channel_key(&MemModule { device_locator: Some("ChannelB-DIMM1".into()), ..Default::default() }).as_deref(), Some("CHANNEL B"));
        assert_eq!(channel_key(&MemModule { device_locator: Some("DIMM 0".into()), ..Default::default() }), None);
        assert_eq!(ddr_label(3200).as_deref(), Some("DDR4-3200"));
        assert_eq!(ddr_label(5600).as_deref(), Some("DDR5-5600"));
        assert_eq!(ddr_label(1600).as_deref(), Some("DDR3-1600"));
        assert_eq!(pnp_ids("PCI\\VEN_10DE&DEV_2520&SUBSYS_X").1, Some(0x2520));
        assert_eq!(pnp_ids("PCI\\VEN_1002&DEV_1638").0, Some(0x1002));
        assert_eq!(pnp_ids("").0, None);
    }
}
