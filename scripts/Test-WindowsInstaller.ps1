[CmdletBinding()]
param([Parameter(Mandatory)][string]$Package)

$ErrorActionPreference = 'Stop'
$directory = 'C:\dockon'
if ($env:GITHUB_ACTIONS -ne 'true' -or (Test-Path -LiteralPath $directory)) {
    throw 'Run this test on a disposable Windows CI runner with a fresh C:\dockon.'
}

$packagePath = (Resolve-Path -LiteralPath $Package).Path
$setup = Start-Process -FilePath $packagePath -ArgumentList '/S', '/NS' -WindowStyle Hidden -PassThru -Wait
if ($setup.ExitCode -ne 0) { throw "Installer failed with exit code $($setup.ExitCode)." }

try {
    foreach ($binary in 'dockon-desktop', 'dockon-daemon', 'dockon-hosts-helper', 'dockonctl', 'uninstall') {
        if (!(Test-Path -LiteralPath "$directory\$binary.exe" -PathType Leaf)) {
            throw "Installed executable is missing: $binary"
        }
    }
    $acl = Get-Acl -LiteralPath $directory
    if (!$acl.AreAccessRulesProtected -or (Get-Acl -LiteralPath "$directory\dockon-hosts-helper.exe").AreAccessRulesProtected) {
        throw 'Expected the protected installer directory to propagate permissions to its binaries.'
    }

    $env:DOCKON_DESKTOP_EXECUTABLE = "$directory\dockon-desktop.exe"
    node ./scripts/desktop-smoke.mjs
    if ($LASTEXITCODE -ne 0) { throw 'The installed app failed the native smoke test.' }

    $sentinel = Join-Path $directory 'user-file.txt'
    Set-Content -LiteralPath $sentinel -Value 'preserve'
}
finally {
    $uninstall = Start-Process -FilePath "$directory\uninstall.exe" -ArgumentList '/S' -WindowStyle Hidden -PassThru -Wait
    if ($uninstall.ExitCode -ne 0) { throw "Uninstaller failed with exit code $($uninstall.ExitCode)." }
}

if ((Get-Content -LiteralPath $sentinel -Raw).Trim() -ne 'preserve' -or (Test-Path -LiteralPath "$directory\dockon-daemon.exe")) {
    throw 'Uninstall must preserve user files and remove Dockon binaries.'
}
