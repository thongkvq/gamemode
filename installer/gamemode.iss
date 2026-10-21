; Bộ cài GameMode (Inno Setup 6).
; Build: cargo build --release, rồi
;   "%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" installer\gamemode.iss
; Kết quả: dist\GameMode-Setup-<version>.exe

#define AppName "GameMode"
#define AppVersion "1.2.0"
#define AppPublisher "thongkvq"
#define AppExe "GameMode.exe"

[Setup]
AppId={{6F1D2C7A-4B8E-4E57-9A3D-2C5B7E9F1A40}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL=https://github.com/thongkvq/gamemode
AppSupportURL=https://github.com/thongkvq/gamemode
; Cài theo từng user: không cần quyền admin để cài, và app ghi được gamemode.cfg / session.json cạnh exe.
; Bản thân GameMode.exe vẫn tự xin quyền admin khi chạy (manifest highestAvailable).
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\{#AppExe}
SetupIconFile=..\icon.ico
OutputDir=..\dist
OutputBaseFilename=GameMode-Setup-{#AppVersion}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
CloseApplications=force
; Mutex mà GameMode tạo khi chạy (chống mở 2 bản): bộ cài/gỡ cài nhận ra app đang chạy dưới khay
; và nhắc người dùng thoát (app chạy admin nên bộ cài không tự đóng được).
AppMutex=Local\GameModeTacticalRamPurge
VersionInfoVersion={#AppVersion}
VersionInfoDescription={#AppName} Setup

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
english.DesktopIconVN=Tạo shortcut ngoài Desktop
english.RunAppVN=Mở GameMode ngay

[Tasks]
Name: "desktopicon"; Description: "{cm:DesktopIconVN}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "..\target\release\gamemode.exe"; DestDir: "{app}"; DestName: "{#AppExe}"; Flags: ignoreversion
Source: "..\assets\fonts\OFL-ChakraPetch.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\assets\fonts\OFL-JetBrainsMono.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\Gỡ cài đặt {#AppName}"; Filename: "{uninstallexe}"
Name: "{userdesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
; shellexec để Windows đọc manifest và hỏi quyền admin như khi mở bằng tay.
Filename: "{app}\{#AppExe}"; Description: "{cm:RunAppVN}"; Flags: postinstall nowait skipifsilent shellexec

[UninstallRun]
; Xoá task "chạy cùng Windows" nếu người dùng đã bật (bỏ qua nếu không có hoặc không đủ quyền).
Filename: "{sys}\schtasks.exe"; Parameters: "/Delete /TN ""GameMode"" /F"; Flags: runhidden; RunOnceId: "DelTask"

[UninstallDelete]
Type: files; Name: "{app}\gamemode.cfg"
Type: files; Name: "{app}\session.json"
Type: files; Name: "{app}\session.last.json"
Type: files; Name: "{app}\gamemode-crash.log"
Type: files; Name: "{app}\*.txt"
Type: dirifempty; Name: "{app}"
