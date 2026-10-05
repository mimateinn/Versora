<p align="center">
  <img src="docs/brand/versora.png" width="112" height="112" alt="Versora icon">
</p>

<h1 align="center">Versora</h1>

<p align="center">
Translate files on your own computer. Drop one file, a folder, or a zip — you get a matching translated file for each input, in the same folder shape.
</p>

<p align="center">Previously called Smart File Translation System. Existing links still work.</p>

## Related apps

<table>
  <tr>
    <td width="50%" align="center" valign="top">
      <a href="https://github.com/mimateinn/Versora"><img src="docs/brand/versora.png" width="64" height="64" alt="Versora icon"></a>
      <h3><a href="https://github.com/mimateinn/Versora">Versora</a></h3>
      <p>Translate files on your own computer, keeping the original folder structure.</p>
    </td>
    <td width="50%" align="center" valign="top">
      <a href="https://github.com/mimateinn/Litora"><img src="docs/brand/litora.png" width="64" height="64" alt="Litora icon"></a>
      <h3><a href="https://github.com/mimateinn/Litora">Litora</a></h3>
      <p>A desktop library and reader for your books.</p>
    </td>
  </tr>
</table>

## Windows desktop download

[Download Versora v0.3.0-preview.1 for Windows x64](https://github.com/mimateinn/Versora/releases/tag/v0.3.0-preview.1)

Choose `Versora-0.3.0-preview.1-win32-x64-setup.exe` under **Assets**. This is an unsigned prerelease: see the release notes and `SHA256SUMS.txt` before installing. The source ZIP from the green **Code** button is for development.

Versora now opens in its own Windows desktop window. The translation engine, file handling and settings are written in Rust; the desktop window uses Tauri and WebView2. Running the installed application does not require Python, Node.js or a local web server.

The native window uses a single 48 px header with Minimize, Maximize/Restore and Close controls. Drag a blank part of the header to move the window; double-click it to maximize or restore. Closing warns about unsaved edits, requests cancellation of an active job and waits for pending writes before exit.

### Install and start

1. Use Windows 10 or 11, 64-bit, with the Microsoft Edge WebView2 Evergreen Runtime already installed. The installer checks this prerequisite; it does not download a runtime.
2. Download the setup file and checksum file from the release above. To compare the setup file's SHA-256, run `Get-FileHash -Algorithm SHA256 .\Versora-0.3.0-preview.1-win32-x64-setup.exe` in PowerShell.
3. Run the installer. It installs for your current Windows account in `%LOCALAPPDATA%\Programs\Versora`.
4. Open **Versora** from the installed shortcut.
5. In **Settings → Translators**, add a supported developer API key, or select an already installed and signed-in official native CLI executable. Choose the translator and model, then return to **Translate**.

No provider credentials are included. API translation sends selected text and any relevant glossary/purpose instructions to the provider you choose, and may incur that provider's charges.

## Translate files

Choose the source and target languages and a purpose, then select files, a folder or a ZIP with the native file picker or drag and drop. Press **Translate**.

Each supported input gets a separate translated output, with relative folders retained for folder and ZIP jobs. Originals are left unchanged. Finished files can be opened, saved with the native **Save As** dialog, or exported together as a ZIP within an 80 MB combined-file and 84 MB archive limit. Five small changed PDFs can already exceed the combined limit because each embeds its font; their individual saved files remain available. Existing output names get a new suffix rather than silently overwriting a file.

**Cancel** keeps files that have already been saved. **Retry** runs only failed or unfinished files. File concurrency is configurable from 1–16, with a separate limit for each translator.

Settings include purposes, translator order and models, project glossaries, 12 interface languages, two Litora-style modes (Standard and Opal) with a light or dark tone; press the active mode again to switch tones and reduced motion.

Litora-style 12 px scrollbars show a subdued thumb at rest, a stronger thumb over the scrolling area, and accent feedback (iridescent in Opal) when hovering over or dragging the thumb. Theme changes animate colors; page changes use short entry transitions. **Reduce motion** removes movement and theme transitions while retaining a brief page fade; the Windows reduced-motion preference disables the animations. The app version stays visible in the bottom footer while content scrolls.

The default ordinary Latin interface font is locally bundled [Libron v0.25](https://github.com/nicoverbruggen/libron/releases/tag/v0.25), with its actual Regular, Bold, Italic and Bold Italic styles under the SIL Open Font License 1.1. Main/page headings, class-based heading labels and the brand wordmark use a separate heading token with Litora's original stack: Poppins, Space Grotesk, Noto Sans TC, system-ui, sans-serif. The tested offline Windows host has Noto Sans TC available; its actual Medium/Bold glyphs are verified separately from CSS family declarations. Poppins is a preferred family rather than a newly bundled or downloaded font. Existing CJK/script fallbacks and functional monospace styling remain, and no system font is installed. Interface font selection does not change original or translated document fonts; PDF reflow uses the separately bundled Noto font described below.

### Formats

| Input | Translation behavior |
| --- | --- |
| TXT, Markdown (`.md`, `.markdown`) | Text and paragraphs; Markdown syntax and protected code are retained. |
| DOCX, XLSX | Document text and spreadsheet string cells; review complex layouts after translation. |
| JSON, CSV, TSV, YAML | Text values/cells while retaining the file's structural fields. |
| PO/POT, XLIFF (`.xlf`, `.xliff`) | Translation text with message identifiers and inline codes protected. |
| HTML/HTM, SRT, VTT | Visible text or subtitle text; markup, cue identifiers and timings are retained. |
| PDF | Extractable text becomes a newly laid-out translated PDF; the original page layout is not reproduced. |
| Lua, JavaScript, TypeScript, GDScript | **Game-text mode** selects player-facing string literals; review its selection and the output. |

Text-based files must use UTF-8. Jobs are bounded to 400 files and 80 MB of input/expanded ZIP content. Unsupported, malformed or oversized files report an error or skip instead of claiming completion.

Structured-text extraction is conservative: identifier-like values, URLs, filenames and numbers may stay unchanged. Review the selected text and output, especially single-word labels and game strings. A file containing no selected text can be saved byte-for-byte without contacting a translator.

CSV/TSV writeback retains UTF-8 BOMs, record endings and untouched cells, with an 80 MB output cap. YAML supports a bounded subset of YAML 1.2; changed output normalizes comments, quoting and anchor presentation. Plain `Null`/`NULL`, signed radix integers and integers outside the supported i64 range are rejected rather than silently changing their types. Use `null`/`~`, supported i64 integer forms, or quoted/`!!str` strings as appropriate. Custom tags, merge keys and explicit non-string tags on quoted/block scalars are rejected. Mapping keys and numeric/boolean/null values are validated after serialization.

PDF support is text extraction and A4 text reflow. Changed PDFs embed the complete Noto Sans CJK TC font (about 16.4 MB per PDF). It does not perform OCR or preserve source graphics, typography, annotations, forms or exact layout. Encrypted PDFs must be decrypted first. Missing glyphs report an error; arbitrary script shaping is not certified. Input/text/output guards do not certify a parser memory ceiling for compressed PDF objects. Office and game-text adapters also do not promise universal format fidelity.

## Translators and current limits

Translator status text follows the selected interface language. Finding a native CLI, reporting signed in and being eligible to attempt a request are separate from **Connection tested**. The first visit to **Settings → Translators** in each app session automatically checks the installed CLIs' version and login/status information. It does not sign in or start translation, and changing window focus does not repeat it. Use **Re-check** to refresh the local status.

**Test connection** is available only when a translator is eligible for an attempt. It makes a real provider request and may consume quota. Only a recent successful explicit test marks the transport verified; this does not certify translation quality or CLI tool isolation.

- Developer API adapters: **OpenAI, Anthropic, Gemini and xAI**. Custom base URLs must pass the public HTTPS host allowlist; arbitrary local proxies and chat websites are not supported.
- CLI adapters: **Claude Code, Codex CLI and Grok CLI**, when an official native executable is already installed and signed in. The app does not run `.cmd`/`.bat` shims; known official Codex installations may resolve to their native executable.
- Paid/live provider compatibility, live CLI translation and model translation quality remain **NOT_RUN** for this build. Offline Demo and controlled protocol tests are not real-model translation.
- Controlled Windows process-tree cancellation has been tested. Live CLI tool isolation remains **NOT_RUN**, and full no-tool isolation is not certified: Codex can retain shell tools and inherited MCP configuration; Grok home isolation is not implemented. Parser/writeback work and external provider activity are not guaranteed to stop instantly.
- The native preview builds on the latest PR4 UI work. PR4 remains a separate, unmerged pull request; this prerelease does not represent its acceptance.
- Windows 11 Snap Layouts on hover over the app's HTML Maximize button have not been verified.
- Windows x64 is the packaged target. A clean Windows VM, other operating systems, comprehensive screen-reader journeys and every document layout have not been validated.

For an explicit offline UI check, set `SFTS_DEMO=1` before launching Versora. **Demo** adds target-language tags to test text; it does not translate and is clearly identified in the app.

## Local settings and credentials

The installation directory contains application files. Your separate data directory is `%LOCALAPPDATA%\Versora`: settings, project glossaries, custom prompts, completed outputs and update state live there. The default output location is `%LOCALAPPDATA%\Versora\data\outputs`; you can choose another output folder.

Saved provider credentials are encrypted with Windows user-scope DPAPI in `credentials.dpapi` and are never returned to the interface as plaintext. They remain tied to the Windows account; copying this file to another account is not a credential migration.

Upgrade and uninstall preserve the user-data directory and the separate WebView2 cache at `%LOCALAPPDATA%\com.mimateinn.versora`. The legacy import option copies recognized settings, glossaries and outputs without changing the source folder or replacing existing native data.

## Updates

**Settings → Updates** can check official GitHub release metadata and open the download page. Startup and periodic checks are configurable while the app is running.

Automatic package downloading, signed installation and recovery are disabled in this preview. Install updates manually from the official release page. A SHA-256 checksum checks file integrity; it is not a publisher signature.

## Build the native desktop source

Use the [source at the native release tag](https://github.com/mimateinn/Versora/tree/v0.3.0-preview.1/desktop). The historical root `start.bat`/`start.sh` and Python code belong to the browser application, rather than the native build.

On the provisioned Windows build host, with the locked crates available, installed Rust 1.97.1 MSVC, Visual Studio 2022 Build Tools and Windows SDK:

```powershell
cd desktop
.\scripts\Invoke-Rust.ps1 -CargoArgs @('test', '--offline', '--locked', '--workspace')
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS = '-C target-feature=+crt-static --remap-path-prefix=' + $env:USERPROFILE + '=/build-host --remap-path-prefix=' + $env:USERPROFILE.Replace('\','/') + '=/build-host'
.\scripts\Invoke-Rust.ps1 -CargoArgs @('build', '--offline', '--locked', '--release', '-p', 'versora-desktop', '--features', 'custom-protocol')
```

The compiler wrapper selects the audited toolchain paths; configure those paths for another build host. The native frontend is bundled local content and has no separate Node build step. Release packaging uses NSIS, an allowlisted payload and recorded checksums; generated binaries, test profiles and user data must stay out of source Git.

See [the native feature map](https://github.com/mimateinn/Versora/blob/v0.3.0-preview.1/desktop/FEATURE_MAP.md) and [updater contract](https://github.com/mimateinn/Versora/blob/v0.3.0-preview.1/desktop/UPDATE-ADAPTER.md) for source boundaries. Third-party license notices and required source archives accompany the installer. The repository does not declare a license for the application itself.

## Earlier downloads

[v0.2.0-preview.1](https://github.com/mimateinn/Versora/releases/tag/v0.2.0-preview.1) remains available as the historical **Browser preview**. Its Python/browser launcher and local `.env` behavior are separate from this Rust desktop version.
