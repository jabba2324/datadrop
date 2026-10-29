# DataDrop 0.3 design

Mockups for the 0.3 UI refresh: every screen in the app, in dark and light. They show what the app does today; features it doesn't have yet are marked below, each with an issue.

- `datadrop-0.3.figcraft` holds them, one page per theme. Open it in [FigCraft](https://github.com/storytold/figcraft).
- `png/dark/` and `png/light/` have each screen at 1440×900.
- `overview-*.png` show all the screens on one sheet.

The file is generated, so change the scripts rather than the file. `ddkit.py` has the colour tokens and shared parts (buttons, inputs, the title bar, explorer and status bar). `screens.py` has one function per screen.

```sh
python3 screens.py datadrop-0.3.figcraft > /tmp/ui.jsonl
figcraft-cli run /tmp/ui.jsonl --no-system-fonts
figcraft-cli export datadrop-0.3.figcraft -o png/dark --format png --page Dark --no-system-fonts
figcraft-cli export datadrop-0.3.figcraft -o png/light --format png --page Light --no-system-fonts
```

## Screens

| # | Screen | Covers |
|---|--------|--------|
| 00 | Style guide | Colour tokens (dark/light), syntax colours, type, controls, badges, principles |
| 01 | Welcome | First launch, with no connections |
| 02 | Workspace: results | Editor, autocomplete, parameters hint, grid, filter chip, an edited cell, the unsaved-edits bar |
| 03 | Workspace: running | Run in progress, Stop, skeleton rows |
| 04 | Query error | Error underlined in the editor, the Messages tab, a toast |
| 05 | Query plan | Analyzed/estimated tree, with the slowest step marked |
| 06 | Manual commit and find | Transaction bar, find and replace |
| 07 | Review edits | Grid edits as SQL before saving |
| 08 | Query parameters | Values for `:name` placeholders |
| 09 | Command palette | ⌘K: tables, commands, saved queries |
| 10 | History | History sidebar and the details of a query |
| 11 | New connection | Server databases, keychain, SSH tunnel |
| 12 | Connection: database file | SQLite / DuckDB |
| 13 | Trust SSH host | First connection to a host key |
| 14 | Table editor | Columns with pending ALTERs |
| 15 | Object DDL | The DDL tab |
| 16 | Import | CSV/TSV/JSON column mapping |
| 17 | Compare schemas | Schema diff and its migration |
| 18 | ER diagram | Tables and their relations |
| 19 | Settings | Theme, editor text size, row limit |
| 20 | Confirmations | Every confirm dialog |
| 21 | Menus, toasts and states | The explorer and grid context menus, toasts, banners, empty states, status |
| 22 | Several statements | One result per statement, rows-not-shown warning, the WHERE filter |
| 23 | Editing rows | Edit mode, a changed cell, deleted and added rows, the value editor |
| 24 | Connection: URL, TLS and SSH | URL mode, CA certificate, where passwords are kept, SSH agent / password / key file |
| 25 | Table editor: foreign keys | On delete, and validation errors |
| 26 | Table editor: indexes and checks | Indexes (unique, primary), check constraints |
| 27 | New table | Creating a table, and validation errors |

## Implementing in egui

- **Colours:** take the tokens from `DARK` / `LIGHT` in `ddkit.py` and set them on `egui::Visuals`; replace the defaults rather than tweaking them.
- **Fonts:** Inter for the UI and a monospace face for SQL and numbers. The mockups use Liberation Mono; JetBrains Mono would suit the app.
- **Shapes:** radii are 6 px for controls, 8–10 px for cards and 14 px for sheets. Borders are 1 px. Spacing is on a 4/8 px grid.
- **Icons:** keep `egui_phosphor` for icons; the mockups draw simple stand-ins.

## Not in the app yet

These appear in the designs, but the app doesn't have them yet:

| Feature | Screens | Issue |
|---------|---------|-------|
| Welcome screen and recent files | 01 | #1 |
| Command palette (⌘K) | 09 | #2 |
| Errors shown in the editor, Jump to line, "did you mean" | 04 | #3 |
| Messages view with server notices | 04 | #4 |
| Slowest plan step highlighted | 05 | #5 |
| Find with regex and whole word | 06 | #6 |
| Settings dialog | 19 | #7 |
| Copy row as INSERT / CSV, filter by a value | 21 | #8 |
| Remembered parameter values, the parameters hint | 08, 02 | #9 |
| One-step database files, drag and drop | 12, 01 | #10 |
| Row counts, connection dots, saved queries in the explorer | 02 | #11 |
| History details and earlier runs | 10 | #12 |
| Status bar | all | #13 |
| Table editor and ER diagram as tabs | 14, 15, 18 | #14 |
