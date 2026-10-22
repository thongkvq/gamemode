// Nhúng icon + manifest vào exe Windows.
//
// Trên Windows MSVC (bản build thật của tool) hành vi giữ nguyên như cũ: winres tự
// tìm rc.exe có sẵn trong Windows SDK.
// Với target GNU (x86_64-pc-windows-gnu) thì cần `windres` của MinGW; nếu máy build
// không có (ví dụ chỉ `cargo check` trên Linux) thì bỏ qua bước nhúng resource thay vì
// làm hỏng cả lần build.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() != "windows" {
        return;
    }

    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if env == "gnu" && !has_windres() {
        println!(
            "cargo:warning=Bỏ qua nhúng icon/manifest: không tìm thấy windres của MinGW (chỉ cần khi build exe thật)."
        );
        return;
    }

    let mut res = winres::WindowsResource::new();
    res.set_icon("icon.ico");
    // Tên hiển thị trong Task Manager / Properties của exe (tên file vẫn là GameMode.exe
    // để OTA, gỡ cài, task Scheduler và mutex một-bản cũ vẫn nhận ra bản cũ).
    res.set("ProductName", "Paroda GameMode");
    res.set("FileDescription", "Paroda GameMode");
    res.set_manifest(r#"
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
<trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
        <requestedPrivileges>
            <requestedExecutionLevel level="highestAvailable" uiAccess="false" />
        </requestedPrivileges>
    </security>
</trustInfo>
</assembly>
"#);
    res.compile().unwrap();
}

/// Tìm `x86_64-w64-mingw32-windres` (hoặc `windres`) trong PATH.
fn has_windres() -> bool {
    let path = std::env::var("PATH").unwrap_or_default();
    let candidates: &[&str] = if cfg!(target_pointer_width = "64") {
        &["x86_64-w64-mingw32-windres", "windres"]
    } else {
        &["i686-w64-mingw32-windres", "windres"]
    };
    for dir in std::env::split_paths(&path) {
        for c in candidates {
            let p = dir.join(if cfg!(windows) { format!("{c}.exe") } else { c.to_string() });
            if p.is_file() {
                return true;
            }
        }
    }
    false
}
