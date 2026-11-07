# Developing Gnirehtet NG

_Modified for Gnirehtet NG by the Gnirehtet NG contributors._

## Windows build setup

Use Windows 11 x64 and Visual Studio 2022 with MSBuild, MSVC v143, the UWP C++ component group, and Windows SDK 10.0.26100.0. The repository uses Rust 1.98.0, Java 17, Android SDK 36, and Gradle Wrapper 8.11.1. The PowerShell setup scripts keep their downloads and caches inside the checkout. Run these from the repository root in Windows PowerShell.

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\bootstrap-rust.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\bootstrap-android.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\bootstrap-gradle.ps1
```

The Android bootstrap installs the project-local SDK and accepts its license prompts. Read and accept the [Android SDK terms](https://developer.android.com/studio/terms) before running it. The Windows build downloads the pinned NuGet CLI on first use and verifies its Microsoft signature.

## Build and test

Close Gnirehtet NG and unplug the phone before the offline checks. If ADB is cold, start it through the helper before running the build from a terminal that captures subprocess output.

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\ensure-adb.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-m3.ps1 -Configuration Release
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-m3.ps1 -Configuration Release
```

`build-m3.ps1` builds Android and Rust via `build-m2.ps1`, compiles WinUI, and runs a no-device smoke test. Outputs go to `out/m2` and `out/m3/x64/Release`. For a separate development shell, run `. .\scripts\dev-shell.ps1` to set up the current PowerShell process. Run `cargo test --locked --manifest-path .\relay-rust\Cargo.toml` for Rust or `.\gradlew.bat :app:check` for Android.

For an end-to-end check, connect one authorized device, approve the VPN prompt, turn off Wi-Fi and mobile data, and verify real traffic through the tunnel. Check **Stop**, USB reconnect, and GUI exit on the intended Windows and Android builds.

## Build the portable release

1. Update `productVersion` and increment `androidVersionCode` in `config/product.properties`. Build all components together, since Android and Rust both read this file.
2. Set up a private Android release keystore. Use the same signing certificate for subsequent APK updates. Android requires removing the old app before installing an APK signed with a different certificate, which clears its local settings.
3. Place the signing properties in the local Gradle user home, `.gradle-home/gradle.properties`. This file is ignored by Git. Use forward slashes in the absolute keystore path.

   ```properties
   RELEASE_STORE_FILE=C:/path/outside-the-repository/gnirehtet-ng-release.jks
   RELEASE_STORE_PASSWORD=<store password>
   RELEASE_KEY_ALIAS=<key alias>
   RELEASE_KEY_PASSWORD=<key password>
   ```

4. Build and package from the repository root.

   ```powershell
   powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-m3.ps1 -Configuration Release
   . .\scripts\dev-shell.ps1
   .\gradlew.bat --no-daemon :app:assembleRelease
   .\scripts\package-release.ps1
   ```

5. Extract `out/release/GnirehtetNG-<version>-windows-x64.zip` to a new folder and smoke-test it on Windows 11. In GitHub Releases, create a tag such as `v0.2.0` at the tested commit and attach the portable ZIP as a release asset. Include the main changes and known limitations in the release notes.

### Redistribution

Use a newly built ZIP for each release. It includes the project's LICENSE and NOTICE,
the Android Platform-Tools notice under `adb/`, and the license and notice files
from the restored Windows runtime packages under `licenses/`.

- Review the [Android SDK terms](https://developer.android.com/studio/terms), sections
  3.4 and 3.5, and the component notices for the ADB binaries you ship.
- Read section 3 of the bundled Windows App SDK license. Its redistribution terms
  require distributors and external end users to agree to terms that protect
  Microsoft and its code at least as much as the SDK terms. Arrange that agreement
  through the release distribution channel.
- Use the original x64 VC++ runtime DLLs from the Visual Studio `VC\Redist` tree.
  Check the terms for the installed Visual Studio edition and the
  [Visual Studio 2022 redistribution list](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution)
  before distribution.

Keep the ZIP, APK, signing files, toolchains, and diagnostic exports out of Git. The source checkout retains the Gradle Wrapper and Cargo lockfile so contributors can use the same build inputs.

See the [system overview](docs/architecture/system.md) and the [upstream relay internals](https://github.com/Genymobile/gnirehtet/blob/1eb2e58bc91e268dba62561d11e3bebd9b100883/DEVELOP.md) for data-plane background.
