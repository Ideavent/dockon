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
    foreach ($relative in 'apache/bin/httpd.exe', 'php/php-cgi.exe', 'mysql/bin/mysqld.exe', 'mailpit/mailpit.exe') {
        if (!(Test-Path -LiteralPath "$directory/bin/$relative" -PathType Leaf)) {
            throw "Bundled runtime is missing: $relative"
        }
    }
    $env:DOCKON_DATA_DIR = (New-Item -ItemType Directory -Path (Join-Path $env:RUNNER_TEMP ('dockon-' + [guid]::NewGuid()))).FullName
    $daemon = Start-Process -FilePath "$directory\dockon-daemon.exe" -PassThru -WindowStyle Hidden
    try {
        $ready = $false
        for ($attempt = 0; $attempt -lt 50; $attempt++) {
            if ($daemon.HasExited) { throw "Installed daemon exited with code $($daemon.ExitCode)." }
            if (Test-Path -LiteralPath "$env:DOCKON_DATA_DIR\connection.json") {
                $runtimeReply = & "$directory\dockonctl.exe" runtimes | ConvertFrom-Json
                if ($runtimeReply.ok) { $ready = $true; break }
            }
            Start-Sleep -Milliseconds 300
        }
        if (!$ready) { throw 'Installed daemon did not become ready.' }
        foreach ($runtime in 'apache', 'php', 'php-cgi') {
            if (!($runtimeReply.data | Where-Object { $_.runtime -eq $runtime -and $_.source -eq 'dockon_bundle' })) {
                throw "Installed daemon did not register bundled runtime: $runtime"
            }
        }
    }
    finally {
        if (!$daemon.HasExited) {
            if (Test-Path -LiteralPath "$env:DOCKON_DATA_DIR\connection.json") {
                & "$directory\dockonctl.exe" shutdown | Out-Null
            }
            if (!$daemon.WaitForExit(5000)) { $daemon.Kill() }
        }
    }
    if (!$acl.AreAccessRulesProtected -or (Get-Acl -LiteralPath "$directory\dockon-hosts-helper.exe").AreAccessRulesProtected) {
        throw 'Expected the protected installer directory to propagate permissions to its binaries.'
    }

    New-Item -ItemType Directory -Path "$directory\www\fixture-project\nested", "$directory\wwwold\outside-project" -Force | Out-Null
    if ($env:DOCKON_TEST_INTERACTIVE_WEBVIEW -eq '1') {
        $env:DOCKON_DESKTOP_EXECUTABLE = "$directory\dockon-desktop.exe"
        $env:DOCKON_WORKSPACE_SMOKE = '1'
        node ./scripts/desktop-smoke.mjs
        if ($LASTEXITCODE -ne 0) { throw 'The installed app failed the native smoke test.' }
    }

    $sentinel = Join-Path $directory 'www/user-file.txt'
    Set-Content -LiteralPath $sentinel -Value 'preserve'
}
finally {
    $uninstall = Start-Process -FilePath "$directory\uninstall.exe" -ArgumentList '/S' -WindowStyle Hidden -PassThru -Wait
    if ($uninstall.ExitCode -ne 0) { throw "Uninstaller failed with exit code $($uninstall.ExitCode)." }
}

if ((Get-Content -LiteralPath $sentinel -Raw).Trim() -ne 'preserve' -or (Test-Path -LiteralPath "$directory\dockon-daemon.exe")) {
    throw 'Uninstall must preserve user files and remove Dockon binaries.'
}
