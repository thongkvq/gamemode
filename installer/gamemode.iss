; Bộ cài Paroda GameMode (Inno Setup 6).
; Tên hiển thị là "Paroda GameMode", nhưng tên file exe, thư mục cài, AppId, AppMutex và
; tên bộ cài vẫn giữ "GameMode" để bản cũ nhận ra nhau khi cập nhật OTA / gỡ cài.
; Build: cargo build --release, rồi
;   "%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" installer\gamemode.iss
; Kết quả: dist\GameMode-Setup-<version>.exe

#define AppName "Paroda GameMode"
#define AppVersion "1.5.1"
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
DefaultDirName={localappdata}\Programs\GameMode
DefaultGroupName=Paroda GameMode
; Bản cũ dùng group "GameMode": không kế thừa để nhóm Start Menu mang tên mới.
UsePreviousGroup=no
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
english.RunAppVN=Mở Paroda GameMode ngay

[Tasks]
Name: "desktopicon"; Description: "{cm:DesktopIconVN}"; GroupDescription: "{cm:AdditionalIcons}"

[InstallDelete]
; Bản trước tên hiển thị là "GameMode" -> shortcut cũ nằm ở Desktop và nhóm Start Menu "GameMode".
; Xoá trước khi cài để không còn hai shortcut (một cũ một mới) cùng trỏ GameMode.exe.
Type: files; Name: "{userdesktop}\GameMode.lnk"
Type: files; Name: "{userprograms}\GameMode\GameMode.lnk"
Type: files; Name: "{userprograms}\GameMode\Gỡ cài đặt GameMode.lnk"
Type: dirifempty; Name: "{userprograms}\GameMode"

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
; Cập nhật OTA: bộ cài do GameMode gọi kèm /UPDATE thì luôn mở lại app sau khi cài xong —
; không có skipifsilent nên chạy được cả khi bộ cài ở chế độ /VERYSILENT.
Filename: "{app}\{#AppExe}"; Flags: nowait shellexec; Check: IsUpdateRun

[UninstallRun]
; Xoá task "chạy cùng Windows" nếu người dùng đã bật (bỏ qua nếu không có hoặc không đủ quyền).
Filename: "{sys}\schtasks.exe"; Parameters: "/Delete /TN ""GameMode"" /F"; Flags: runhidden; RunOnceId: "DelTask"

[UninstallDelete]
Type: files; Name: "{app}\gamemode.cfg"
Type: files; Name: "{app}\session.json"
Type: files; Name: "{app}\session.last.json"
Type: files; Name: "{app}\gamemode-crash.log"
Type: files; Name: "{app}\update-pending.json"
Type: files; Name: "{app}\*.txt"
; Báo cáo từng trận: xoá hết file .json rồi tới cả thư mục reports.
Type: files; Name: "{app}\reports\*.json"
Type: filesandordirs; Name: "{app}\reports"
Type: dirifempty; Name: "{app}"

[Code]
{ Bộ cài có được chạy kèm tham số /UPDATE không (do GameMode tự cập nhật OTA gọi).
  Nhận cả "/UPDATE", "-UPDATE" và "/UPDATE=1". }
function IsUpdateRun(): Boolean;
var
  I: Integer;
  P: String;
begin
  Result := False;
  for I := 1 to ParamCount do
  begin
    P := ParamStr(I);
    if (CompareText(P, '/UPDATE') = 0) or (CompareText(P, '-UPDATE') = 0) then
    begin
      Result := True;
      Exit;
    end;
  end;
  if ExpandConstant('{param:UPDATE|}') = '1' then
    Result := True;
end;
