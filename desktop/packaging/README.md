# Windows native packaging

`Build-WindowsPackage.ps1` stages only an enumerated native payload and compiles
the existing NSIS 3.11 installation. It never installs or downloads tools, starts
the application, publishes a release, or writes built artifacts into Git.

Use the Windows x64 release executable built with static CRT and Tauri custom
protocol enabled. Supply its reviewed SHA256 and the actual checkout HEAD as
`SourceSha`. Production mode refuses any uncommitted tracked or untracked source.
Private user build/cache paths in the EXE are also refused. Build Rust with
`--remap-path-prefix=<local USERPROFILE>=/build-host` before release staging.
The current audited toolchain is Rust 1.97.1, MSVC 14.44 and the existing per-user
Tauri NSIS cache. The exact executable/tool locations are checked in the script.

```powershell
./Build-WindowsPackage.ps1 -Mode Production -SourceExe <release/versora.exe> `
  -ExpectedExeSha256 <reviewed-64-hex-digest> -SourceSha <clean-40-hex-HEAD> `
  -BuildId <unique-lowercase-build-id>
```

The formal installer targets `%LOCALAPPDATA%\Programs\Versora`, creates a current
user Start Menu shortcut and records a current user uninstall entry. Existing
Evergreen WebView2 is required; no runtime is downloaded. There are no file
associations or application/profile/cache writes in the installer. Uninstall
preserves both application data directories and unknown program/shortcut files.

Owned upgrades stage a new allowlisted payload, move the old known files to a
unique backup, and replace them by rename. Failed replacement restores known
files; failed restoration retains backups. Failed shortcut/registry integration
retains the completed program and backup directories for manual recovery rather
than reporting success. Reusing a transaction directory is refused. Program,
payload, transaction and shortcut paths reject reparse points; known file names
occupied by directories are also refused. A loaded or read-only executable is
preserved, with no process termination or reboot deletion.

`Export-ThirdPartyNotices.ps1` runs Cargo offline and verifies registry archive
SHA256 values against Cargo.lock. It collects cached published notice texts,
reviewed supplements, unchanged public crate source archives, Rust notices, NSIS
COPYING and approved font licenses. It does not collect developer profiles,
settings, build caches, credentials or user documents. See `notices/README.md`
for supplemental provenance requirements.

Test mode compiles the same transaction/removal code with a fixed target under
ignored `output/production-tests/<TestId>`, a private shortcut and a separate
HKCU namespace. It allows dirty source but identifies that fact and refuses
publication. Before any installation, review the emitted `build-plan.json`,
installer checksum and concrete paths.

```powershell
./Build-WindowsPackage.ps1 -Mode Test -SourceExe <release/versora.exe> `
  -ExpectedExeSha256 <reviewed-64-hex-digest> -SourceSha <actual-40-hex-HEAD> `
  -BuildId <unique-build-id> -TestId <unique-test-id>
./Invoke-IsolatedPackageTest.ps1 -Phase Install -PlanPath <build-plan.json> `
  -ExpectedInstallerSha256 <reviewed-installer-digest>
```

The test runner supports separate `Install`, `RefuseRunningApp`, `RefuseReparse`,
`Upgrade`, `Uninstall` and `Verify` phases. The caller launches and closes the
installed native application between phases using a separate scoped profile;
the runner never starts or stops the application. Running-app refusal requires
the installed executable to be running; reparse refusal temporarily places a
controlled junction at a known file name and restores that exact file afterward.
Unknown files, an unknown junction and a real current-user DPAPI empty credential
record are verified through install, upgrade and uninstall. Upgrade requires a
new compiled BuildId using the same TestId. Final evidence distinguishes checks
actually executed from `NOT_RUN` checks.
