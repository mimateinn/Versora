# Packaging notice inputs

`Export-ThirdPartyNotices.ps1` is offline. Root provides reviewed supplemental
texts and `provenance.json` before a package is built. Existing cached crate
license files, all locked registry source archives, NSIS COPYING, Rust library
COPYRIGHT HTML and the approved Noto font OFL are collected automatically.

The JSON schema is:

```json
{
  "schema": 1,
  "entries": [
    {
      "id": "microsoft-webview2-sdk",
      "file": "Microsoft-WebView2-SDK.txt",
      "source": "https://www.nuget.org/packages/Microsoft.Web.WebView2/1.0.3650.58/License",
      "sha256": "exact SHA256 of the unchanged reviewed local text"
    }
  ]
}
```

Required IDs are `microsoft-webview2-sdk`, `rust-compiler-builtins`, and
`mit-standard-terms`. Additional pinned upstream notices can be included under
other unique IDs. File names are flat; every text is verified against its recorded
SHA256 before use. Source URLs identify provenance, not a download operation.

Shared MIT terms must be explicitly identified as shared/template terms. They
must not introduce invented copyright years or treat a placeholder as a package
copyright notice. Original Cargo metadata, source declarations and copyright
headers remain in each unchanged `.crate` archive. Three PDF crates explicitly
declare MIT but omit a standalone license upstream; the dependency inventory
records this metadata-based coverage choice.

The vendored Microsoft static WebView2 loader is separately licensed from Bill
Avery's Rust bindings. Its NuGet SDK version is 1.0.3650.58. A copied SDK license
must not be relabeled as the Rust bindings' MIT license.

Installed Rust `COPYRIGHT-library.html` includes LLVM exception references, but
does not identify compiler-builtins itself. The supplemental pinned notice at
Rust commit `8bab26f4f68e0e26f0bb7960be334d5b520ea452`, path
`library/compiler-builtins/LICENSE.txt`, retains its actual attribution.

Noto Sans CJK TC Regular 2.004 has actual embedded copyright 2014-2021 Adobe.
The font and its OFL are bundled; no font is installed into the operating system.
Libron 0.25 official static WOFF2 UI faces are checked against the approved
`assets/fonts/libron/SOURCE.json`. Its unchanged OFL, full upstream COPYRIGHT and
source provenance are included as separate flat files and in the aggregate
notices. The reusable TTF faces remain in published source, with no system font
installation or upstream font build script execution.
No project license is inferred from dependency notices.
