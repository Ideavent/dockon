[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') {
    throw 'Build the Windows installer on a Windows host.'
}

$repo = Split-Path -Parent $PSScriptRoot
$stage = Join-Path $repo '.local/windows-installer'
$target = 'x86_64-pc-windows-msvc'
$templateUri = 'https://raw.githubusercontent.com/tauri-apps/tauri/tauri-cli-v2.12.4/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi'
$templateHash = 'D00089D3B1A2C4A30D7B6E098079100CB8F25E980F84D4515ACA4597B95E967D'

Push-Location $repo
try {
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    $upstream = Join-Path $stage 'upstream.nsi'
    Invoke-WebRequest -Uri $templateUri -OutFile $upstream
    if ((Get-FileHash -LiteralPath $upstream).Hash -ne $templateHash) {
        throw 'The upstream Tauri NSIS template hash does not match.'
    }

    $template = Get-Content -LiteralPath $upstream -Raw
    $template = $template.Replace('!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive' + "`n" + '!insertmacro MUI_PAGE_DIRECTORY', '')
    $template = $template.Replace('Function .onInit' + "`n", 'Function .onInit' + "`n" + '  StrCpy $INSTDIR "C:\dockon"' + "`n")
    $template = $template.Replace('!define MUI_PAGE_CUSTOMFUNCTION_SHOW un.ConfirmShow', '')
    $template = $template.Replace('!define MUI_PAGE_CUSTOMFUNCTION_LEAVE un.ConfirmLeave', '')
    $secureDirectory = @'
  ; Secure a dedicated application directory before installing executable files.
  ReadRegStr $R0 SHCTX "${MANUPRODUCTKEY}" ""
  System::Call 'kernel32::GetFileAttributesW(w "$INSTDIR") i.r1'
  ${If} $1 != -1
    IntOp $2 $1 & 0x410
    ${If} $2 != 0x10
    ${OrIf} $R0 != $INSTDIR
      Abort "C:\dockon already exists and is not a registered Dockon installation."
    ${EndIf}
  ${Else}
    System::Call 'advapi32::ConvertStringSecurityDescriptorToSecurityDescriptorW(w "O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FRFX;;;BU)", i 1, *p .r3, p 0) i.r0'
    ${If} $0 = 0
      Abort "Could not prepare the Dockon installation directory permissions."
    ${EndIf}
    System::Call '*(i 12, p r3, i 0) p.r4'
    System::Call 'kernel32::CreateDirectoryW(w "$INSTDIR", p r4) i.r0'
    System::Free $4
    System::Call 'kernel32::LocalFree(p r3)'
    ${If} $0 = 0
      Abort "Could not create the protected Dockon installation directory."
    ${EndIf}
  ${EndIf}
'@
    $template = $template.Replace('Section Install' + "`n", 'Section Install' + "`n" + $secureDirectory + "`n")
    $workspace = @'
  System::Call 'kernel32::GetFileAttributesW(w "$INSTDIR\www") i.r1'
  ${If} $1 = -1
    ClearErrors
    CreateDirectory "$INSTDIR\www"
    IfErrors 0 +2
    Abort "Could not create the Dockon www directory."
    nsExec::Exec '"$SYSDIR\icacls.exe" "$INSTDIR\www" /grant:r "*S-1-5-32-545:(OI)(CI)M"'
    Pop $0
    ${If} $0 != 0
      Abort "Could not grant access to the Dockon www directory."
    ${EndIf}
  ${EndIf}
'@
    $template = $template.Replace('  ; Copy main executable' + "`n" + '  File', $workspace + "`n" + '  ; Copy main executable' + "`n" + '  File')
    Set-Content -LiteralPath (Join-Path $stage 'installer.nsi') -Value $template -Encoding utf8
    & "$PSScriptRoot/Stage-WindowsStack.ps1" -Stage $stage

    cargo build --locked --release --target $target -p dockon-daemon --bin dockon-daemon --bin dockonctl -p dockon-router --bin dockon-hosts-helper
    if ($LASTEXITCODE -ne 0) { throw 'Building the Dockon sidecars failed.' }
    foreach ($binary in 'dockon-daemon', 'dockon-hosts-helper', 'dockonctl') {
        Copy-Item -LiteralPath "target/$target/release/$binary.exe" -Destination (Join-Path $stage "$binary-$target.exe")
    }

    pnpm --filter '@dockon/desktop' tauri build --ci --target $target --config src-tauri/tauri.windows-installer.json -- --locked
    if ($LASTEXITCODE -ne 0) { throw 'Building the Dockon installer failed.' }

    $version = (Get-Content -Raw 'apps/desktop/src-tauri/tauri.conf.json' | ConvertFrom-Json).version
    $package = Get-Item -LiteralPath "target/$target/release/bundle/nsis/dockon_${version}_x64-setup.exe"
    $hash = (Get-FileHash -LiteralPath $package.FullName).Hash.ToLowerInvariant()
    Set-Content -LiteralPath "$($package.FullName).sha256" -Value "$hash  $($package.Name)" -Encoding ascii
    $package
}
finally {
    Pop-Location
}
