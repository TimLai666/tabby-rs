$ErrorActionPreference = 'Stop'

$version = '3.02'
$url = 'https://www.nasm.us/pub/nasm/releasebuilds/3.02/win64/nasm-3.02-installer-x64.exe'
$expectedSha256 = '0DDB40310861EB29F4D649FEB9466779982A2D251C0DB2B9CF0D21CF591171F3'
$installer = Join-Path $env:RUNNER_TEMP "nasm-$version-installer-x64.exe"

$command = Get-Command nasm -ErrorAction SilentlyContinue
if ($command) {
    $nasmExecutable = $command.Source
} else {
    Invoke-WebRequest -Uri $url -OutFile $installer
    $actualSha256 = (Get-FileHash -Path $installer -Algorithm SHA256).Hash
    if ($actualSha256 -ne $expectedSha256) {
        throw "NASM installer checksum mismatch: expected $expectedSha256, got $actualSha256"
    }

    $process = Start-Process -FilePath $installer -ArgumentList '/S' -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        throw "NASM installer failed with exit code $($process.ExitCode)"
    }

    $nasmExecutable = Join-Path ${env:ProgramFiles} 'NASM\nasm.exe'
}

if (-not (Test-Path $nasmExecutable)) {
    throw "NASM executable was not found at $nasmExecutable"
}

$nasmDirectory = Split-Path -Parent $nasmExecutable
Add-Content -Path $env:GITHUB_PATH -Value $nasmDirectory
$env:Path = "$nasmDirectory;$env:Path"
& $nasmExecutable -v
