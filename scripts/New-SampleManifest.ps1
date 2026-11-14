param(
    [string]$Id,
    [int]$Port = 0,
    [switch]$ApacheContainer
)
$workspace = Split-Path $PSScriptRoot -Parent
if (-not $Id) {
    if ($ApacheContainer) { $Id = 'apache-container' }
    else { $Id = 'hello-http' }
}
if ($Id -notmatch '^[a-zA-Z0-9_-]{1,64}$') { throw 'Invalid project ID' }
$manifest = @{
    schema_version = 1
    id = $Id
    name = $Id
    root = Join-Path $workspace 'examples/hello-http'
    port = $Port
}
if ($ApacheContainer) {
    $manifest.provider = 'container'
    $manifest.health = @{ type = 'http'; path = '/' }
    $manifest.container = @{
        engine = 'docker'
        image = 'docker.io/library/httpd@sha256:32e9403360ab2f1047523e72f256527fb9aee68b342abf6ad03f0d629157f3d9'
        internal_port = 80
        entrypoint = '/usr/local/apache2/bin/httpd'
        command = @('-DFOREGROUND', '-c', 'PidFile /dockon-data/httpd.pid')
    }
} else {
    $binary = Join-Path $workspace 'target/debug/dockon-hello-http.exe'
    if (-not $IsWindows -and $PSVersionTable.PSEdition -eq 'Core') { $binary = Join-Path $workspace 'target/debug/dockon-hello-http' }
    if (-not (Test-Path -LiteralPath $binary)) { throw 'Build the sample first: cargo build -p dockon-hello-http' }
    $manifest.executable = $binary
    $manifest.args = @()
}
$outDir = Join-Path $workspace '.local'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$outPath = Join-Path $outDir "$Id.project.json"
[System.IO.File]::WriteAllText($outPath, ($manifest | ConvertTo-Json), [System.Text.UTF8Encoding]::new($false))
Write-Output $outPath
