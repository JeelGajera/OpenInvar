$ErrorActionPreference = 'Stop'

# OpenInvar Installation Script for Windows

$GITHUB_REPO = "JeelGajera/OpenInvar"
$TARGET = "x86_64-pc-windows-msvc"
$ASSET_NAME = "openinvar-${TARGET}.zip"
$DOWNLOAD_URL = "https://github.com/${GITHUB_REPO}/releases/latest/download/${ASSET_NAME}"

$INSTALL_ROOT = [System.IO.Path]::Combine($env:LOCALAPPDATA, "Programs", "openinvar")
$DEFAULT_BIN_DIR = [System.IO.Path]::Combine($INSTALL_ROOT, "bin")

function Write-Step ($msg) {
    Write-Host "`n» " -NoNewline -ForegroundColor Cyan
    Write-Host $msg -ForegroundColor White -Bold
}

function Write-Info ($msg) {
    Write-Host "  info " -NoNewline -ForegroundColor Blue
    Write-Host $msg
}

function Write-Success ($msg) {
    Write-Host "  success " -NoNewline -ForegroundColor Green
    Write-Host $msg
}

function Write-Warn ($msg) {
    Write-Host "  warn " -NoNewline -ForegroundColor Yellow
    Write-Host $msg
}

function Write-Error-Exit ($msg) {
    Write-Host "  error " -NoNewline -ForegroundColor Red
    Write-Host $msg
    exit 1
}

# ASCII Logo
Write-Host @'
   ____                   ____
  / __ \____  ___  ____  /  _/___ _   ______ ______
 / / / / __ \/ _ \/ __ \ / // __ \ | / / __ `/ ___/
/ /_/ / /_/ /  __/ / / // // / / / |/ / /_/ / /
\____/ .___/\___/_/ /_/___/_/ /_/|___/\__,_/_/
    /_/
'@ -ForegroundColor Blue

Write-Step "Initializing installation..."
Write-Info "Detected Platform: Windows (x86_64)"
Write-Info "Target: $TARGET"

# Check for existing installation
$EXISTING_EXE = Get-Command openinvar -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source
if ($EXISTING_EXE) {
    Write-Step "Updating existing installation..."
    try {
        $OLD_VERSION = (& $EXISTING_EXE --version).Split(" ")[-1]
        Write-Info "Found OpenInvar $OLD_VERSION at $EXISTING_EXE"
    } catch {
        Write-Info "Found existing OpenInvar at $EXISTING_EXE"
    }
    $INSTALL_TARGET = $EXISTING_EXE
} else {
    Write-Step "Preparing new installation..."
    $INSTALL_TARGET = [System.IO.Path]::Combine($DEFAULT_BIN_DIR, "openinvar.exe")
}

$BIN_DIR = [System.IO.Path]::GetDirectoryName($INSTALL_TARGET)

# 1. Create temp directory
$TMP_DIR = [System.IO.Path]::Combine(
    [System.IO.Path]::GetTempPath(),
    "openinvar-install-" + [System.Guid]::NewGuid().ToString().Substring(0, 8)
)
New-Item -ItemType Directory -Force -Path $TMP_DIR | Out-Null

try {
    # 2. Download
    Write-Step "Downloading latest release..."
    Write-Info "URL: $DOWNLOAD_URL"
    $ZIP_PATH = [System.IO.Path]::Combine($TMP_DIR, $ASSET_NAME)
    
    Invoke-WebRequest -Uri $DOWNLOAD_URL -OutFile $ZIP_PATH -UseBasicParsing
    Write-Success "Download complete."

    # 3. Extract
    Write-Step "Extracting binary..."
    Expand-Archive -Path $ZIP_PATH -DestinationPath $TMP_DIR -Force
    Write-Success "Extraction complete."

    # 4. Install
    Write-Step "Finalizing installation..."
    if (!(Test-Path $BIN_DIR)) {
        New-Item -ItemType Directory -Force -Path $BIN_DIR | Out-Null
    }

    $SOURCE_EXE = [System.IO.Path]::Combine($TMP_DIR, "openinvar.exe")
    
    # Try to move, handle file lock if running
    try {
        Move-Item -Path $SOURCE_EXE -Destination $INSTALL_TARGET -Force
    } catch {
        Write-Warn "Could not replace openinvar.exe. It might be in use."
        Write-Info "Retrying in 2 seconds..."
        Start-Sleep -Seconds 2
        Move-Item -Path $SOURCE_EXE -Destination $INSTALL_TARGET -Force
    }

    # 5. Verify install
    $NEW_VERSION = (& $INSTALL_TARGET --version).Split(" ")[-1]
    Write-Host "`n✅ OpenInvar $NEW_VERSION has been installed!`n" -ForegroundColor Green

    # 6. PATH setup
    $USER_PATH = [Environment]::GetEnvironmentVariable("PATH", "User")

    if ($USER_PATH -notlike "*$BIN_DIR*") {
        Write-Step "Updating PATH..."
        Write-Info "Adding $BIN_DIR to User PATH"

        $NEW_PATH = "$USER_PATH;$BIN_DIR"
        [Environment]::SetEnvironmentVariable("PATH", $NEW_PATH, "User")

        # Update current session path too
        $env:PATH = "$env:PATH;$BIN_DIR"

        Write-Warn "PATH updated. You may need to restart your terminal for changes to take effect."
    }
    else {
        Write-Success "OpenInvar is ready! Run 'openinvar --help' to get started."
    }

} catch {
    Write-Error-Exit "Installation failed: $_"
}
finally {
    # Cleanup
    if (Test-Path $TMP_DIR) {
        Remove-Item -Path $TMP_DIR -Recurse -Force
    }
}

Write-Host "==========================================" -ForegroundColor Blue
