$env:CARGO_HOME = Join-Path $PSScriptRoot '..\.tools\cargo'
$env:RUSTUP_HOME = Join-Path $PSScriptRoot '..\.tools\rustup'
$env:PATH = "$(Join-Path $env:CARGO_HOME 'bin');$env:PATH"
