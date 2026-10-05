# Libron UI token contract

Load `libron.css` from the copied font package. It defines four actual static
faces under the single family `Libron`: normal 400, italic 400, normal 700 and
italic 700. It does not use an installed font, remote URL or variable weight range.

Prefix the consuming application's existing Latin UI and display stacks with
`"Libron"`. Preserve the complete existing CJK/system fallback order. Keep its
code/path monospace token, source/translated document fonts and icon assets
separate. Do not apply a global `* { font-family: ... }` override.

Use these semantic weights:

| Role | Weight | Face |
| --- | --- | --- |
| Body, controls, captions | 400 | Regular |
| Body/control italic | 400 italic | Italic |
| Strong text, headings | 700 | Bold |
| Strong italic | 700 italic | Bold Italic |

Set `font-synthesis:none` on the UI root; it is inherited. Replace UI requests for
500/600 with the intended actual 400/700 role instead of claiming a variable
font. Leave existing declarations that explicitly use a different monospace
family under that family's own contract.

Versora binds this contract to `--f-ui`, `--f-display`, `--body-weight:400`,
`--control-weight:400` and `--strong-weight:700`. Its `--f-mono` and document
preview font declarations are preserved. Litora should bind the same semantic
contract to its existing equivalent tokens in an isolated patch, preserving its
own fallback families and document typography.

Before accepting another application's patch, verify actual native font use
(font readiness plus rendered platform-font evidence), all supported locales,
buttons/small text/numbers, minimum-window layout, light/dark and real high-DPI
rendering. Computed family names alone do not prove which font painted glyphs.
Copy LICENSE, COPYRIGHT and SOURCE.json with the fonts; retain original font bytes.
