# Lift every module of Settlers 4 for Windows: s4/lifted/<crate>/src (not committed).
param(
    [string]$GameDir = "extracted/s4",
    [string]$Target = "x86_64-pc-windows-msvc"
)

$ErrorActionPreference = "Stop"

# Locate repo root
$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

# Find Exe directory
$ExeDir = $null
if (Test-Path (Join-Path $GameDir "Exe\S4_Main.exe")) {
    $ExeDir = Join-Path $GameDir "Exe"
} elseif (Test-Path (Join-Path $GameDir "S4_Main.exe")) {
    $ExeDir = $GameDir
} elseif (Test-Path "extracted\s4\Exe\S4_Main.exe") {
    $ExeDir = "extracted\s4\Exe"
} else {
    Write-Error "Cannot find S4_Main.exe in '$GameDir' or 'extracted\s4\Exe'. Please specify -GameDir <path to Settlers 4>."
    exit 1
}

Write-Host "Using game Exe directory: $ExeDir" -ForegroundColor Cyan

# Ensure Cargo placeholders exist for crates missing a lib.rs
Get-ChildItem "s4/lifted" -Directory | ForEach-Object {
    $crateDir = $_.FullName
    $cargoToml = Join-Path $crateDir "Cargo.toml"
    $libRs = Join-Path $crateDir "src\lib.rs"
    if ((Test-Path $cargoToml) -and -not (Test-Path $libRs)) {
        New-Item -ItemType Directory -Force (Join-Path $crateDir "src") | Out-Null
        Set-Content -Path $libRs -Value "// Placeholder until tools/lift_all generates this crate.`npub fn dispatch(_c: &mut s4_x86::Cpu, _addr: u32) -> bool { false }`n"
    }
}

# 1. Build the lift tool
Write-Host "Building lift tool ($Target)..." -ForegroundColor Cyan
cargo build --release --target $Target -p lift
if ($LASTEXITCODE -ne 0) {
    Write-Error "Failed to build lift tool"
    exit $LASTEXITCODE
}

$LiftExe = Join-Path $RepoRoot "target\$Target\release\lift.exe"
if (-not (Test-Path $LiftExe)) {
    Write-Error "lift.exe not found at $LiftExe"
    exit 1
}

$Modules = @(
    @{ bin = "S4_Main.exe";        crate = "s4main" },
    @{ bin = "GfxEngine.dll";      crate = "gfxengine" },
    @{ bin = "GuiEngine2.dll";     crate = "guiengine2" },
    @{ bin = "SoundEngine.dll";    crate = "soundengine" },
    @{ bin = "EventEngine.dll";    crate = "eventengine" },
    @{ bin = "MapAccess.dll";      crate = "mapaccess" },
    @{ bin = "SaveGameAccess.dll"; crate = "savegameaccess" },
    @{ bin = "VideoEngine.dll";    crate = "videoengine" },
    @{ bin = "S4_BBGCClient.dll";  crate = "s4bbgcclient" },
    @{ bin = "BB_Support.dll";     crate = "bbsupport" },
    @{ bin = "EconomyAI.dll";      crate = "economyai" },
    @{ bin = "MapGenerator.dll";   crate = "mapgenerator" },
    @{ bin = "RTComp.dll";         crate = "rtcomp" },
    @{ bin = "Utility.dll";        crate = "utility" },
    @{ bin = "MSVCRT.DLL";         crate = "msvcrt" },
    @{ bin = "MSVCP60.DLL";        crate = "msvcp60" },
    @{ bin = "MSVCIRT.DLL";        crate = "msvcirt" },
    @{ bin = "binkw32.dll";        crate = "binkw32" }
)

function Get-ExpectedSha256([string]$tsvPath) {
    if (-not (Test-Path $tsvPath)) { return $null }
    $firstLine = Get-Content -Path $tsvPath -TotalCount 1
    if ($firstLine -match '^# sha256\s+([0-9a-fA-F]+)') {
        return $matches[1].ToLower()
    }
    return $null
}

foreach ($m in $Modules) {
    $bin = $m.bin
    $crate = $m.crate
    $binPath = Join-Path $ExeDir $bin

    if (-not (Test-Path $binPath)) {
        Write-Warning "Skipping ${bin}: file not found at $binPath"
        continue
    }

    $cratePath = Join-Path $RepoRoot "s4\lifted\$crate"
    $outDir = Join-Path $cratePath "src"

    if (Test-Path $outDir) {
        Remove-Item -Recurse -Force $outDir
    }
    New-Item -ItemType Directory -Force $outDir | Out-Null

    # Determine function list TSV
    $candTsv = Join-Path $RepoRoot "tools\lift\functions\$bin.tsv"
    $tsv = "export\$bin\functions.tsv"
    if (Test-Path $candTsv) {
        $expectedHash = Get-ExpectedSha256 $candTsv
        if ($expectedHash) {
            $fileHash = (Get-FileHash -Path $binPath -Algorithm SHA256).Hash.ToLower()
            if ($fileHash -eq $expectedHash) {
                $tsv = $candTsv
            }
        }
    }

    $patchFile = Join-Path $RepoRoot "tools\lift\patches\$bin.tsv"
    $liftArgs = @($binPath, $tsv, $outDir)
    if (Test-Path $patchFile) {
        $liftArgs += @("--patch", $patchFile)
    }

    Write-Host -NoNewline "$bin : "
    & $LiftExe @liftArgs
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Lifting failed for $bin"
        exit $LASTEXITCODE
    }
}

Write-Host "`nAll modules lifted successfully into s4/lifted/*/src!" -ForegroundColor Green
Write-Host "Next step: Rebuild s4-play with:" -ForegroundColor Yellow
Write-Host "  cargo build --release -j 4 --target $Target -p s4-play --features sdl3-from-source" -ForegroundColor White
