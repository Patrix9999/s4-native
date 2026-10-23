# Check that a directory holds the supported game version (GOG Settlers 4 Gold 2.50.1516)
param(
    [Parameter(Mandatory=$true)]
    [string]$GameDir
)

$RepoRoot = Split-Path -Parent $PSScriptRoot
$ShaList = Join-Path $RepoRoot "tools\game-files.sha256"

if (-not (Test-Path (Join-Path $GameDir "Exe\S4_Main.exe"))) {
    Write-Error "Not a Settlers 4 directory: $(Join-Path $GameDir 'Exe\S4_Main.exe') is missing."
    exit 2
}

$bad = 0
Get-Content $ShaList | ForEach-Object {
    $line = $_.Trim()
    if ($line -and -not $line.StartsWith("#")) {
        $parts = -split $line
        $expectedHash = $parts[0].ToLower()
        $relPath = $parts[1]
        $fullPath = Join-Path $GameDir $relPath

        if (-not (Test-Path $fullPath)) {
            Write-Host "MISSING   $relPath" -ForegroundColor Red
            $bad = 1
        } else {
            $actualHash = (Get-FileHash -Path $fullPath -Algorithm SHA256).Hash.ToLower()
            if ($actualHash -ne $expectedHash) {
                Write-Host "DIFFERENT $relPath" -ForegroundColor Red
                $bad = 1
            }
        }
    }
}

foreach ($d in @("Gfx", "Snd", "Video", "Map", "Config")) {
    $fullDir = Join-Path $GameDir $d
    if (-not (Test-Path $fullDir)) {
        Write-Host "MISSING   $d/ (game data directory)" -ForegroundColor Red
        $bad = 1
    }
}

if ($bad -ne 0) {
    Write-Warning "This is not the supported version (GOG Gold 2.50.1516), or the files are incomplete."
    exit 1
}

Write-Host "OK: all executables and DLLs match GOG Settlers 4 Gold 2.50.1516." -ForegroundColor Green
