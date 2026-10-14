<#
.SYNOPSIS
  Builds the Ondalux Heatmap website and publishes it to the hosting
  (HostGator / cPanel) over FTPS, or packages it as a .zip for cPanel.

.DESCRIPTION
  1. Runs `node website/build.mjs` (pages, download, sitemap).
  2. With -Zip: writes website/ondaluxheatmap-site.zip to upload with
     cPanel > File Manager > public_html > Upload, then Extract.
     Without -Zip: uploads every file in website/public to the server
     with curl.exe (included in Windows 10/11) over explicit TLS.

  The password is never stored in a file or passed on the command line:
  it is read from $env:ONDALUX_FTP_PASS or typed at the prompt.

.EXAMPLE
  # Once per PowerShell session (values from cPanel > FTP Accounts):
  $env:ONDALUX_FTP_HOST = "ftp.ondaluxheatmap.com"
  $env:ONDALUX_FTP_USER = "your-ftp-user"
  .\website\deploy.ps1                 # build and upload to public_html
  .\website\deploy.ps1 -RemoteDir ""   # if the FTP account opens in public_html
  .\website\deploy.ps1 -Zip            # only build the .zip for cPanel
#>
param(
    [switch]$Zip,
    [switch]$SkipBuild,
    [string]$RemoteDir = "public_html",
    [int]$Port = 21
)
$ErrorActionPreference = "Stop"
$here = $PSScriptRoot
$public = Join-Path $here "public"

if (-not $SkipBuild) {
    node (Join-Path $here "build.mjs")
    if ($LASTEXITCODE -ne 0) { throw "build failed" }
}

$files = Get-ChildItem -Path $public -Recurse -File -Force
$total = ($files | Measure-Object Length -Sum).Sum / 1MB
Write-Host ("{0} files, {1:N1} MB" -f $files.Count, $total)

if ($Zip) {
    $zipPath = Join-Path $here "ondaluxheatmap-site.zip"
    if (Test-Path $zipPath) { Remove-Item $zipPath }
    # The folder content (including .htaccess) goes at the root of the zip.
    Compress-Archive -Path (Join-Path $public "*") -DestinationPath $zipPath
    Write-Host "Wrote $zipPath"
    Write-Host "cPanel > File Manager > public_html > Upload, then right-click the zip > Extract."
    return
}

$hostName = $env:ONDALUX_FTP_HOST
$user = $env:ONDALUX_FTP_USER
if (-not $hostName) { $hostName = Read-Host "FTP server (e.g. ftp.ondaluxheatmap.com)" }
if (-not $user) { $user = Read-Host "FTP user" }
$pass = $env:ONDALUX_FTP_PASS
if (-not $pass) {
    $secure = Read-Host "FTP password" -AsSecureString
    $pass = [Runtime.InteropServices.Marshal]::PtrToStringBSTR(
        [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure))
}

# Credentials go to curl through stdin (-K -), never on the command line.
$escaped = ("{0}:{1}" -f $user, $pass).Replace('\', '\\').Replace('"', '\"')
$config = "user = `"$escaped`""
$base = "ftp://{0}:{1}/{2}" -f $hostName, $Port, ($RemoteDir.Trim("/"))
if ($RemoteDir.Trim("/") -ne "") { $base += "/" }

$i = 0
foreach ($f in $files) {
    $i++
    $rel = $f.FullName.Substring($public.Length + 1).Replace('\', '/')
    $url = $base + [Uri]::EscapeUriString($rel)
    Write-Host ("[{0}/{1}] {2}" -f $i, $files.Count, $rel)
    $config | curl.exe --silent --show-error --fail --ssl-reqd --ftp-create-dirs `
        --retry 2 -K - -T $f.FullName $url
    if ($LASTEXITCODE -ne 0) { throw "upload failed: $rel (curl exit $LASTEXITCODE)" }
}
Write-Host "Published to https://ondaluxheatmap.com"
