[CmdletBinding()]
param([Parameter(Mandatory)][string]$Stage)

$ErrorActionPreference = 'Stop'
$Stage = [IO.Path]::GetFullPath($Stage)
$cache = New-Item -ItemType Directory -Force -Path (Join-Path $Stage '../stack-downloads')
$packages = @(
    @{ Name = 'apache'; File = 'apache.zip'; Prefix = 'Apache24'; Binary = 'bin/httpd.exe'; Algorithm = 'SHA512'
       Url = 'https://www.apachelounge.com/download/VS18/binaries/httpd-2.4.71-261112-Win64-VS18.zip'
       Hash = 'CA0EB9067369D4E4A596A5ED7E31D53BEE70F2524B1A671AA70CC0E8E07C838726B4CD6B22249FF3180171B7B8DD011DCF44DFA38CE5005373878635EF8894F9' },
    @{ Name = 'php'; File = 'php.zip'; Prefix = ''; Binary = 'php-cgi.exe'; Algorithm = 'SHA256'
       Url = 'https://windows.php.net/downloads/releases/php-8.5.12-1-nts-Win32-vs17-x64.zip'
       Hash = '2a36a2edcb7a5b9252f638fd5e4cf84d7f6e066092654220dc0b63e5cf4e9723' },
    @{ Name = 'mysql'; File = 'mysql.zip'; Prefix = 'mysql-8.4.13-winx64'; Binary = 'bin/mysqld.exe'; Algorithm = 'SHA256'
       Url = 'https://cdn.mysql.com/Downloads/MySQL-8.4/mysql-8.4.13-winx64.zip'
       Hash = 'e9cd98b1d23f122987b1afca26ad942efec5891481103d48ae3cde237fa3ea54' },
    @{ Name = 'mailpit'; File = 'mailpit.zip'; Prefix = ''; Binary = 'mailpit.exe'; Algorithm = 'SHA256'
       Url = 'https://github.com/axllent/mailpit/releases/download/v1.33.1/mailpit-windows-amd64.zip'
       Hash = '0bcf8f1e7c21005b30c206064ff2f37dac602ef66774e558379d326f5673d652' }
)

$output = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'bin')
foreach ($package in $packages) {
    $archive = Join-Path $cache.FullName $package.File
    if (!(Test-Path -LiteralPath $archive) -or (Get-FileHash -LiteralPath $archive -Algorithm $package.Algorithm).Hash -ne $package.Hash) {
        curl.exe --fail --silent --show-error --location --retry 2 --proto '=https' --proto-redir '=https' --output $archive $package.Url
        if ($LASTEXITCODE -ne 0) { throw "Could not download $($package.Name)." }
    }
    if ((Get-FileHash -LiteralPath $archive -Algorithm $package.Algorithm).Hash -ne $package.Hash) {
        throw "Digest verification failed for $($package.Name)."
    }
    $destination = Join-Path $output.FullName $package.Name
    $extract = New-Item -ItemType Directory -Path (Join-Path $Stage ('extract-' + [guid]::NewGuid()))
    try {
        Expand-Archive -LiteralPath $archive -DestinationPath $extract.FullName
        $source = $extract.FullName
        if ($package.Prefix) { $source = Join-Path $source $package.Prefix }
        if (!(Test-Path -LiteralPath (Join-Path $source $package.Binary) -PathType Leaf)) {
            throw "Missing executable in $($package.Name)."
        }
        Get-ChildItem -LiteralPath $source -Recurse -File -Filter '*.pdb' |
            ForEach-Object { Remove-Item -LiteralPath $_.FullName }
        if (Test-Path -LiteralPath $destination) {
            $resolved = Get-Item -LiteralPath $destination
            if ($resolved.FullName -ne $destination -or ($resolved.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                throw "Unexpected staging directory: $destination"
            }
            Remove-Item -LiteralPath $resolved.FullName -Recurse -Force
        }
        New-Item -ItemType Directory -Path $destination | Out-Null
        Get-ChildItem -LiteralPath $source -Force | Move-Item -Destination $destination
    } finally {
        Remove-Item -LiteralPath $extract.FullName -Recurse -Force
    }
}
