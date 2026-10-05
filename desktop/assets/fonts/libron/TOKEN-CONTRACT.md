# Libron UI and separate heading token contract

Load `libron.css` from the copied font package. It defines four actual static
faces under the single family `Libron`: normal 400, italic 400, normal 700 and
italic 700. It does not use an installed font, remote URL or variable weight range.

Prefix the consuming application's ordinary Latin UI and non-heading display
stacks with `"Libron"`. Preserve the complete existing CJK/system fallback order.
Keep code/path monospace, source/translated document fonts and icon assets
separate. Do not apply a global `* { font-family: ... }` override.

Use these semantic weights for ordinary Libron UI:

| Role | Weight | Face |
| --- | --- | --- |
| Body, controls, captions | 400 | Regular |
| Body/control italic | 400 italic | Italic |
| Strong UI text | 700 | Bold |
| Strong italic | 700 italic | Bold Italic |

Set `font-synthesis:none` on the UI root; it is inherited. Replace Libron UI
requests for 500/600 with the intended actual 400/700 role instead of claiming a
variable font. Existing heading and monospace families retain their own weights.

Versora binds ordinary UI to `--f-ui`, non-heading display to `--f-display`, and
uses `--body-weight:400`, `--control-weight:400`, `--strong-weight:700`.
Its separate `--f-heading` restores Litora's original exact stack:
`"Poppins","Space Grotesk","Noto Sans TC",system-ui,sans-serif`.
Apply the heading token to h1-h6, `[role=heading]`, class-based page/section/dialog
titles, and the brand wordmark. Preserve their existing sizes and weights; this
is a font-family correction. General UI remains Libron. No remote Poppins font
request or new system-font installation is added. When Poppins and Space Grotesk
are unavailable, the existing installed Noto Sans TC paints the actual offline
heading glyphs; distinguish this fallback from the preferred family in evidence.
The `--f-mono` and document preview declarations remain unchanged.

Litora should bind the same separate roles to its equivalent tokens in an
isolated patch. Do not replace its explicit reader/book typography or globally
reset its display token, which also serves non-heading UI.

Before accepting a patch, verify actual native glyph use for all four Libron
faces and for heading regular/medium/bold weights, all supported locales,
buttons/small text/numbers, minimum-window layout, light/dark and real high-DPI
rendering. Computed family names alone do not prove which font painted glyphs.
Copy LICENSE, COPYRIGHT and SOURCE.json with the fonts; retain original font bytes.
