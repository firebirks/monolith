# Assets of the design system

Compiled into the binary by `src/fonts.rs`. Nothing here is loaded at run
time from anywhere else (`docs/adr/0012-design-system.md`).

| File | What | Source | Licence |
| --- | --- | --- | --- |
| `fonts/Inter-Regular.ttf`, `Inter-Medium.ttf`, `Inter-SemiBold.ttf` | Inter 4.1, the static instances of `extras/ttf/` | https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip (SHA-256 of the archive `9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e`) | SIL Open Font License 1.1, `licenses/Inter-OFL.txt` |
| `fonts/JetBrainsMono-Regular.ttf` | JetBrains Mono 2.304, `fonts/ttf/` | https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip (`6f6376c6ed2960ea8a963cd7387ec9d76e3f629125bc33d1fdcd7eb7012f7bbf`) | SIL Open Font License 1.1, `licenses/JetBrainsMono-OFL.txt` |
| `fonts/LucideMonolith.ttf` | the icon font of Lucide 1.57.0 (`font/lucide.ttf` of the npm package `lucide-static`) cut down with fontTools 4.60.1 to the 68 icons of `src/icons.rs`, at their code points | https://registry.npmjs.org/lucide-static/-/lucide-static-1.57.0.tgz (`afc8602c5231ce64ee227286f93772d0438eddd3b947b645bc14bfd1e49da518`) | ISC, and MIT for the icons inherited from Feather, `licenses/Lucide-ISC.txt` |
| `fonts/NotoEmoji-Regular.ttf` | Noto Emoji, monochrome, as distributed in the crate `epaint_default_fonts` 0.32.3 | crates.io, `epaint_default_fonts-0.32.3/fonts/` | SIL Open Font License 1.1, `licenses/NotoEmoji-OFL.txt` |

SHA-256 of the files here:

    40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82  fonts/Inter-Regular.ttf
    97ad806f526e41546d46365bb3a393145f75b7b1568913db74549ad8b8dba872  fonts/Inter-Medium.ttf
    78a843fade9d4612a5567302fb595b56976eb5fcebf4fea5a5912d638bafcde3  fonts/Inter-SemiBold.ttf
    a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f  fonts/JetBrainsMono-Regular.ttf
    0e618c3158657b3e09d5fffa0e770bea7715eab5d57f523400a9b91ae0ecfaa3  fonts/LucideMonolith.ttf
    415dc6290378574135b64c808dc640c1df7531973290c4970c51fdeb849cb0c5  fonts/NotoEmoji-Regular.ttf

The Open Font License allows the fonts to be bundled with software and
redistributed with it, with the licence, as long as they are not sold on
their own; the names Inter and JetBrains Mono are not used for the cut-down
or changed files, which these are not.
