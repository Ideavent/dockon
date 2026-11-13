param([string]$Id = 'hello-http', [int]$Port = 0)
$workspace = Split-Path $PSScriptRoot -Parent
$binary = Join-Path $workspace 'target\debug\dockon-hello-http.exe'
if (-not $IsWindows -and $PSVersionTable.PSEdition -eq 'Core') { $binary = Join-Path $workspace 'target/debug/dockon-hello-http' }
if (-not (Test-Path -LiteralPath $binary)) { throw 'Build the sample first: cargo build -p dockon-hello-http' }
$manifest = @{ schema_version = 1; id = $Id; name = $Id; root = (Join-Path $workspace 'examples\hello-http'); executable = $binary; args = @(); port = $Port }
$outDir = Join-Path $workspace '.local'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$outPath = Join-Path $outDir "$Id.project.json"
[System.IO.File]::WriteAllText($outPath, ($manifest | ConvertTo-Json), [System.Text.UTF8Encoding]::new($false))
Write-Output $outPath
