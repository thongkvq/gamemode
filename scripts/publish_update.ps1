#Requires -Version 5.1
<#
.SYNOPSIS
    Publish bản cập nhật OTA của GameMode lên server nội bộ (Proxmox host + container nginx).

.DESCRIPTION
    Việc script làm, theo thứ tự:
      1. Kiểm tra -Version khớp `version` trong Cargo.toml và `#define AppVersion` trong
         installer\gamemode.iss; kiểm tra file bộ cài đã build.
      2. Tính size + sha256, ghi manifest.json (UTF-8 không BOM) ra thư mục tạm.
      3. scp bộ cài + manifest + releases.json lên /tmp của host Proxmox.
      4. Một lệnh ssh: `pct push` bộ cài vào /srv/dhcs-update/apps/gamemode/releases/,
         push manifest vào manifest.json.tmp rồi `mv` thành manifest.json (đổi nguyên tử),
         chown www-data, xoá file tạm ở /tmp.
      5. Đọc lại manifest qua HTTPS và so version + sha256; sai thì throw.

    Chạy từ PC Windows của chủ máy (cần OpenSSH + khoá ssh tới host Proxmox):
        scripts\publish_update.ps1 -Version 1.3.0 -Changelog "# 1.3.0 - ..."
    Thêm -DryRun để chỉ tạo manifest và in đường dẫn, KHÔNG upload.

.EXAMPLE
    .\scripts\publish_update.ps1 -Version 1.3.0 -Changelog "# 1.3.0`n`n- Them cap nhat OTA" -DryRun
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$Changelog,
    [string]$Package,
    [string]$Server = '192.168.100.2',
    [string]$ContainerId = '106',
    [string]$KeyPath = "$env:USERPROFILE\.ssh\id_ed25519_proxmox",
    [string]$PublicBaseUrl = 'https://update-dhcs.paroda.net/apps/gamemode',
    # Thư mục trong container; đổi cùng -PublicBaseUrl khi muốn publish thử lên kênh khác.
    [string]$RemoteDir = '/srv/dhcs-update/apps/gamemode',
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'

$RepoRoot = Split-Path -Parent $PSScriptRoot
$ScpExe = Join-Path $env:SystemRoot 'System32\OpenSSH\scp.exe'
$SshExe = Join-Path $env:SystemRoot 'System32\OpenSSH\ssh.exe'
$RemoteUser = 'root'
$MinimumVersion = '1.0.0'

function Fail([string]$Message) {
    throw "publish_update: $Message"
}

# --- 1. Kiểm tra version khớp Cargo.toml + installer -------------------------------------
$cargoPath = Join-Path $RepoRoot 'Cargo.toml'
$issPath = Join-Path $RepoRoot 'installer\gamemode.iss'
if (-not (Test-Path -LiteralPath $cargoPath)) { Fail "không thấy $cargoPath" }
if (-not (Test-Path -LiteralPath $issPath)) { Fail "không thấy $issPath" }

$cargoText = Get-Content -LiteralPath $cargoPath -Raw
if ($cargoText -notmatch '(?m)^\s*version\s*=\s*"([^"]+)"') { Fail "không đọc được version trong Cargo.toml" }
$cargoVersion = $Matches[1]
if ($cargoVersion -ne $Version) {
    Fail "-Version $Version nhưng Cargo.toml ghi $cargoVersion (sửa Cargo.toml trước, hoặc truyền đúng -Version)"
}

$issText = Get-Content -LiteralPath $issPath -Raw
if ($issText -notmatch '#define\s+AppVersion\s+"([^"]+)"') { Fail "không đọc được #define AppVersion trong installer\gamemode.iss" }
$issVersion = $Matches[1]
if ($issVersion -ne $Version) {
    Fail "-Version $Version nhưng installer\gamemode.iss ghi $issVersion"
}

# --- 2. Bộ cài + manifest ----------------------------------------------------------------
if (-not $Package) { $Package = "dist\GameMode-Setup-$Version.exe" }
$packageFull = if ([System.IO.Path]::IsPathRooted($Package)) { $Package } else { Join-Path $RepoRoot $Package }
if (-not (Test-Path -LiteralPath $packageFull)) {
    Fail "không thấy bộ cài $packageFull — chạy cargo build --release rồi ISCC installer\gamemode.iss trước"
}

$filename = "GameMode-Setup-$Version.exe"
$size = (Get-Item -LiteralPath $packageFull).Length
$sha256 = (Get-FileHash -LiteralPath $packageFull -Algorithm SHA256).Hash.ToLowerInvariant()
$downloadUrl = "$PublicBaseUrl/releases/$filename"

$manifest = [ordered]@{
    app             = 'GameMode'
    channel         = 'stable'
    version         = $Version
    minimum_version = $MinimumVersion
    filename        = $filename
    download_url    = $downloadUrl
    size            = [int64]$size
    sha256          = $sha256
    changelog       = $Changelog
    pub_date        = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
}
$manifestJson = $manifest | ConvertTo-Json -Depth 5

$tmpDir = Join-Path $env:TEMP 'gamemode-publish'
New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null
$manifestPath = Join-Path $tmpDir 'manifest.json'
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText($manifestPath, $manifestJson, $utf8NoBom)

Write-Host "Bộ cài : $packageFull"
Write-Host "  size   : $size byte"
Write-Host "  sha256 : $sha256"
Write-Host "Manifest: $manifestPath"

if ($DryRun) {
    Write-Host '--- DryRun: manifest.json (chưa upload) ---'
    Write-Host $manifestJson
    Write-Host "DryRun xong. Manifest ở: $manifestPath"
    return
}

# --- 3. releases.json (mảng manifest cũ + mới, mới nhất ở đầu) ----------------------------
function Get-RemoteText([string]$Url) {
    try {
        $resp = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 30
        return $resp.Content
    } catch {
        # PS 5.1 ném WebException, PS 7 ném HttpResponseException — đọc mã trạng thái từ cả hai.
        $status = 0
        $webResp = $_.Exception.Response
        if ($webResp -and $webResp.StatusCode) { $status = [int]$webResp.StatusCode }
        if ($status -eq 404) { return $null }
        Fail "không đọc được $Url (HTTP $status): $($_.Exception.Message)"
    }
}

$releasesUrl = "$PublicBaseUrl/releases.json"
$existingJson = Get-RemoteText $releasesUrl
$all = @()
if ($existingJson) {
    try {
        $parsed = $existingJson | ConvertFrom-Json
        if ($parsed) { $all = @($parsed) }
    } catch {
        Write-Warning "releases.json hiện có không đọc được ($($_.Exception.Message)) — bắt đầu lại từ mảng rỗng"
        $all = @()
    }
}
# Bỏ bản trùng version rồi chèn bản mới lên đầu.
$all = @($all | Where-Object { $_.version -ne $Version })
$all = @([pscustomobject]$manifest) + $all

# Ghép tay để chắc chắn ra MẢNG JSON kể cả khi chỉ có 1 phần tử (ConvertTo-Json của PS 5.1
# bỏ dấu [] khi mảng chỉ có một phần tử).
$items = @()
foreach ($m in $all) { $items += ($m | ConvertTo-Json -Depth 5 -Compress) }
$releasesJson = "[`n  " + ($items -join ",`n  ") + "`n]"
$releasesPath = Join-Path $tmpDir 'releases.json'
[System.IO.File]::WriteAllText($releasesPath, $releasesJson, $utf8NoBom)

# --- 4. Upload: scp lên /tmp của host, rồi một lệnh ssh đẩy vào container ----------------
foreach ($exe in @($ScpExe, $SshExe)) {
    if (-not (Test-Path -LiteralPath $exe)) { Fail "không thấy $exe (cài OpenSSH Client của Windows)" }
}
if (-not (Test-Path -LiteralPath $KeyPath)) { Fail "không thấy khoá ssh $KeyPath" }

$stamp = [Guid]::NewGuid().ToString('N').Substring(0, 8)
$remotePkg = "/tmp/$filename"
$remoteManifest = "/tmp/gamemode-manifest-$stamp.json"
$remoteReleases = "/tmp/gamemode-releases-$stamp.json"
$target = "$RemoteUser@$Server"

Write-Host "scp lên $target ..."
& $ScpExe -i $KeyPath -o BatchMode=yes -o StrictHostKeyChecking=accept-new `
    $packageFull $manifestPath $releasesPath "${target}:/tmp/"
if ($LASTEXITCODE -ne 0) { Fail "scp thất bại (exit $LASTEXITCODE)" }

# Đổi tên file tạm trên host cho khớp tên dùng trong lệnh ssh bên dưới.
& $SshExe -i $KeyPath -o BatchMode=yes $target `
    "mv /tmp/manifest.json $remoteManifest && mv /tmp/releases.json $remoteReleases"
if ($LASTEXITCODE -ne 0) { Fail "ssh (đổi tên file tạm) thất bại (exit $LASTEXITCODE)" }

# Một lệnh ssh duy nhất: push bộ cài + manifest + releases.json vào container, đổi manifest
# nguyên tử bằng `mv`, chown www-data, rồi xoá file tạm ở /tmp của host.
$remoteScript = @"
set -e
pct exec $ContainerId -- mkdir -p $RemoteDir/releases
pct push $ContainerId $remotePkg $RemoteDir/releases/$filename --perms 644
pct push $ContainerId $remoteManifest $RemoteDir/manifest.json.tmp --perms 644
pct exec $ContainerId -- mv $RemoteDir/manifest.json.tmp $RemoteDir/manifest.json
pct push $ContainerId $remoteReleases $RemoteDir/releases.json.tmp --perms 644
pct exec $ContainerId -- mv $RemoteDir/releases.json.tmp $RemoteDir/releases.json
pct exec $ContainerId -- chown -R www-data:www-data $RemoteDir
rm -f $remotePkg $remoteManifest $remoteReleases
"@
# Checkout có autocrlf thì here-string mang CRLF, bash trên host sẽ hỏng ở "set -e\r".
$remoteScript = $remoteScript -replace "`r", ""

Write-Host "pct push vào container $ContainerId ..."
& $SshExe -i $KeyPath -o BatchMode=yes $target $remoteScript
if ($LASTEXITCODE -ne 0) { Fail "ssh (publish) thất bại (exit $LASTEXITCODE)" }

# --- 5. Kiểm tra lại qua HTTPS -----------------------------------------------------------
Start-Sleep -Seconds 1
$liveJson = Get-RemoteText "$PublicBaseUrl/manifest.json"
if (-not $liveJson) { Fail "không đọc lại được $PublicBaseUrl/manifest.json sau khi publish" }
$live = $liveJson | ConvertFrom-Json
if ($live.version -ne $Version) { Fail "manifest trên server ghi version $($live.version), mong đợi $Version" }
if ("$($live.sha256)".ToLowerInvariant() -ne $sha256) {
    Fail "manifest trên server ghi sha256 $($live.sha256), mong đợi $sha256"
}
if ([int64]$live.size -ne [int64]$size) { Fail "manifest trên server ghi size $($live.size), mong đợi $size" }

Write-Host "OK: đã publish GameMode $Version"
Write-Host "  manifest : $PublicBaseUrl/manifest.json"
Write-Host "  bộ cài   : $downloadUrl"
