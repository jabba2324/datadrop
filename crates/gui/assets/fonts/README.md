# Fonts

The desktop app embeds these, under the SIL Open Font License 1.1 (the `OFL-*.txt` files):

- [Inter](https://rsms.me/inter/) 4.1: Regular, Medium and SemiBold, for the interface.
- [JetBrains Mono](https://www.jetbrains.com/lp/mono/) 2.304: Regular, for SQL and values.

Both are subset to drop their Private Use Area glyphs (U+E000–U+F8FF), where the icon
font (Phosphor) keeps its icons; nothing else is changed:

```sh
pyftsubset Inter-Regular.ttf --unicodes="U+0000-DFFF,U+F900-10FFFF" --layout-features='*' \
  --glyph-names --name-IDs='*' --name-languages='*' --notdef-outline --output-file=out.ttf
```
