# Libron v0.25

Unmodified official font assets from
[nicoverbruggen/libron v0.25](https://github.com/nicoverbruggen/libron/releases/tag/v0.25),
source commit `46cf11c80a3b3a07c5b509b79d14c0e38feb1b35`.

`ttf/` contains the four desktop styles. `woff2/` contains the same four webfont
styles. `libron.css` and `TOKEN-CONTRACT.md` provide a reusable local UI integration
for Versora and an isolated Litora patch. No font has been converted, subsetted,
renamed or installed on the operating system; no upstream build script was run.

`SOURCE.json` records official archive URLs, publisher archive hashes, pinned
source version, per-file sizes/hashes and inspected static-face metadata.
`LICENSE` is the upstream SIL Open Font License 1.1; `COPYRIGHT` retains notices
for Newsreader, Readerly and Libron. Keep both files with every redistributed copy.

The Latin and Vietnamese sample glyphs are present in all four inspected TTF
faces. CJK and Thai samples require the application's existing fallback fonts.
This package supplies ordinary Latin UI and non-heading display typography.
Headings and the brand use a separate original Litora stack: Poppins, Space
Grotesk, Noto Sans TC, system-ui, sans-serif. Poppins is preferred; the existing
installed Noto Sans TC is the verified offline fallback when the first two are
unavailable. Existing heading sizes and weights are preserved. No remote
heading-font request is added. This package does not replace document/PDF fonts
such as the separately licensed Noto asset outside this directory.
