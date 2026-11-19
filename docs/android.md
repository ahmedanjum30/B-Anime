# Building for Android

B-Anime's Android build targets **arm64-v8a only** (64-bit ARMv8 phones and newer). Older 32-bit
(armeabi-v7a) and x86 devices aren't supported.

## One-time setup

1. **Android Studio**, with these from its SDK Manager:
   - an Android SDK platform (36 or newer),
   - **NDK 27** (Side by side).

   Set `ANDROID_HOME` to the SDK folder, usually `%LOCALAPPDATA%\Android\Sdk`.
2. **Java 17**, with `JAVA_HOME` set. Android Studio's bundled JBR also works.
3. **Rust's Android target:**

   ```powershell
   rustup target add aarch64-linux-android
   ```
4. **libclang.** rquickjs (the extension engine) ships no QuickJS bindings for Android, so they're
   generated during the build, which needs `libclang.dll`. The NDK's Windows build doesn't include
   it. The quickest way to get it without installing LLVM is the Python package, kept in its own
   folder:

   ```powershell
   python -m pip install libclang --target "$env:LOCALAPPDATA\libclang"
   ```

   Or install LLVM and set `LIBCLANG_PATH` to the folder holding `libclang.dll`.

## Build

```powershell
scripts\android-build.ps1          # release APK
scripts\android-build.ps1 -Debug   # debug APK
```

The script finds the NDK, points bindgen at the NDK's Android headers, runs
`tauri android build --target aarch64 --apk`, and prints where the APK ended up (under
`app\src-tauri\gen\android\app\build\outputs\apk\`).

## Signing

Android only installs signed APKs. Release builds are signed when
`app\src-tauri\gen\android\keystore.properties` exists (it's git-ignored):

```properties
storeFile=C:/Users/<you>/.android/banime-release.jks
storePassword=...
keyAlias=banime
keyPassword=...
```

Without it, the release APK comes out unsigned (`app-universal-release-unsigned.apk`).

**Back up the keystore and its password.** Every update must be signed with the same key: if it's
lost, installed copies can't be updated and users have to uninstall first (losing their data).

The first build takes 10 to 20 minutes (every crate compiles for Android and Gradle downloads its
tools); later builds reuse both.

## What differs from desktop

- **Layout:** under 720px wide the left rail becomes a bottom bar (Home, Library, Manga, Discover,
  More), and pages switch to one column. The same CSS serves desktop windows that narrow.
- **Window calls:** maximizing the window and attaching file pickers to their parent window are
  desktop-only (`#[cfg(desktop)]`).
- **Credentials (not done yet):** the `keyring` crate has no Android store and falls back to an
  in-memory mock that doesn't keep anything, so the AniList sign-in won't stick on Android. It
  needs an Android credential store (for example the Android Keystore through a Tauri plugin).
