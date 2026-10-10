# Primus reference

What the scanner replaces, captured before the subscription lapses (~May 2027).

| file | what |
|---|---|
| `Primus_Filters_MASTER_2026-09-25.md` | Every filter, static list and custom field, verbatim as loaded, with clause notes, the regression set and the fix queue. Copied unchanged from the user's master (file name says 09-05; its header says version 2026-09-24, and it carries the 2026-09-25 LowFloat_HOD_Break_Up rebuild and the set-wide stale-reference note in §4A). **The source of truth for `filters/`.** |
| `screens/*.png` | The four Primus screens as laid out on 2026-09-25: which panels exist, their columns, green = long / red = short, statics carry a member count, charts are link-grouped to panels. The reference for the display side. |

Screens:

- `left-sharp-move.png`: the sharp-move family (Spike, Pop/Drop Sensitive, Medium/EXT, High_ATR), each with `ATR_POS` / `ATR_NEG`.
- `right-hold-breakout.png`: hold and breakout families, M3–M8, LowFloat/HOD breaks. Also shows a
  `CatchALL_LF_HOD` panel that the master does not list.
- `top-flush-reversal.png`: Open_Flush, Blowoff and Quiet reversals.
- `fourth-area-contra-statics.png`: the Contra panels, grouped four to a window with a `Filter Name` column, and the statics (Movers A-lists sorted by `RNK_UP`/`RNK_DWN`, imbalances, halts, NYSE IND, PM coils, LowFloat movers).

Not in the master, so not yet translated: the 14 Contra expressions (§4C describes them but doesn't
quote them), the Breakout_Hold pair's full expression (§4.25 is prose), PM_LowFloat_Coil_Up (§5.6 is
prose), the plain `Halted` static, and `CatchALL_LF_HOD`.
