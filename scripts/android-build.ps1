<#
.SYNOPSIS
  Builds the B-Anime Android APK for arm64-v8a (ARMv8 and newer only).

.DESCRIPTION
  Needs, once per machine (see docs/android.md):
    - Android SDK with NDK 27 (ANDROID_HOME set; Android Studio installs both)
    - Java 17 (JAVA_HOME set)
    - rustup target add aarch64-linux-android
    - libclang, for the QuickJS bindings:
        python -m pip install libclang --target "$env:LOCALAPPDATA\libclang"
      or set LIBCLANG_PATH to any folder holding libclang.dll.

.EXAMPLE
  scripts\android-build.ps1          # release APK
  scripts\android-build.ps1 -Debug   # debug APK (faster, unsigned-debug key)
#>
param([switch]$Debug)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

if (-not $env:ANDROID_HOME) { throw "ANDROID_HOME isn't set. Install the Android SDK (Android Studio) and set it." }
if (-not $env:NDK_HOME) {
  $ndk = Get-ChildItem "$env:ANDROID_HOME\ndk" -Directory -ErrorAction SilentlyContinue | Sort-Object Name -Descending | Select-Object -First 1
  if (-not $ndk) { throw "No NDK under $env:ANDROID_HOME\ndk. Install one from Android Studio's SDK Manager." }
  $env:NDK_HOME = $ndk.FullName
}
if (-not $env:LIBCLANG_PATH) { $env:LIBCLANG_PATH = "$env:LOCALAPPDATA\libclang\clang\native" }
if (-not (Test-Path "$env:LIBCLANG_PATH\libclang.dll")) {
  throw "libclang.dll not found in $env:LIBCLANG_PATH. Run: python -m pip install libclang --target `"$env:LOCALAPPDATA\libclang`""
}

# bindgen parses QuickJS's headers against the NDK's Android sysroot.
$tc = (Join-Path $env:NDK_HOME "toolchains\llvm\prebuilt\windows-x86_64") -replace '\\', '/'
$clangInc = Get-ChildItem "$tc/lib/clang" -Directory | Sort-Object Name -Descending | Select-Object -First 1
$env:BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android = "--sysroot=$tc/sysroot -isystem $(($clangInc.FullName) -replace '\\', '/')/include"

Write-Host "NDK:      $env:NDK_HOME"
Write-Host "libclang: $env:LIBCLANG_PATH"

Push-Location "$root\app"
try {
  $buildArgs = @("tauri", "android", "build", "--target", "aarch64", "--apk")
  if ($Debug) { $buildArgs += "--debug" }
  # Tauri logs progress to stderr; Windows PowerShell would treat that as a failure under "Stop".
  # The exit code decides instead.
  $ErrorActionPreference = "Continue"
  & npx @buildArgs
  $code = $LASTEXITCODE
  $ErrorActionPreference = "Stop"
  if ($code -ne 0) { throw "Android build failed (exit code $code)." }
} finally { Pop-Location }

$out = Get-ChildItem "$root\app\src-tauri\gen\android\app\build\outputs\apk" -Recurse -Filter *.apk | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($out) { Write-Host "APK: $($out.FullName) ($([math]::Round($out.Length / 1MB, 1)) MB)" }
