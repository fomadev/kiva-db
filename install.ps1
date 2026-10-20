# Installs the latest KivaDB release on Windows.
#
#   irm https://raw.githubusercontent.com/fomadev/kiva-db/main/install.ps1 | iex
#
# Options, as environment variables:
#   KIVADB_INSTALL_DIR      where to put kivadb.exe
#                           (default: %LOCALAPPDATA%\Programs\KivaDB, no administrator rights needed)
#   KIVADB_VERSION          a version to install instead of the latest, e.g. 1.3.0
#   KIVADB_NO_MODIFY_PATH   set to 1 to leave the PATH unchanged
#
# Works with Windows PowerShell 5.1 and PowerShell 7.
# Copyright (c) 2026 Fordima Inc. Licensed under the Fordima Public License (FPL) v1.0.

# Everything is inside a function: when run with `| iex`, an error must stop
# the installation, not close the user's PowerShell window.
function Install-KivaDB {
    $ErrorActionPreference = 'Stop'
    # Download progress bars make Windows PowerShell 5.1 very slow.
    $ProgressPreference = 'SilentlyContinue'
    # Windows PowerShell 5.1 may not offer TLS 1.2 by default; GitHub requires it.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $repo = 'fomadev/kiva-db'
    $asset = 'kivadb_win_x64.zip'

    # --- Which archive for this system --------------------------------------
    $arch = $env:PROCESSOR_ARCHITECTURE
    if ($env:PROCESSOR_ARCHITEW6432) { $arch = $env:PROCESSOR_ARCHITEW6432 }
    if ($arch -ne 'AMD64') {
        throw "No ready-made KivaDB for Windows on $arch. Build it from the source code: https://github.com/$repo/blob/main/DOCUMENTATION.md#13-build-from-the-source-code"
    }

    if ($env:KIVADB_VERSION) {
        $base = "https://github.com/$repo/releases/download/v$($env:KIVADB_VERSION.TrimStart('v'))"
    } else {
        $base = "https://github.com/$repo/releases/latest/download"
    }
    if ($env:KIVADB_INSTALL_DIR) {
        $dir = $env:KIVADB_INSTALL_DIR
    } else {
        $dir = Join-Path $env:LOCALAPPDATA 'Programs\KivaDB'
    }

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("kivadb-install-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        # --- Download and verify ---------------------------------------------
        Write-Host "Downloading $asset..."
        $zip = Join-Path $tmp $asset
        try {
            Invoke-WebRequest -Uri "$base/$asset" -OutFile $zip -UseBasicParsing
        } catch {
            throw "Cannot download $base/$asset ($($_.Exception.Message)). Check the Internet connection and the version."
        }

        $sums = Join-Path $tmp 'SHA256SUMS.txt'
        $haveSums = $true
        try {
            Invoke-WebRequest -Uri "$base/SHA256SUMS.txt" -OutFile $sums -UseBasicParsing
        } catch {
            $haveSums = $false
        }
        if ($haveSums) {
            $line = Get-Content $sums | Where-Object { $_ -match "^\s*([0-9a-fA-F]{64})\s+\*?$([regex]::Escape($asset))\s*$" } | Select-Object -First 1
            if (-not $line) { throw "SHA256SUMS.txt has no entry for $asset." }
            $expected = ($line -split '\s+')[0].ToLower()
            $actual = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash.ToLower()
            if ($actual -ne $expected) {
                throw "Checksum mismatch for ${asset}: the download was damaged or altered. Nothing was installed."
            }
            Write-Host 'Checksum verified (SHA-256).'
        } else {
            Write-Warning 'This release has no SHA256SUMS.txt: the download cannot be verified.'
        }

        Expand-Archive -Path $zip -DestinationPath $tmp -Force
        $new = Join-Path $tmp 'kivadb.exe'
        if (-not (Test-Path $new)) { throw 'The archive does not contain kivadb.exe.' }

        # --- Install -----------------------------------------------------------
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
        $exe = Join-Path $dir 'kivadb.exe'
        try {
            Copy-Item -Path $new -Destination $exe -Force
        } catch {
            if (Test-Path $exe) {
                throw "Cannot replace $exe : KivaDB is probably running. Stop it first (kivadb stop), or update it with: kivadb update"
            }
            throw "Cannot write $exe ($($_.Exception.Message)). Choose another folder with KIVADB_INSTALL_DIR."
        }

        # --- Check -------------------------------------------------------------
        $version = & $exe --version 2>$null
        if ($LASTEXITCODE -ne 0 -or -not $version) {
            throw "kivadb.exe was installed in $dir but does not run on this system."
        }
        Write-Host "$version installed in $exe"

        # --- PATH --------------------------------------------------------------
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $parts = @()
        if ($userPath) { $parts = $userPath -split ';' | Where-Object { $_ } }
        $inPath = $parts | Where-Object { $_.TrimEnd('\') -ieq $dir.TrimEnd('\') }
        if ($env:KIVADB_NO_MODIFY_PATH -eq '1') {
            if (-not $inPath) { Write-Host "Add $dir to your PATH to type 'kivadb' from any folder." }
        } elseif (-not $inPath) {
            [Environment]::SetEnvironmentVariable('Path', (($parts + $dir) -join ';'), 'User')
            Write-Host "$dir was added to your PATH. Open a new terminal to use it everywhere."
        }
        # This window can use it right away.
        if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $dir.TrimEnd('\') })) {
            $env:Path = "$env:Path;$dir"
        }

        Write-Host 'Start it with: kivadb start'
        Write-Host 'Later, update it with: kivadb update'
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }
}

try {
    Install-KivaDB
} catch {
    Write-Host "Error: $($_.Exception.Message)" -ForegroundColor Red
    # Run as a file (powershell -File install.ps1), report the failure; run
    # with `| iex`, do not close the window.
    if ($PSCommandPath) { exit 1 }
}
