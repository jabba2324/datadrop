"""Every DataDrop 0.3 screen, in dark and light, as a FigCraft script (JSON lines).

    python3 screens.py out.figcraft > ui.jsonl && figcraft-cli run ui.jsonl
"""
import json
import sys

from ddkit import *  # noqa: F403
from ddkit import steps

MX, MW = SIDE, W - SIDE


def screen(name, x, y, c):
    return add("frame.create", {"x": x, "y": y, "width": W, "height": H, "name": name,
                                "props": {"fill": c["bg"], "clip": True}})


def shell(root, c, selected="orders", mode="explorer", txn="Auto", running=False, status="ok",
          right="Ln 9, Col 23   ·   UTF-8   ·   Spaces: 2", info=None):
    titlebar(root, c, txn=txn, running=running)
    sidebar(root, c, selected=selected, mode=mode)
    kw = {"info": info} if info else {}
    statusbar(root, c, right=right, state=status, **kw)


def label_caps(parent, x, y, s, c):
    text(parent, x, y, s, c["faint"], size=11, weight="Semi Bold", letterSpacing="6%")


def scrim(root, c):
    rect(root, 0, 0, W, H, c["overlay"], name="Scrim")


def sheet(root, c, w, h, title, sub=None, y=None):
    x = (W - w) // 2
    y = (H - h) // 2 if y is None else y
    d = frame(root, x, y, w, h, fill=c["panel"], r=14, stroke=c["border2"], name=f"Sheet/{title}",
              effects=shadow(c["shadow"], 24, 64))
    text(d, 28, 24, title, c["text"], size=18, weight="Semi Bold")
    if sub:
        text(d, 28, 50, sub, c["muted"], size=13, width=w - 100)
    text(d, w - 44, 20, "×", c["faint"], size=20)
    return d


def footer(d, c, w, h, primary, secondary="Cancel", left=None, danger=False):
    f = frame(d, 0, h - 68, w, 68, fill=c["panel"], name="Footer", strokeTopWeight=1,
              stroke=c["border"])
    pw = int(len(primary) * 7.2 + 32)
    button(f, w - 24 - pw, 20, primary, c, "danger" if danger else "primary", w=pw)
    if secondary:
        sw = int(len(secondary) * 7.2 + 28)
        button(f, w - 24 - pw - 10 - sw, 20, secondary, c, "ghost", w=sw)
    if left:
        button(f, 24, 20, left, c, "secondary")
    return f


# --- workspace pieces ------------------------------------------------------------------------

TABS = [("orders_by_month.sql", "Postgres", True), ("customers", "Postgres", False),
        ("events · analytics", "ClickHouse", False)]


def tabs_bar(root, c, tabs=TABS, active=0, y=TOP):
    bar = frame(root, MX, y, MW, 38, fill=c["panel"], name="Tabs", strokeBottomWeight=1,
                stroke=c["border"])
    tx = 0
    for i, (t, drv, dirty) in enumerate(tabs):
        w = int(len(t) * 7.2 + 64)
        on = i == active
        tab = frame(bar, tx, 0, w, 38, fill=c["bg"] if on else "none", name=f"Tab/{t}")
        if on:
            rect(tab, 0, 0, w, 2, c["accent"])
        else:
            rect(tab, w - 1, 10, 1, 18, c["border"])
        driver_badge(tab, 12, 11, drv, size=16)
        text(tab, 36, 10, t, c["text"] if on else c["muted"], size=12.5,
             weight="Medium" if on else "Regular")
        if dirty:
            ellipse(tab, w - 20, 16, 7, c["warn"] if dirty == "warn" else c["muted"])
        else:
            text(tab, w - 22, 9, "×", c["faint"], size=14)
        tx += w
    text(bar, tx + 14, 8, "+", c["faint"], size=17)
    return bar


def editor(root, c, y, h, lines=QUERY, cursor=(9, 34), name="Editor"):
    ed = frame(root, MX, y, MW, h, fill=c["bg"], name=name)
    sql_lines(ed, 8, 16, lines, c, cursor=cursor)
    return ed


def result_tabs(rs, c, active="Results", badge="11 rows", extra=None, views=("Results", "Plan")):
    hx = 12
    for t in views:
        on = t == active
        b = badge if t == "Results" else (extra if t == "Messages" else None)
        w = int(len(t) * 7.4 + (len(b) * 6.4 + 18 if b else 0) + 24)
        tb = frame(rs, hx, 8, w, 28, fill=c["hover"] if on else "none", r=6,
                   name=f"ResultTab/{t}")
        text(tb, 12, 6, t, c["text"] if on else c["muted"], size=12.5,
             weight="Semi Bold" if on else "Medium")
        if b:
            fg, bg = (c["bad"], c["bad"] + "22") if t == "Messages" else (c["muted"], c["border"])
            pill(tb, 12 + len(t) * 7.4 + 6, 5, b, fg, bg, h=18, size=10.5)
        hx += w + 4
    return hx


def results_panel(root, c, y, **kw):
    rs = frame(root, MX, y, MW, H - STATUS - y, fill=c["panel"], name="Results",
               strokeTopWeight=1, stroke=c["border"])
    result_tabs(rs, c, **kw)
    return rs


def row_limit(rs, c, x, value="10,000"):
    text(rs, x, 14, "Row limit", c["faint"], size=12)
    box = frame(rs, x + 62, 9, 72, 26, fill=c["bg"], r=6, stroke=c["border2"], name="Row limit")
    text(box, 9, 5, value, c["text"], size=12, family=MONO)


def where_bar(rs, c, y, cond="", applied=False):
    """The result's filter: a WHERE condition run on the server."""
    bar = frame(rs, 0, y, MW, 38, fill=c["panel"], name="Filter", strokeTopWeight=1,
                stroke=c["border"])
    text(bar, 16, 11, "WHERE", c["faint"], size=12.5, family=MONO, weight="Regular")
    box = frame(bar, 70, 5, 460, 28, fill=c["bg"], r=6,
                stroke=c["accent"] if applied else c["border2"], name="Condition")
    funnel_icon(box, 10, 8.5, c["accent"] if applied else c["faint"])
    text(box, 28, 6, cond or "revenue > 10000 AND top_region <> 'NA'", c["text"] if cond
         else c["faint"], size=12.5, family=MONO)
    button(bar, 540, 5, "Apply", c, "secondary", h=28)
    if applied:
        button(bar, 608, 5, "Clear", c, "ghost", h=28)
        text(bar, 680, 11, "filtered on the server", c["faint"], size=12)
    return bar


def results_header(rs, c, timing="38 ms", where="", applied=False, buttons=None,
                   limit=True):
    """Timing, row limit, Copy all and Export… on the right; the WHERE bar below. Returns
    where the grid starts."""
    buttons = buttons if buttons is not None else (("Export…", "secondary"), ("Copy all", "ghost"))
    ax = MW - 12
    for lbl, kind in buttons:
        w = int(len(lbl) * 7.0 + 24)
        ax -= w
        button(rs, ax, 8, lbl, c, kind, w=w)
        ax -= 8
    if limit:
        ax -= 146
        row_limit(rs, c, ax)
        ax -= 14
    if timing:
        text(rs, ax - len(timing) * 6.2, 14, timing, c["faint"], size=12)
    if where is None:
        return 46
    where_bar(rs, c, 46, where, applied)
    return 84


COLS = [("#", "", 48, "r"), ("month", "timestamptz", 220, "l"), ("orders", "int8", 120, "r"),
        ("revenue", "numeric", 160, "r"), ("avg_order", "numeric", 140, "r"),
        ("top_region", "text", 180, "l"), ("refunded", "numeric", 140, "r")]
DATA = [
    ("2026-10-01 00:00:00+00", "2,941", "412,880.50", "140.39", "EMEA", "3,120.00"),
    ("2026-09-01 00:00:00+00", "3,218", "455,102.10", "141.42", "NA", "4,880.75"),
    ("2026-08-01 00:00:00+00", "2,877", "398,411.00", "138.48", "NA", None),
    ("2026-07-01 00:00:00+00", "2,650", "371,930.25", "140.35", "EMEA", "2,210.00"),
    ("2026-06-01 00:00:00+00", "2,712", "380,004.90", "140.12", "APAC", "1,975.40"),
    ("2026-05-01 00:00:00+00", "2,488", "349,887.60", "140.63", "NA", None),
    ("2026-04-01 00:00:00+00", "2,301", "322,145.00", "140.00", "EMEA", "905.10"),
    ("2026-03-01 00:00:00+00", "2,155", "301,700.45", "140.00", "NA", "1,440.00"),
    ("2026-02-01 00:00:00+00", "1,998", "279,120.30", "139.70", "APAC", None),
    ("2026-01-01 00:00:00+00", "2,076", "290,640.00", "140.00", "NA", "2,600.80"),
]


def data_grid(parent, c, y, h, cols=COLS, data=DATA, selected=None, edited=None, sort="month",
              readonly=False, w=MW, x=0):
    grid = frame(parent, x, y, w, h, fill=c["bg"], name="Grid", strokeTopWeight=1,
                 stroke=c["border"])
    hdr = frame(grid, 0, 0, w, 40, fill=c["panel"], name="Grid header", strokeBottomWeight=1,
                stroke=c["border"])
    cx = 0
    for i, (n_, t_, cw, al) in enumerate(cols):
        if i:
            rect(hdr, cx, 8, 1, 24, c["border"])
        if n_ != "#":
            text(hdr, cx + 12, 5, n_, c["text"], size=12.5, weight="Semi Bold")
            text(hdr, cx + 12, 21, t_, c["faint"], size=10.5)
            if n_ == sort:
                text(hdr, cx + cw - 22, 11, "↓", c["accent"], size=13, weight="Bold")
        cx += cw
    for r_i, row in enumerate(data):
        ry = 40 + r_i * 30
        if ry + 30 > h:
            break
        sel = r_i == selected
        line = frame(grid, 0, ry, w, 30,
                     fill=c["sel_row"] if sel else (c["zebra"] if r_i % 2 else "none"),
                     name=f"Row {r_i + 1}")
        if sel:
            rect(line, 0, 0, 2, 30, c["accent"])
        cx = 0
        for (n_, t_, cw, al), v in zip(cols, (str(r_i + 1),) + tuple(row)):
            if v is None:
                xx = cx + cw - 12 - 4 * 7.2 if al == "r" else cx + 12
                text(line, xx, 7, "NULL", c["faint"], size=12, family=MONO)
            else:
                col = c["faint"] if n_ == "#" else c["text"]
                fam = MONO if al == "r" or t_.startswith(("timestamp", "date", "uuid", "DateTime")) \
                    else SANS
                tw = len(v) * 7.5
                xx = cx + cw - 12 - tw if al == "r" else cx + 12
                text(line, xx, 7, v, col, size=12.5, family=fam)
            cx += cw
        if edited and edited[0] == r_i:
            ex = sum(cw for _, _, cw, _ in cols[:edited[1]])
            e = frame(line, ex, 2, cols[edited[1]][2], 26, fill=c["warn"] + "1F", r=4,
                      stroke=c["warn"] + "80", name="Edited cell")
            text(e, 12, 5, edited[2], c["text"], size=12.5)
    return grid


ED_Y, ED_H = TOP + 38, 350
RS_Y = ED_Y + ED_H


def toast(root, c, x, y, kind, title, body=None, w=360):
    col = {"ok": c["ok"], "bad": c["bad"], "info": c["accent"], "warn": c["warn"]}[kind]
    h = 64 if body else 44
    t = frame(root, x, y, w, h, fill=c["raised"], r=10, stroke=c["border2"], name=f"Toast/{title}",
              effects=shadow(c["shadow"], 8, 24))
    rect(t, 0, 0, 3, h, col)
    ellipse(t, 16, 17, 10, col)
    text(t, 36, 12, title, c["text"], size=13, weight="Semi Bold", width=w - 72)
    if body:
        text(t, 36, 32, body, c["muted"], size=12, width=w - 72)
    text(t, w - 26, 10, "×", c["faint"], size=15)


# --- screens ---------------------------------------------------------------------------------


def s_style(x, y, c):
    root = screen("00 · Style guide", x, y, c)
    text(root, 64, 56, "DataDrop 0.3 — visual language", c["text"], size=28, weight="Semi Bold")
    text(root, 64, 98, "Quiet chrome, content first. One accent, flat surfaces separated by "
         "1 px borders, 6 px radii, an 8 px grid.", c["muted"], size=14)
    label_caps(root, 64, 150, "COLOUR", c)
    for i, (k, lbl) in enumerate([("bg", "Background"), ("panel", "Panel"), ("raised", "Raised"),
                                  ("border", "Border"), ("text", "Text"), ("muted", "Muted"),
                                  ("accent", "Accent"), ("ok", "Success"), ("warn", "Warning"),
                                  ("bad", "Danger")]):
        sx = 64 + i * 128
        top = frame(root, sx, 176, 112, 72, fill=DARK[k], r=8, stroke=c["border2"],
                    name=f"Swatch/{k}")
        rect(top, 0, 36, 112, 36, LIGHT[k])
        text(root, sx, 256, lbl, c["text"], size=12.5, weight="Medium")
        text(root, sx, 274, f"{DARK[k][:7]} / {LIGHT[k][:7]}", c["faint"], size=10.5, family=MONO)
    label_caps(root, 64, 318, "SYNTAX", c)
    syn = frame(root, 64, 342, 600, 150, fill=c["panel"], r=10, stroke=c["border"], name="Syntax")
    sql_lines(syn, 8, 16, QUERY[1:7], c, start=2)
    label_caps(root, 720, 318, "TYPE", c)
    for i, (lbl, sz, wt, fam) in enumerate([
            ("Heading · Inter 18 Semi Bold", 18, "Semi Bold", SANS),
            ("Body · Inter 13 Regular", 13, "Regular", SANS),
            ("Label · Inter 12.5 Medium", 12.5, "Medium", SANS),
            ("CAPTION · INTER 11 SEMI BOLD +6%", 11, "Semi Bold", SANS),
            ("Code · Mono 13  SELECT * FROM orders;", 13, "Regular", MONO)]):
        text(root, 720, 346 + i * 30, lbl, c["text"] if i < 3 else c["muted"], size=sz,
             weight=wt, family=fam)
    label_caps(root, 64, 530, "CONTROLS", c)
    bx = 64
    bx += button(root, bx, 556, "Run", c, "primary", icon="▶", hint="⌘↵") + 12
    bx += button(root, bx, 556, "Explain", c, "secondary") + 12
    bx += button(root, bx, 556, "Discard", c, "ghost") + 12
    bx += button(root, bx, 556, "Drop table", c, "danger") + 12
    button(root, bx, 556, "Stop", c, "disabled", icon="■")
    input_box(root, 64, 628, 260, "prod-db.internal", c, label="Host")
    input_box(root, 340, 628, 260, "Filter objects…", c, placeholder=True, label="Placeholder")
    input_box(root, 616, 628, 260, "shop", c, label="Focused", focus=True)
    px = 64
    for lbl, fg in [("PK", c["amber"]), ("FK", c["accent"]), ("UNIQUE", c["purple"]),
                    ("NOT NULL", c["muted"]), ("11 rows", c["muted"]), (":since", c["param"])]:
        px += pill(root, px, 690, lbl, fg, fg + "22") + 8
    for i, d in enumerate(DRIVERS):
        driver_badge(root, 64 + i * 120, 732, d, size=24)
        text(root, 96 + i * 120, 736, d, c["muted"], size=12)
    for i, k in enumerate(["table", "view", "fn", "seq", "db", "idx"]):
        kind_icon(root, 64 + i * 90, 780, k, c)
        text(root, 84 + i * 90, 779, {"table": "Table", "view": "View", "fn": "Function",
                                      "seq": "Sequence", "db": "Database", "idx": "Column"}[k],
             c["muted"], size=12)
    label_caps(root, 920, 530, "PRINCIPLES", c)
    for i, p_ in enumerate([
            "Menus fold into a ⌘K palette; the toolbar keeps only Run, Explain, Stop.",
            "The connection is always visible, with its driver and live status.",
            "Numbers right-aligned in mono; NULL is faint, never blank.",
            "Unsaved state is amber everywhere: edited cells, dirty tabs, the edits bar.",
            "Dialogs are sheets with a clear primary action, bottom right.",
            "Both themes come from one token set (above), not egui defaults."]):
        ellipse(root, 920, 562 + i * 40, 6, c["accent"])
        text(root, 936, 555 + i * 40, p_, c["text"], size=13, width=460)


def s_welcome(x, y, c):
    root = screen("01 · Welcome (no connections)", x, y, c)
    titlebar(root, c, conn=("No connection", None, ""))
    sb = frame(root, 0, TOP, SIDE, H - TOP - STATUS, fill=c["panel"], name="Explorer",
               strokeRightWeight=1, stroke=c["border"])
    side_switch(sb, c, "explorer")
    text(sb, 0, 300, "No connections yet", c["muted"], size=13, weight="Medium", width=SIDE,
         align="center")
    text(sb, 32, 324, "Add one to browse its tables here.", c["faint"], size=12, width=SIDE - 64,
         align="center")
    statusbar(root, c, state="none", info="", right="DataDrop 0.3.0")
    cx = MX + (MW - 640) // 2
    drop = frame(root, cx + 288, 168, 64, 64, fill=c["accent"], r=18, name="Logo")
    ellipse(drop, 20, 22, 24, "#FFFFFF")
    text(root, cx, 256, "Welcome to DataDrop", c["text"], size=26, weight="Semi Bold", width=640,
         align="center")
    text(root, cx, 296, "A fast SQL client for Postgres, MySQL, SQLite, SQL Server, ClickHouse "
         "and DuckDB.", c["muted"], size=14, width=640, align="center")
    cards = [("Add a connection", "Server databases, over SSH if you need it.", "⌘N", "Postgres"),
             ("Open a database file", "SQLite or DuckDB, from your disk.", "⌘O", "DuckDB"),
             ("Import from a URL", "postgres://, mysql://, sqlserver://…", "⌘⇧V", "MySQL")]
    for i, (t, b, k, d) in enumerate(cards):
        card = frame(root, cx + i * 216, 360, 208, 150, fill=c["panel"], r=12,
                     stroke=c["accent"] if i == 0 else c["border"], name=f"Start/{t}")
        driver_badge(card, 18, 18, d, size=28)
        text(card, 18, 62, t, c["text"], size=14, weight="Semi Bold")
        text(card, 18, 86, b, c["muted"], size=12.5, width=172)
        pill(card, 18, 118, k, c["muted"], c["hover"], h=20)
    label_caps(root, cx, 560, "RECENT FILES", c)
    for i, (f_, p_) in enumerate([("orders_by_month.sql", "~/work/reports"),
                                  ("warehouse.duckdb", "~/data"),
                                  ("churn_cohorts.sql", "~/work/reports")]):
        r = frame(root, cx, 584 + i * 36, 640, 32, fill="none", r=6, name=f"Recent/{f_}")
        text(r, 12, 7, "≡", c["faint"], size=13)
        text(r, 34, 7, f_, c["text"], size=13)
        text(r, 640 - 12 - len(p_) * 7, 8, p_, c["faint"], size=12)


def s_results(x, y, c):
    root = screen("02 · Workspace — results", x, y, c)
    shell(root, c)
    tabs_bar(root, c)
    ed = editor(root, c, ED_Y, ED_H)
    ac = frame(ed, 44 + 8 + 34 * CW - 4, 16 + 9 * 21 + 2, 300, 120, fill=c["raised"], r=8,
               stroke=c["border2"], name="Autocomplete", effects=shadow(c["shadow"]))
    for i, (n_, t_, src) in enumerate([("created_at", "timestamptz", "orders"),
                                       ("customer_id", "int8", "orders"),
                                       ("currency", "char(3)", "orders"),
                                       ("coupon_code", "text", "orders")]):
        r = frame(ac, 4, 4 + i * 28, 292, 26, fill=c["accent_soft"] if i == 0 else "none", r=5,
                  name=f"Completion/{n_}")
        kind_icon(r, 8, 6, "idx", c)
        text(r, 30, 5, n_, c["text"], size=12.5, family=MONO)
        text(r, 292 - 12 - len(src) * 6.2, 6, src, c["faint"], size=11)
    hint = frame(ed, MW - 236, 12, 220, 26, fill=c["raised"], r=6, stroke=c["border"],
                 name="Params hint")
    text(hint, 10, 5, "1 parameter", c["muted"], size=12)
    pill(hint, 96, 3, ":since = 2026-01-01", c["param"], c["param"] + "22", h=20, size=11)
    rs = results_panel(root, c, RS_Y)
    gy = results_header(rs, c, where="revenue > 10000", applied=True)
    data_grid(rs, c, gy, 360, selected=1, edited=(3, 5, "EMEA · West"))
    bar = frame(rs, MW - 432, gy + 300, 420, 40, fill=c["raised"], r=10, stroke=c["border2"],
                name="Pending edits", effects=shadow(c["shadow"], 6, 20))
    ellipse(bar, 14, 16, 8, c["warn"])
    text(bar, 30, 11, "1 unsaved change", c["text"], size=12.5, weight="Medium")
    button(bar, 220, 6, "Discard", c, "ghost", h=28)
    button(bar, 296, 6, "Review & save", c, "primary", h=28)


def s_running(x, y, c):
    root = screen("03 · Workspace — running", x, y, c)
    shell(root, c, running=True, status="busy", info="prod-db  ·  statement 1 of 1  ·  4.2 s")
    tabs_bar(root, c, tabs=[("orders_by_month.sql", "Postgres", False)] + TABS[1:])
    editor(root, c, ED_Y, ED_H, cursor=None)
    rs = results_panel(root, c, RS_Y, badge=None)
    text(rs, 200, 14, "Running… 4.2 s", c["muted"], size=12.5)
    button(rs, 310, 8, "Stop", c, "danger", icon="■")
    # Progress strip and skeleton rows.
    rect(rs, 0, 0, MW, 2, c["border"])
    rect(rs, 0, 0, MW * 0.42, 2, c["accent"])
    g = frame(rs, 0, 46, MW, 400, fill=c["bg"], name="Skeleton", strokeTopWeight=1,
              stroke=c["border"])
    for i in range(9):
        for j, (cx, cw) in enumerate([(60, 180), (300, 80), (440, 120), (600, 90), (740, 140)]):
            rect(g, cx, 20 + i * 32, cw * (0.6 + 0.4 * ((i + j) % 3) / 2), 10,
                 c["hover"], r=5, opacity=1 - i * 0.09)
    mid = frame(rs, (MW - 360) // 2, 180, 360, 92, fill=c["raised"], r=12, stroke=c["border2"],
                name="Running card", effects=shadow(c["shadow"], 8, 30))
    ring = frame(mid, 20, 26, 40, 40, fill="none", r=20, stroke=c["border2"], strokeWeight=3,
                 name="Spinner track")
    vector(mid, 20, 26, 40, 40, "M20 0 A20 20 0 0 1 40 20", c["accent"], weight=3, name="Spinner")
    text(mid, 76, 24, "Running on prod-db…", c["text"], size=14, weight="Semi Bold")
    text(mid, 76, 46, "4.2 s  ·  press ⌘. or Stop to cancel on the server", c["muted"], size=12)


def s_error(x, y, c):
    root = screen("04 · Query error", x, y, c)
    shell(root, c, right="Ln 6, Col 16   ·   UTF-8   ·   Spaces: 2")
    tabs_bar(root, c)
    bad = [list(l) for l in QUERY]
    bad[5] = [("kw", "FROM"), ("ident", " order o")]
    ed = editor(root, c, ED_Y, ED_H, lines=bad, cursor=(6, 10))
    # Squiggle under the bad identifier and an inline marker.
    vector(ed, 44 + 8 + 5 * CW, 16 + 5 * 21 + 17, 5 * CW, 3,
           "M0 1.5 Q2 0 4 1.5 T8 1.5 T12 1.5 T16 1.5 T20 1.5 T24 1.5 T28 1.5 T32 1.5 T36 1.5 T39 1.5",
           c["bad"], weight=1.2, name="Squiggle")
    ellipse(ed, 2, 16 + 5 * 21 + 4, 8, c["bad"])
    tip = frame(ed, 44 + 8 + 18 * CW, 16 + 5 * 21 - 3, 380, 24, fill=c["raised"], r=6,
                stroke=c["bad"] + "88", name="Error tip", effects=shadow(c["shadow"], 4, 14))
    text(tip, 12, 4, 'relation "order" does not exist — did you mean "orders"?', c["text"],
         size=12)
    rs = results_panel(root, c, RS_Y, active="Messages", badge=None, extra="1 error",
                       views=("Results", "Messages", "Plan"))
    results_header(rs, c, timing="failed after 2 ms", where=None, buttons=(), limit=False)
    box = frame(rs, 16, 56, MW - 32, 124, fill=c["bad"] + "12", r=10, stroke=c["bad"] + "55",
                name="Error")
    ellipse(box, 18, 20, 10, c["bad"])
    text(box, 38, 14, 'ERROR 42P01   relation "order" does not exist', c["text"], size=13.5,
         weight="Semi Bold", family=MONO)
    text(box, 38, 40, "Statement 1 of 1, line 6, column 6", c["muted"], size=12.5)
    code = frame(box, 38, 66, 520, 40, fill=c["bg"], r=6, stroke=c["border"], name="Excerpt")
    text(code, 12, 6, "6  FROM order o", c["text"], size=12.5, family=MONO)
    text(code, 12 + 8 * 7.5, 22, "^^^^^", c["bad"], size=12.5, family=MONO)
    bx = MW - 32 - 16
    for lbl, kind in [("Jump to line", "secondary"), ("Copy", "ghost")]:
        w_ = int(len(lbl) * 7.0 + 24)
        bx -= w_
        button(box, bx, 14, lbl, c, kind, w=w_)
        bx -= 8
    label_caps(rs, 16, 200, "EARLIER IN THIS TAB", c)
    for i, (dot, msg, t_) in enumerate([(c["ok"], "SET search_path TO shop, public", "1 ms"),
                                        (c["muted"], "NOTICE  extension \"citext\" already exists, "
                                         "skipping", "")]):
        text(rs, 16, 224 + i * 26, "●", dot, size=9)
        text(rs, 32, 222 + i * 26, msg, c["muted"], size=12.5, family=MONO)
        text(rs, MW - 60, 222 + i * 26, t_, c["faint"], size=12)
    toast(root, c, W - 380, TOP + 50, "bad", "Query failed", 'relation "order" does not exist')


def s_plan(x, y, c):
    root = screen("05 · Query plan", x, y, c)
    shell(root, c)
    tabs_bar(root, c)
    editor(root, c, ED_Y, 254, cursor=None)
    py = ED_Y + 254
    rs = results_panel(root, c, py, active="Plan", badge=None)
    pill(rs, 200, 12, "Analyzed", c["ok"], c["ok"] + "22", h=20)
    text(rs, 280, 13, "planning 0.4 ms   ·   execution 38.2 ms", c["muted"], size=12.5)
    results_header(rs, c, timing="", where=None, buttons=(("Copy", "ghost"),), limit=False)
    t = frame(rs, 0, 46, MW, 560, fill=c["bg"], name="Plan tree", strokeTopWeight=1,
              stroke=c["border"])
    hdr = frame(t, 0, 0, MW, 34, fill=c["panel"], name="Plan header", strokeBottomWeight=1,
                stroke=c["border"])
    cols = [("Step", 16), ("Rows", 560), ("Est. rows", 650), ("Cost", 750), ("Time (ms)", 850)]
    for n_, cx in cols:
        text(hdr, cx, 9, n_, c["muted"], size=12, weight="Semi Bold")
    nodes = [
        (0, "Sort", "month DESC", "11", "11", "4,211", 38.2, None),
        (1, "HashAggregate", "group by date_trunc('month', created_at)", "11", "24", "4,205",
         37.9, None),
        (2, "Hash Join", "c.id = o.customer_id", "28,104", "27,900", "3,980", 33.0, None),
        (3, "Seq Scan on orders o", "status = 'paid' AND created_at >= $1  ·  removed 284k",
         "28,104", "31,000", "3,120", 26.4, "slow"),
        (3, "Hash", "", "48,210", "48,210", "620", 5.1, None),
        (4, "Index Only Scan on customers c", "customers_pkey", "48,210", "48,210", "510", 4.2,
         None),
    ]
    for i, (d, op, det, rows, est, cost, ms, flag) in enumerate(nodes):
        ry = 34 + i * 56
        r = frame(t, 0, ry, MW, 56, fill=c["warn"] + "10" if flag else "none",
                  name=f"Plan/{op}", strokeBottomWeight=1, stroke=c["border"])
        if flag:
            rect(r, 0, 0, 2, 56, c["warn"])
        ix = 32 + d * 22
        if d:
            vector(r, ix - 30, 0, 10, 22, "M0 0 L0 20 L10 20", c["border2"], weight=1.2)
        if i < len(nodes) - 1 and nodes[i + 1][0] > d:
            chevron(r, ix - 16, 12, c["muted"], open_=True)
        text(r, ix, 10, op, c["text"], size=13, weight="Semi Bold")
        if det:
            text(r, ix, 30, det, c["muted"], size=12, family=MONO)
        text(r, 560, 18, rows, c["text"], size=12.5, family=MONO)
        bad_est = abs(int(rows.replace(",", "")) - int(est.replace(",", ""))) > 5
        text(r, 650, 18, est, c["warn"] if bad_est and flag else c["muted"], size=12.5,
             family=MONO)
        text(r, 750, 18, cost, c["muted"], size=12.5, family=MONO)
        bw = 220
        rect(r, 850, 23, bw, 6, c["hover"], r=3)
        rect(r, 850, 23, bw * ms / 38.2, 6, c["warn"] if flag else c["accent"], r=3)
        text(r, 850 + bw + 12, 17, f"{ms:.1f}", c["text"], size=12.5, family=MONO)


def s_txn_find(x, y, c):
    root = screen("06 · Manual commit + find", x, y, c)
    shell(root, c, txn="Manual", status="txn", info="prod-db  ·  auto-commit off")
    tabs_bar(root, c, tabs=[("orders_by_month.sql", "Postgres", "warn")] + TABS[1:])
    # Transaction banner.
    tb = frame(root, MX, ED_Y, MW, 44, fill=c["warn"] + "16", name="Transaction bar",
               strokeBottomWeight=1, stroke=c["warn"] + "55")
    ellipse(tb, 16, 18, 8, c["warn"])
    text(tb, 32, 13, "Transaction open", c["text"], size=13, weight="Semi Bold")
    text(tb, 160, 14, "Changes made in this tab aren't saved (or seen by others) until you commit",
         c["muted"], size=12.5)
    button(tb, MW - 24 - 90 - 10 - 96, 8, "Roll back", c, "secondary", w=96)
    button(tb, MW - 24 - 90, 8, "Commit", c, "primary", w=90)
    # Find & replace.
    fb = frame(root, MX, ED_Y + 44, MW, 46, fill=c["panel"], name="Find bar",
               strokeBottomWeight=1, stroke=c["border"])
    input_box(fb, 14, 7, 300, "o.total", c, h=32, focus=True, mono=True)
    pill(fb, 280, 13, "Aa", c["accent"], c["accent_soft"], h=20, w=22, size=10.5)
    input_box(fb, 326, 7, 260, "o.amount", c, h=32, mono=True)
    text(fb, 600, 14, "2 of 3", c["muted"], size=12.5)
    for i, a in enumerate(["↑", "↓"]):
        f = frame(fb, 660 + i * 30, 9, 28, 28, fill="none", r=6, stroke=c["border2"],
                  name=f"Find {a}")
        text(f, 9, 4, a, c["text"], size=13)
    button(fb, 732, 9, "Replace", c, "secondary")
    button(fb, 814, 9, "Replace all", c, "ghost")
    text(fb, MW - 30, 10, "×", c["faint"], size=16)
    lines = [list(l) for l in QUERY]
    ed = editor(root, c, ED_Y + 90, ED_H - 90 + 40, lines=lines, cursor=None)
    for (ln, col, cur) in [(4, 11, False), (5, 17, True)]:
        rect(ed, 52 + col * CW, 16 + (ln - 1) * 21 - 1, 7 * CW, 19,
             (c["warn"] + "66") if cur else (c["warn"] + "2A"), r=3)
    rs = results_panel(root, c, RS_Y + 40, badge="1 row")
    gy = results_header(rs, c, timing="4 ms")
    data_grid(rs, c, gy, 300, data=DATA[3:5], selected=0, edited=(0, 5, "EMEA · West"))


def s_review(x, y, c):
    root = screen("07 · Review edits", x, y, c)
    shell(root, c)
    tabs_bar(root, c)
    editor(root, c, ED_Y, ED_H, cursor=None)
    rs = results_panel(root, c, RS_Y)
    gy = results_header(rs, c, where=None)
    data_grid(rs, c, gy, 400, selected=1, edited=(3, 5, "EMEA · West"))
    scrim(root, c)
    DW, DH = 680, 470
    d = sheet(root, c, DW, DH, "Review 2 changes",
              "They run in one transaction on prod-db: all of them, or none.")
    for i, (kind, what, sql) in enumerate([
            ("UPDATE", "orders · row 4 · top_region",
             "UPDATE orders SET top_region = 'EMEA · West'\n  WHERE id = 40211;"),
            ("DELETE", "orders · row 9",
             "DELETE FROM orders WHERE id = 39870;")]):
        cy = 100 + i * 128
        card = frame(d, 28, cy, DW - 56, 116, fill=c["bg"], r=10, stroke=c["border"],
                     name=f"Change/{i}")
        col = c["warn"] if kind == "UPDATE" else c["bad"]
        pill(card, 14, 14, kind, col, col + "22", h=20)
        text(card, 90, 15, what, c["muted"], size=12.5)
        text(card, DW - 56 - 70, 15, "Revert", c["accent"], size=12.5, weight="Medium")
        code = frame(card, 14, 46, DW - 84, 56, fill=c["panel"], r=6, stroke=c["border"],
                     name="SQL")
        text(code, 12, 9, sql, c["text"], size=12.5, family=MONO, lineHeight="160%")
    text(d, 28, 368, "Rows are matched on the primary key (id).", c["faint"], size=12)
    footer(d, c, DW, DH, "Save 2 changes", left="Copy SQL")


def s_params(x, y, c):
    root = screen("08 · Query parameters", x, y, c)
    shell(root, c)
    tabs_bar(root, c)
    editor(root, c, ED_Y, ED_H)
    results_panel(root, c, RS_Y, badge=None)
    scrim(root, c)
    DW, DH = 600, 400
    d = sheet(root, c, DW, DH, "Values for the query",
              "Numbers go in as they are, null as NULL, anything else as text.")
    hdr_y = 104
    for n_, cx in [("Parameter", 28), ("Value", 170), ("As", 452)]:
        text(d, cx, hdr_y, n_, c["muted"], size=12, weight="Semi Bold")
    for i, (p_, v, mode, foc) in enumerate([(":since", "2026-01-01", "Auto", True),
                                            (":region", "EMEA", "Text", False),
                                            (":limit", "", "NULL", False)]):
        ry = 128 + i * 48
        pill(d, 28, ry + 7, p_, c["param"], c["param"] + "22", h=22, size=12)
        input_box(d, 170, ry, 268, v or "NULL", c, placeholder=not v, focus=foc, mono=True, h=36)
        seg = frame(d, 452, ry + 2, 120, 32, fill=c["bg"], r=7, stroke=c["border"],
                    name="Mode")
        on = ["Auto", "Text", "NULL"].index(mode)
        frame(seg, 2 + on * 39, 2, 38, 28, fill=c["raised"], r=5, stroke=c["border2"])
        for j, m in enumerate(["Auto", "Text", "NULL"]):
            text(seg, 2 + j * 39, 8, m, c["text"] if j == on else c["muted"], size=11.5,
                 weight="Medium", width=38, align="center")
    f = footer(d, c, DW, DH, "Run", secondary="Cancel")
    button(f, DW - 24 - 60 - 10 - 80 - 10 - 80, 20, "Explain", c, "secondary", w=80)


def s_palette(x, y, c):
    root = screen("09 · Command palette", x, y, c)
    shell(root, c)
    tabs_bar(root, c)
    editor(root, c, ED_Y, ED_H, cursor=None)
    rs = results_panel(root, c, RS_Y)
    gy = results_header(rs, c, where=None)
    data_grid(rs, c, gy, 400, selected=None)
    rect(root, 0, 0, W, H, c["overlay"][:7] + "80", name="Scrim")
    PW = 640
    p = frame(root, (W - PW) // 2, 110, PW, 470, fill=c["panel"], r=14, stroke=c["border2"],
              name="Palette", effects=shadow(c["shadow"], 24, 64))
    q = frame(p, 0, 0, PW, 56, fill="none", name="Query", strokeBottomWeight=1,
              stroke=c["border"])
    search_icon(q, 20, 21, c["muted"])
    text(q, 44, 17, "ord", c["text"], size=16)
    rect(q, 44 + 3 * 9.2, 17, 2, 20, c["accent"])
    pill(q, PW - 54, 18, "esc", c["muted"], c["hover"], h=20)
    groups = [
        ("TABLES", [("table", "orders", "prod-db · shop · 312k rows", "↵ open"),
                    ("table", "order_items", "prod-db · shop · 1.2M rows", ""),
                    ("view", "orders_paid", "prod-db · shop · view", "")]),
        ("COMMANDS", [("cmd", "Format SQL", "Editor", "⇧⌘F"),
                      ("cmd", "Explain analyze", "Query", "⌥⌘E"),
                      ("cmd", "Compare schemas…", "Schema", ""),
                      ("cmd", "Import into orders…", "Schema", "")]),
        ("SAVED QUERIES", [("file", "orders_by_month.sql", "~/work/reports", "")]),
    ]
    gy = 64
    first = True
    for g, items in groups:
        label_caps(p, 20, gy + 4, g, c)
        gy += 24
        for kind, n_, sub, k in items:
            r = frame(p, 8, gy, PW - 16, 34, fill=c["accent_soft"] if first else "none", r=7,
                      name=f"Item/{n_}")
            if kind in ("table", "view"):
                kind_icon(r, 12, 10, kind, c)
            else:
                text(r, 12, 7, "›" if kind == "cmd" else "≡", c["muted"], size=14, weight="Bold")
            # Highlight the match.
            text(r, 36, 8, n_, c["text"], size=13, weight="Medium")
            if n_.lower().startswith("ord"):
                rect(r, 36, 27, 3 * 7.6, 1.5, c["accent"])
            text(r, 36 + len(n_) * 7.8 + 14, 9, sub, c["faint"], size=12)
            if k:
                kw_ = int(len(k) * 9 + 16)
                pill(r, PW - 16 - 12 - kw_, 7, k, c["muted"], c["hover"], h=20, w=kw_)
            gy += 36
            first = False
        gy += 6
    ft = frame(p, 0, 470 - 36, PW, 36, fill=c["bg"], name="Hints", strokeTopWeight=1,
               stroke=c["border"])
    text(ft, 20, 10, "↑↓ move    ↵ open    ⌘↵ open in a new tab    ⇥ commands only", c["faint"],
         size=11.5)


def s_history(x, y, c):
    root = screen("10 · History", x, y, c)
    shell(root, c, mode="history")
    tabs_bar(root, c, tabs=[("History · prod-db", "Postgres", False)] + TABS[:1])
    ed = frame(root, MX, ED_Y, MW, H - ED_Y - STATUS, fill=c["bg"], name="History detail")
    text(ed, 32, 28, "Ran just now on prod-db  ·  38 ms  ·  11 rows", c["muted"], size=12.5)
    text(ed, 32, 52, "Revenue and order counts by month", c["text"], size=18, weight="Semi Bold")
    bx = 32
    for lbl, kind in [("Open in a new tab", "primary"), ("Insert at the cursor", "secondary"),
                      ("Copy", "ghost")]:
        bx += button(ed, bx, 92, lbl, c, kind) + 8
    code = frame(ed, 32, 140, MW - 64, 290, fill=c["panel"], r=10, stroke=c["border"],
                 name="SQL")
    sql_lines(code, 0, 16, QUERY, c)
    label_caps(ed, 32, 456, "RUNS OF THIS QUERY", c)
    for i, (when, took, rows, st) in enumerate([("Today 16:02", "38 ms", "11 rows", "ok"),
                                                ("Today 11:47", "41 ms", "11 rows", "ok"),
                                                ("Mon 09:13", "2 ms", "error", "bad"),
                                                ("Mon 09:12", "44 ms", "10 rows", "ok")]):
        r = frame(ed, 32, 480 + i * 40, MW - 64, 36, fill=c["panel"] if i == 0 else "none", r=6,
                  name=f"Run/{when}", strokeBottomWeight=1, stroke=c["border"])
        ellipse(r, 12, 14, 8, c["ok"] if st == "ok" else c["bad"])
        text(r, 32, 9, when, c["text"], size=12.5)
        text(r, 200, 9, took, c["muted"], size=12.5, family=MONO)
        text(r, 300, 9, rows if st == "ok" else 'relation "order" does not exist',
             c["muted"] if st == "ok" else c["bad"], size=12.5)


def s_connection(x, y, c, file_db=False):
    name = "12 · Connection — database file" if file_db else "11 · New connection"
    root = screen(name, x, y, c)
    shell(root, c)
    frame(root, MX, TOP, MW, H - TOP - STATUS, fill=c["bg"], name="Main (behind)")
    scrim(root, c)
    DW, DH = 760, 600
    d = sheet(root, c, DW, DH, "Edit connection" if file_db else "New connection",
              "Pick a database, then how to reach it.")
    rect(d, 0, 84, DW, 1, c["border"])
    lst = frame(d, 0, 85, 220, DH - 85 - 69, fill=c["bg"], name="Drivers",
                strokeRightWeight=1, stroke=c["border"])
    sel_drv = "DuckDB" if file_db else "Postgres"
    for i, drv in enumerate(["Postgres", "MySQL", "SQLite", "SQL Server", "ClickHouse",
                             "DuckDB", "CockroachDB"]):
        sel = drv == sel_drv
        r = frame(lst, 10, 12 + i * 40, 200, 36, fill=c["accent_soft"] if sel else "none", r=8,
                  stroke=c["accent"] + "66" if sel else None, name=f"Driver row/{drv}")
        driver_badge(r, 10, 8, drv, size=20)
        text(r, 40, 9, drv, c["text"] if sel else c["muted"], size=13,
             weight="Semi Bold" if sel else "Medium")
        if drv in ("SQLite", "DuckDB"):
            text(r, 152, 10, "file", c["faint"], size=11)
    fx, fw = 248, DW - 248 - 28
    if file_db:
        input_box(d, fx, 128, fw, "warehouse", c, label="Name")
        input_box(d, fx, 196, fw - 104, "~/data/warehouse.duckdb", c, label="File", mono=True,
                  focus=True)
        button(d, fx + fw - 92, 198, "Browse…", c, "secondary", w=92)
        cb = frame(d, fx, 254, 16, 16, fill=c["accent"], r=4, name="Create")
        text(cb, 3, -1, "✓", "#FFFFFF", size=12, weight="Bold")
        text(d, fx + 26, 253, "Create the file if it doesn't exist", c["text"], size=12.5)
        drop = frame(d, fx, 296, fw, 150, fill=c["bg"], r=10, stroke=c["border2"],
                     dashes=[6, 4], name="Drop zone")
        text(drop, 0, 52, "Drop a .duckdb, .db or .sqlite file here", c["muted"], size=13,
             weight="Medium", width=fw, align="center")
        text(drop, 0, 76, "or pass it on the command line:  datadrop-gui shop.duckdb",
             c["faint"], size=12, width=fw, align="center")
        text(d, fx, 474, "✓", c["ok"], size=14, weight="Bold")
        text(d, fx + 20, 474, "Opened DuckDB v1.4.1  ·  14 tables", c["ok"], size=12.5,
             weight="Medium")
    else:
        input_box(d, fx, 128, fw, "prod-db", c, label="Name", focus=True)
        input_box(d, fx, 196, fw - 112, "db.internal.example", c, label="Host")
        input_box(d, fx + fw - 100, 196, 100, "5432", c, label="Port", mono=True)
        input_box(d, fx, 264, (fw - 12) // 2, "shop", c, label="Database")
        input_box(d, fx + (fw + 12) // 2, 264, (fw - 12) // 2, "app_readonly", c, label="User")
        input_box(d, fx, 332, fw - 150, "••••••••••••", c, label="Password")
        sel = frame(d, fx + fw - 138, 332, 138, 32, fill=c["raised"], r=6, stroke=c["border2"],
                    name="Password storage")
        lock_icon(sel, 10, 8, c["ok"])
        text(sel, 30, 7, "Keychain", c["text"], size=13)
        chevron(sel, 118, 8, c["muted"])
        ssh = frame(d, fx, 388, fw, 108, fill=c["bg"], r=10, stroke=c["border"],
                    name="SSH tunnel")
        tog = frame(ssh, 14, 14, 34, 20, fill=c["accent"], r=10, name="Toggle")
        ellipse(tog, 16, 2, 16, "#FFFFFF")
        text(ssh, 58, 14, "SSH tunnel", c["text"], size=13, weight="Semi Bold")
        text(ssh, 140, 15, "through a bastion host", c["muted"], size=12)
        input_box(ssh, 14, 56, fw - 28 - 230, "deploy@bastion.example", c, h=32)
        input_box(ssh, fw - 14 - 218, 56, 218, "~/.ssh/id_ed25519", c, h=32, mono=True)
        text(d, fx, 506, "✓", c["ok"], size=14, weight="Bold")
        text(d, fx + 20, 506, "Connected to PostgreSQL 17.2 in 42 ms", c["ok"], size=12.5,
             weight="Medium")
    f = footer(d, c, DW, DH, "Save & connect", left="Test connection")
    if file_db:
        button(f, 24 + 140, 20, "Delete", c, "ghost")


def s_ssh_trust(x, y, c):
    root = screen("13 · Trust SSH host", x, y, c)
    shell(root, c, status="none", info="Connecting to prod-db through bastion.example…")
    frame(root, MX, TOP, MW, H - TOP - STATUS, fill=c["bg"], name="Main (behind)")
    scrim(root, c)
    DW, DH = 560, 400
    d = sheet(root, c, DW, DH, "Trust this SSH host?",
              "DataDrop hasn't connected to bastion.example before. Check the fingerprint with "
              "whoever runs it.")
    card = frame(d, 28, 116, DW - 56, 150, fill=c["bg"], r=10, stroke=c["border"],
                 name="Fingerprint")
    for i, (k, v) in enumerate([("Host", "bastion.example:22"), ("Key", "ED25519"),
                                ("Fingerprint", "SHA256:p3Vd1yQm0x9Kc4u2Rr7…Lw8NfA")]):
        text(card, 16, 18 + i * 42, k, c["muted"], size=12, weight="Medium")
        text(card, 120, 16 + i * 42, v, c["text"], size=13, family=MONO)
    text(d, 28, 284, "It's saved to known_hosts, and you'll be warned if it ever changes.",
         c["muted"], size=12.5, width=DW - 56)
    footer(d, c, DW, DH, "Trust and connect")


def te_header(root, c, tab, name="customers", sub="public  ·  table", new=False,
              counts=(("Columns", "7"), ("Indexes", "3"), ("Foreign keys", "1"), ("Checks", "1"),
                      ("DDL", None))):
    hd = frame(root, MX, TOP, MW, 92, fill=c["panel"], name="Header", strokeBottomWeight=1,
               stroke=c["border"])
    kind_icon(hd, 24, 22, "table", c)
    if new:
        input_box(hd, 46, 12, 260, name, c, h=32, focus=True, mono=True)
        text(hd, 320, 20, sub, c["muted"], size=12.5)
    else:
        text(hd, 46, 18, name, c["text"], size=18, weight="Semi Bold")
        text(hd, 56 + len(name) * 10, 22, sub, c["muted"], size=12.5)
        button(hd, MW - 24 - 112, 14, "Show rows", c, "secondary", w=112)
        button(hd, MW - 24 - 112 - 8 - 96, 14, "Diagram", c, "ghost", w=96)
    tx = 20
    for t, n_ in counts:
        on = t == tab
        w = int(len(t) * 7.4 + 28)
        text(hd, tx + 14, 62, t, c["text"] if on else c["muted"], size=13,
             weight="Semi Bold" if on else "Medium")
        if on:
            rect(hd, tx, 90, w + (22 if n_ else 0), 2, c["accent"])
        if n_:
            pill(hd, tx + w - 4, 62, n_, c["muted"], c["border"], h=18, size=10.5)
            tx += 22
        tx += w + 6
    return hd


def pending_panel(root, c, LW, changes, sql, primary, note="Runs in one transaction.",
                  errors=()):
    pc = frame(root, MX + LW, TOP + 92, MW - LW, H - TOP - 92 - STATUS, fill=c["panel"],
               name="Pending changes", strokeLeftWeight=1, stroke=c["border"])
    PW = MW - LW
    label_caps(pc, 20, 18, "PENDING CHANGES", c)
    pill(pc, 146, 15, str(len(changes)), c["warn"], c["warn"] + "22", h=18, size=10.5)
    y = 49
    for dot, s_ in changes:
        ellipse(pc, 22, y + 5, 7, c[dot])
        text(pc, 38, y, s_, c["text"], size=12.5, width=PW - 60)
        y += 26
    for e in errors:
        b = frame(pc, 16, y + 4, PW - 32, 36, fill=c["bad"] + "14", r=8, stroke=c["bad"] + "55",
                  name="Error")
        text(b, 12, 9, e, c["bad"], size=12, width=PW - 60)
        y += 44
    code = frame(pc, 16, y + 12, PW - 32, 250, fill=c["bg"], r=10, stroke=c["border"],
                 name="SQL")
    sql_lines(code, 0, 14, sql, c)
    y += 274
    text(pc, 20, y, note, c["muted"], size=12, width=PW - 40)
    pw = int(len(primary) * 7.2 + 32)
    button(pc, 16, y + 34, "Copy SQL", c, "ghost")
    button(pc, PW - 16 - pw - 8 - 84, y + 34, "Cancel", c, "secondary", w=84)
    button(pc, PW - 16 - pw, y + 34, primary, c, "disabled" if errors else "primary", w=pw)
    return pc


def s_table_editor(x, y, c, tab="Columns"):
    name = "14 · Table editor" if tab == "Columns" else "15 · Object DDL"
    root = screen(name, x, y, c)
    shell(root, c, selected="customers", right="customers  ·  7 columns")
    te_header(root, c, tab)
    if tab == "DDL":
        body = frame(root, MX, TOP + 92, MW, H - TOP - 92 - STATUS, fill=c["bg"], name="DDL")
        bx = MW - 24
        for lbl, kind in [("Open in a new tab", "secondary"), ("Copy", "ghost")]:
            w_ = int(len(lbl) * 7 + 24)
            bx -= w_
            button(body, bx, 16, lbl, c, kind, w=w_)
            bx -= 8
        ddl = [
            [("kw", "CREATE TABLE"), ("ident", " public.customers (")],
            [("ident", "    id "), ("kw", "bigint GENERATED ALWAYS AS IDENTITY"), ("punct", ",")],
            [("ident", "    email citext "), ("kw", "NOT NULL"), ("punct", ",")],
            [("ident", "    full_name text"), ("punct", ",")],
            [("ident", "    region varchar("), ("num", "8"), ("ident", ") "), ("kw", "NOT NULL"),
             ("kw", " DEFAULT"), ("str", " 'NA'"), ("punct", ",")],
            [("ident", "    referred_by bigint"), ("punct", ",")],
            [("ident", "    created_at timestamptz "), ("kw", "NOT NULL DEFAULT"),
             ("ident", " now()"), ("punct", ",")],
            [("kw", "    CONSTRAINT"), ("ident", " customers_pkey "), ("kw", "PRIMARY KEY"),
             ("ident", " (id)"), ("punct", ",")],
            [("kw", "    CONSTRAINT"), ("ident", " customers_email_key "), ("kw", "UNIQUE"),
             ("ident", " (email)"), ("punct", ",")],
            [("kw", "    CONSTRAINT"), ("ident", " customers_referred_by_fkey "),
             ("kw", "FOREIGN KEY"), ("ident", " (referred_by)")],
            [("kw", "        REFERENCES"), ("ident", " public.customers (id) "),
             ("kw", "ON DELETE SET NULL"), ("punct", ",")],
            [("kw", "    CONSTRAINT"), ("ident", " customers_region_check "), ("kw", "CHECK"),
             ("ident", " (region <> "), ("str", "''"), ("ident", ")")],
            [("punct", ");")],
            [],
            [("kw", "CREATE INDEX"), ("ident", " customers_region_idx "), ("kw", "ON"),
             ("ident", " public.customers (region);")],
            [("kw", "CREATE INDEX"), ("ident", " customers_created_idx "), ("kw", "ON"),
             ("ident", " public.customers (created_at "), ("kw", "DESC"), ("ident", ");")],
            [],
            [("kw", "COMMENT ON COLUMN"), ("ident", " public.customers.email "), ("kw", "IS"),
             ("str", " 'Login and receipts'"), ("punct", ";")],
        ]
        sql_lines(body, 16, 64, ddl, c)
        return
    LW = MW - 400
    body = frame(root, MX, TOP + 92, LW, H - TOP - 92 - STATUS, fill=c["bg"], name="Columns")
    cols = [("", 36), ("Name", 190), ("Type", 170), ("Null", 60), ("Default", 150), ("Keys", 130)]
    hdr = frame(body, 16, 16, LW - 32, 34, fill=c["panel"], name="Col header",
                strokeBottomWeight=1, stroke=c["border"])
    hx = 0
    for n_, w in cols:
        text(hdr, hx + 10, 9, n_, c["muted"], size=12, weight="Semi Bold")
        hx += w
    rows = [("id", "bigint", False, "identity", ["PK"], None),
            ("email", "citext", False, "", ["UNIQUE"], None),
            ("full_name", "text", True, "", [], None),
            ("region", "varchar(8)", False, "'NA'", [], "changed"),
            ("referred_by", "bigint", True, "", ["FK"], None),
            ("created_at", "timestamptz", False, "now()", [], None),
            ("marketing_opt_in", "boolean", False, "false", [], "added")]
    for i, (n_, t_, null, dflt, keys, state) in enumerate(rows):
        ry = 50 + i * 40
        fill = {"added": c["ok"] + "14", "changed": c["warn"] + "14"}.get(state, "none")
        r = frame(body, 16, ry, LW - 32, 40, fill=fill, name=f"Column/{n_}",
                  strokeBottomWeight=1, stroke=c["border"])
        if state:
            rect(r, 0, 0, 2, 40, c["ok"] if state == "added" else c["warn"])
        for k in range(2):
            for j in range(3):
                ellipse(r, 13 + k * 5, 14 + j * 5, 2.4, c["faint"])
        rx = 36
        text(r, rx + 10, 11, n_, c["text"], size=13, family=MONO)
        rx += 190
        tb = frame(r, rx + 6, 6, 158, 28, fill=c["panel"], r=6, stroke=c["border"],
                   name="Type select")
        text(tb, 10, 6, t_, c["text"], size=12.5, family=MONO)
        chevron(tb, 140, 8, c["faint"])
        rx += 170
        cb = frame(r, rx + 20, 12, 16, 16, fill=c["accent"] if null else "none", r=4,
                   stroke=None if null else c["border2"], name="Nullable")
        if null:
            text(cb, 3, -1, "✓", "#FFFFFF", size=12, weight="Bold")
        rx += 60
        text(r, rx + 10, 11, dflt or "—", c["text"] if dflt else c["faint"], size=12.5,
             family=MONO if dflt else SANS)
        rx += 150
        kx = rx + 10
        for k in keys:
            fg = {"PK": c["amber"], "FK": c["accent"], "UNIQUE": c["purple"]}[k]
            kx += pill(r, kx, 10, k, fg, fg + "22") + 6
    add_row = frame(body, 16, 50 + len(rows) * 40 + 8, 140, 30, fill="none", r=6,
                    name="Add column")
    text(add_row, 10, 6, "+  Add column", c["accent"], size=12.5, weight="Medium")
    pending_panel(root, c, LW, [("warn", "Change default of region"),
                                ("ok", "Add column marketing_opt_in")], [
        [("kw", "ALTER TABLE"), ("ident", " public.customers")],
        [("kw", "  ALTER COLUMN"), ("ident", " region")],
        [("kw", "    SET DEFAULT"), ("ident", " "), ("str", "'NA'"), ("punct", ",")],
        [("kw", "  ADD COLUMN"), ("ident", " marketing_opt_in")],
        [("kw", "    boolean NOT NULL")],
        [("kw", "    DEFAULT"), ("ident", " "), ("kw", "false"), ("punct", ";")]],
        "Apply 2 changes")


def s_import(x, y, c):
    root = screen("16 · Import", x, y, c)
    shell(root, c)
    frame(root, MX, TOP, MW, H - TOP - STATUS, fill=c["bg"], name="Main (behind)")
    scrim(root, c)
    DW, DH = 860, 640
    d = sheet(root, c, DW, DH, "Import into orders",
              "In one transaction: all of it goes in, or none of it.")
    f = frame(d, 28, 96, DW - 56, 64, fill=c["bg"], r=10, stroke=c["border"], name="File")
    fi = frame(f, 14, 14, 36, 36, fill=c["ok"] + "22", r=8, name="File icon")
    text(fi, 0, 10, "CSV", c["ok"], size=10.5, weight="Bold", width=36, align="center")
    text(f, 62, 13, "orders-2026-10.csv", c["text"], size=13.5, weight="Semi Bold")
    text(f, 62, 34, "~/Downloads  ·  2.4 MB  ·  18,402 rows  ·  comma-separated, UTF-8",
         c["muted"], size=12)
    button(f, DW - 56 - 14 - 84, 18, "Change", c, "secondary", w=84)
    for i, (on, lbl) in enumerate([(True, "The first row names the columns"),
                                   (True, "Empty fields are NULL")]):
        cx = 28 + i * 280
        cb = frame(d, cx, 182, 16, 16, fill=c["accent"], r=4, name=lbl)
        text(cb, 3, -1, "✓", "#FFFFFF", size=12, weight="Bold")
        text(d, cx + 26, 181, lbl, c["text"], size=12.5)
    tbl = frame(d, 28, 218, DW - 56, 290, fill=c["bg"], r=10, stroke=c["border"],
                name="Mapping")
    hdr = frame(tbl, 0, 0, DW - 56, 36, fill=c["panel"], name="Map header",
                strokeBottomWeight=1, stroke=c["border"])
    for n_, cx in [("In the file", 16), ("", 230), ("Goes into", 266), ("First values", 500)]:
        text(hdr, cx, 10, n_, c["muted"], size=12, weight="Semi Bold")
    maps = [("order_id", "id", "bigint", "40211, 40212, 40213"),
            ("customer", "customer_id", "bigint", "118, 9021, 118"),
            ("placed", "created_at", "timestamptz", "2026-10-01 09:12, 2026-10-01 …"),
            ("amount_eur", "total", "numeric", "140.39, 88.00, 412.10"),
            ("state", "status", "text", "paid, paid, refunded"),
            ("notes", None, None, "gift wrap, —, —")]
    for i, (src, dst, typ, vals) in enumerate(maps):
        ry = 36 + i * 42
        r = frame(tbl, 0, ry, DW - 56, 42, fill="none", name=f"Map/{src}",
                  strokeBottomWeight=1 if i < 5 else 0, stroke=c["border"])
        text(r, 16, 12, src, c["text"], size=13, family=MONO)
        text(r, 230, 11, "→", c["faint"], size=14)
        sel = frame(r, 266, 6, 220, 30, fill=c["panel"] if dst else c["bg"], r=6,
                    stroke=c["border2"], dashes=None if dst else [4, 3], name="Target column")
        text(sel, 10, 6, dst or "Skip this column", c["text"] if dst else c["faint"], size=12.5,
             family=MONO if dst else SANS)
        if typ:
            text(sel, 220 - 30 - len(typ) * 6.2, 8, typ, c["faint"], size=11)
        chevron(sel, 204, 9, c["faint"])
        text(r, 500, 12, vals, c["muted"] if dst else c["faint"], size=12.5, family=MONO)
    text(d, 28, 524, "6 columns in the file  ·  5 go in  ·  orders has 3 more, which take their "
         "defaults", c["faint"], size=12)
    footer(d, c, DW, DH, "Import 18,402 rows")


def s_diff(x, y, c):
    root = screen("17 · Compare schemas", x, y, c)
    shell(root, c)
    frame(root, MX, TOP, MW, H - TOP - STATUS, fill=c["bg"], name="Main (behind)")
    scrim(root, c)
    DW, DH = 900, 660
    d = sheet(root, c, DW, DH, "Compare schemas",
              "The SQL that makes the target like the source opens in a tab.")
    for i, (lbl, conn, drv, sch) in enumerate([("Source", "staging", "Postgres", "shop"),
                                               ("Target", "prod-db", "Postgres", "shop")]):
        bx = 28 + i * 440
        text(d, bx, 98, lbl, c["muted"], size=12, weight="Medium")
        b = frame(d, bx, 118, 404, 40, fill=c["bg"], r=8, stroke=c["border2"],
                  name=f"{lbl} picker")
        driver_badge(b, 10, 10, drv, size=20)
        text(b, 40, 11, conn, c["text"], size=13, weight="Semi Bold")
        text(b, 40 + len(conn) * 8 + 10, 12, sch, c["muted"], size=12.5)
        chevron(b, 380, 15, c["muted"])
    text(d, 438, 128, "→", c["faint"], size=16)
    sm = frame(d, 28, 178, DW - 56, 40, fill="none", name="Summary")
    sx = 0
    for lbl, col in [("2 tables added", c["ok"]), ("3 changed", c["warn"]),
                     ("1 removed", c["bad"]), ("38 the same", c["faint"])]:
        sx += pill(sm, sx, 10, lbl, col, col + "22", h=22, size=11.5) + 8
    lst = frame(d, 28, 226, 330, 350, fill=c["bg"], r=10, stroke=c["border"], name="Changes")
    items = [("+", "ok", "table", "loyalty_points", "new table"),
             ("+", "ok", "table", "loyalty_tiers", "new table"),
             ("~", "warn", "table", "customers", "1 column, 1 index"),
             ("~", "warn", "table", "orders", "column type"),
             ("~", "warn", "view", "orders_paid", "definition"),
             ("−", "bad", "fn", "legacy_total()", "dropped")]
    for i, (sym, col, kind, n_, what) in enumerate(items):
        r = frame(lst, 6, 6 + i * 40, 318, 36, fill=c["accent_soft"] if i == 2 else "none", r=7,
                  name=f"Change/{n_}")
        text(r, 10, 8, sym, c[col], size=14, weight="Bold", family=MONO)
        kind_icon(r, 30, 11, kind, c)
        text(r, 52, 9, n_, c["text"], size=13, weight="Medium")
        text(r, 318 - 12 - len(what) * 6.3, 10, what, c["faint"], size=11.5)
    code = frame(d, 370, 226, DW - 28 - 370, 350, fill=c["bg"], r=10, stroke=c["border"],
                 name="Diff SQL")
    text(code, 16, 14, "customers", c["text"], size=13, weight="Semi Bold")
    text(code, 100, 15, "target → source", c["faint"], size=12)
    diff = [("-", "    region varchar(8) NOT NULL DEFAULT 'NA',"),
            ("+", "    region varchar(16) NOT NULL DEFAULT 'NA',"),
            (" ", "    referred_by bigint,"),
            ("+", "    loyalty_tier smallint REFERENCES loyalty_tiers,"),
            (" ", "    created_at timestamptz NOT NULL DEFAULT now()")]
    for i, (k, l_) in enumerate(diff):
        ly = 46 + i * 24
        if k != " ":
            rect(code, 1, ly - 3, DW - 28 - 372, 22, (c["ok"] if k == "+" else c["bad"]) + "1C")
        text(code, 14, ly, k, c["ok"] if k == "+" else (c["bad"] if k == "-" else c["faint"]),
             size=12.5, family=MONO)
        text(code, 32, ly, l_, c["text"] if k != " " else c["muted"], size=12.5, family=MONO)
    label_caps(code, 16, 186, "MIGRATION", c)
    sql_lines(code, 0, 208, [
        [("kw", "ALTER TABLE"), ("ident", " shop.customers")],
        [("kw", "  ALTER COLUMN"), ("ident", " region "), ("kw", "TYPE"), ("ident", " varchar("),
         ("num", "16"), ("ident", ")"), ("punct", ",")],
        [("kw", "  ADD COLUMN"), ("ident", " loyalty_tier smallint")],
        [("kw", "    REFERENCES"), ("ident", " shop.loyalty_tiers"), ("punct", ";")]], c)
    footer(d, c, DW, DH, "Open migration in a tab", secondary="Close")


def s_diagram(x, y, c):
    root = screen("18 · ER diagram", x, y, c)
    shell(root, c, right="shop  ·  6 tables  ·  5 relations")
    tabs_bar(root, c, tabs=[("Diagram · shop", "Postgres", False)] + TABS[:1])
    cv = frame(root, MX, ED_Y, MW, H - ED_Y - STATUS, fill=c["bg"], name="Canvas")
    # Dot grid (sparse to keep the file small).
    for gx in range(24, MW, 48):
        for gy in range(24, 780, 48):
            rect(cv, gx, gy, 1.5, 1.5, c["border2"])
    tb = frame(cv, 16, 16, 380, 36, fill=c["panel"], r=8, stroke=c["border"], name="Toolbar",
               effects=shadow(c["shadow"], 4, 12))
    bx = 6
    for lbl in ["Fit", "Tidy", "Copy as Mermaid", "Save as SVG…"]:
        w_ = int(len(lbl) * 7.0 + 20)
        f = frame(tb, bx, 4, w_, 28, fill="none", r=6, name=f"Tool/{lbl}")
        text(f, 10, 6, lbl, c["text"], size=12.5, weight="Medium")
        bx += w_ + 2
    zm = frame(cv, MW - 120, 16, 104, 36, fill=c["panel"], r=8, stroke=c["border"], name="Zoom")
    text(zm, 12, 8, "−", c["muted"], size=16)
    text(zm, 36, 10, "100%", c["text"], size=12.5, weight="Medium")
    text(zm, 82, 7, "+", c["muted"], size=16)
    tables = {
        "customers": (80, 110, [("id", "bigint", "PK"), ("email", "citext", "U"),
                                ("region", "varchar(8)", ""), ("referred_by", "bigint", "FK"),
                                ("created_at", "timestamptz", "")]),
        "orders": (440, 80, [("id", "bigint", "PK"), ("customer_id", "bigint", "FK"),
                             ("status", "text", ""), ("total", "numeric", ""),
                             ("created_at", "timestamptz", "")]),
        "order_items": (800, 120, [("id", "bigint", "PK"), ("order_id", "bigint", "FK"),
                                   ("product_id", "bigint", "FK"), ("qty", "int4", ""),
                                   ("price", "numeric", "")]),
        "products": (800, 420, [("id", "bigint", "PK"), ("supplier_id", "bigint", "FK"),
                                ("name", "text", ""), ("price", "numeric", "")]),
        "suppliers": (440, 470, [("id", "bigint", "PK"), ("name", "text", ""),
                                 ("country", "char(2)", "")]),
        "refunds": (80, 430, [("id", "bigint", "PK"), ("order_id", "bigint", "FK"),
                              ("amount", "numeric", ""), ("reason", "text", "")]),
    }
    TW = 250

    def anchor(t, row, side):
        tx_, ty_, _ = tables[t]
        return (tx_ + (TW if side == "r" else 0), ty_ + 40 + row * 28 + 14)

    edges = [("orders", 1, "l", "customers", 0, "r"), ("order_items", 1, "l", "orders", 0, "r"),
             ("order_items", 2, "l", "products", 0, "l"), ("products", 1, "l", "suppliers", 0, "r"),
             ("refunds", 1, "r", "orders", 0, "l"), ("customers", 3, "l", "customers", 0, "l")]
    for a, ar, aside, b, br, bside in edges:
        x1, y1 = anchor(a, ar, aside)
        x2, y2 = anchor(b, br, bside)
        if a == b:
            path = f"M{x1} {y1} C{x1 - 50} {y1} {x2 - 50} {y2} {x2} {y2}"
        elif aside == bside:
            path = f"M{x1} {y1} C{x1 - 70} {y1} {x2 - 70} {y2} {x2} {y2}"
        else:
            mx_ = (x1 + x2) / 2
            path = f"M{x1} {y1} C{mx_} {y1} {mx_} {y2} {x2} {y2}"
        xs, ys = [x1, x2, x1 - 70, x2 - 70, (x1 + x2) / 2], [y1, y2]
        bx0, by0 = min(xs), min(ys)
        # Vectors are positioned by their path's box; give the box explicitly.
        add("node.create", {"type": "vector", "parent": cv, "path": path, "name": f"{a}→{b}",
                            "select": False,
                            "props": {"fill": "none", "stroke": c["accent"] + "AA",
                                      "strokeWeight": 1.5}})
        ellipse(cv, x2 - 4, y2 - 4, 8, c["accent"])
    for n_, (tx_, ty_, cols) in tables.items():
        sel = n_ == "orders"
        t = frame(cv, tx_, ty_, TW, 40 + len(cols) * 28 + 6, fill=c["panel"], r=10,
                  stroke=c["accent"] if sel else c["border2"], name=f"Table/{n_}",
                  effects=shadow(c["shadow"], 6, 18))
        hb = frame(t, 0, 0, TW, 40, fill=c["accent_soft"] if sel else c["raised"], name="Head",
                   strokeBottomWeight=1, stroke=c["border"])
        kind_icon(hb, 12, 13, "table", c)
        text(hb, 34, 11, n_, c["text"], size=13.5, weight="Semi Bold")
        for i, (cn, ct, k) in enumerate(cols):
            ry = 43 + i * 28
            if k:
                fg = {"PK": c["amber"], "FK": c["accent"], "U": c["purple"]}[k]
                text(t, 12, ry + 5, k, fg, size=10, weight="Bold")
            text(t, 38, ry + 4, cn, c["text"], size=12.5, family=MONO)
            text(t, TW - 12 - len(ct) * 6.6, ry + 5, ct, c["faint"], size=11.5)


def s_settings(x, y, c):
    root = screen("19 · Settings", x, y, c)
    shell(root, c)
    frame(root, MX, TOP, MW, H - TOP - STATUS, fill=c["bg"], name="Main (behind)")
    scrim(root, c)
    DW, DH = 820, 560
    d = sheet(root, c, DW, DH, "Settings")
    rect(d, 0, 68, DW, 1, c["border"])
    nav = frame(d, 0, 69, 200, DH - 69 - 68, fill=c["bg"], name="Nav", strokeRightWeight=1,
                stroke=c["border"])
    for i, n_ in enumerate(["Appearance", "Editor", "Results", "Connections", "Shortcuts"]):
        r = frame(nav, 10, 12 + i * 36, 180, 32, fill=c["accent_soft"] if i == 0 else "none",
                  r=7, name=f"Nav/{n_}")
        text(r, 12, 7, n_, c["text"] if i == 0 else c["muted"], size=13,
             weight="Semi Bold" if i == 0 else "Medium")
    fx = 228
    text(d, fx, 92, "Theme", c["text"], size=14, weight="Semi Bold")
    for i, (n_, pal) in enumerate([("System", None), ("Light", LIGHT), ("Dark", DARK)]):
        cx = fx + i * 190
        sel = n_ == ("Dark" if c is DARK else "Light")
        card = frame(d, cx, 120, 176, 116, fill=c["bg"], r=10,
                     stroke=c["accent"] if sel else c["border"],
                     strokeWeight=2 if sel else 1, name=f"Theme/{n_}")
        if pal is None:
            pv = frame(card, 10, 10, 156, 70, fill=LIGHT["bg"], r=6, name="Preview")
            rect(pv, 78, 0, 78, 70, DARK["bg"])
            rect(pv, 0, 0, 40, 70, LIGHT["panel"])
            rect(pv, 78, 0, 30, 70, DARK["panel"])
            rect(pv, 50, 14, 22, 5, LIGHT["accent"], r=2)
            rect(pv, 116, 14, 30, 5, DARK["accent"], r=2)
        else:
            pv = frame(card, 10, 10, 156, 70, fill=pal["bg"], r=6, name="Preview")
            rect(pv, 0, 0, 40, 70, pal["panel"])
            rect(pv, 50, 14, 60, 5, pal["accent"], r=2)
            for j in range(4):
                rect(pv, 50, 28 + j * 9, 90 - j * 12, 4, pal["border2"], r=2)
        text(card, 12, 90, n_, c["text"], size=12.5, weight="Semi Bold" if sel else "Medium")
        if sel:
            ellipse(card, 152, 92, 12, c["accent"])
    text(d, fx, 266, "Editor text size", c["text"], size=14, weight="Semi Bold")
    text(d, fx, 290, "The SQL editor and the results grid.", c["muted"], size=12.5)
    rect(d, fx, 330, 420, 4, c["border2"], r=2)
    rect(d, fx, 330, 170, 4, c["accent"], r=2)
    ellipse(d, fx + 162, 324, 16, "#FFFFFF")
    input_box(d, fx + 440, 316, 80, "13 px", c, h=32, mono=True)
    text(d, fx, 368, "Rows kept per result", c["text"], size=14, weight="Semi Bold")
    text(d, fx, 392, "More rows use more memory; export streams every row regardless.",
         c["muted"], size=12.5)
    input_box(d, fx, 420, 160, "10,000", c, h=32, mono=True)
    footer(d, c, DW, DH, "Done", secondary=None)


def s_confirms(x, y, c):
    root = screen("20 · Confirmations", x, y, c)
    text(root, 64, 48, "Confirmations", c["text"], size=22, weight="Semi Bold")
    text(root, 64, 82, "One shape for every question: what happens, then the way out, then the "
         "action. Destructive actions are red, never the default.", c["muted"], size=13.5)
    items = [
        ("Discard unsaved edits?", "1 edit in the orders grid hasn't been saved.",
         "Discard", True, "Review & save"),
        ("Close with a transaction open?", "2 statements haven't been committed. Closing rolls "
         "them back.", "Roll back & close", True, "Commit & close"),
        ("Save orders_by_month.sql?", "Your changes are lost if you don't save them.",
         "Save", False, "Don't save"),
        ("Delete the staging connection?", "Its saved password is removed from the Keychain too. "
         "Your database isn't touched.", "Delete", True, None),
        ("Clear the history?", "All 1,284 queries in the history are removed. This can't be "
         "undone.", "Clear history", True, None),
        ("Quit DataDrop?", "2 tabs have unsaved changes.", "Quit without saving", True,
         "Review"),
    ]
    for i, (t, body, primary, danger, alt) in enumerate(items):
        cx = 64 + (i % 3) * 448
        cy = 140 + (i // 3) * 220
        d = frame(root, cx, cy, 424, 172, fill=c["panel"], r=14, stroke=c["border2"],
                  name=f"Confirm/{t}", effects=shadow(c["shadow"], 16, 40))
        ic = frame(d, 24, 24, 36, 36, fill=(c["bad"] if danger else c["accent"]) + "22", r=10,
                   name="Icon")
        text(ic, 0, 6, "!" if danger else "?", c["bad"] if danger else c["accent"], size=17,
             weight="Bold", width=36, align="center")
        text(d, 76, 24, t, c["text"], size=15, weight="Semi Bold", width=320)
        text(d, 76, 52, body, c["muted"], size=12.5, width=320)
        pw = int(len(primary) * 7.2 + 28)
        button(d, 424 - 24 - pw, 120, primary, c, "danger" if danger else "primary", w=pw)
        bx = 424 - 24 - pw - 10
        if alt:
            aw = int(len(alt) * 7.2 + 28)
            bx -= aw
            button(d, bx, 120, alt, c, "secondary" if danger else "ghost", w=aw)
            bx -= 10
        button(d, 24, 120, "Cancel", c, "ghost", w=72)


def s_states(x, y, c):
    root = screen("21 · Menus, toasts and states", x, y, c)
    text(root, 64, 48, "Menus, toasts and states", c["text"], size=22, weight="Semi Bold")
    def menu(x, title, items, hi=None):
        label_caps(root, x, 100, title, c)
        h = 12 + sum(9 if n_ == "-" else 28 for n_, _ in items)
        m = frame(root, x, 124, 270, h, fill=c["raised"], r=10, stroke=c["border2"],
                  name=f"Menu/{title}", effects=shadow(c["shadow"], 10, 28))
        my = 6
        for n_, k in items:
            if n_ == "-":
                rect(m, 8, my + 4, 254, 1, c["border"])
                my += 9
                continue
            r = frame(m, 6, my, 258, 26, fill=c["accent_soft"] if n_ == hi else "none", r=6,
                      name=f"Item/{n_}")
            bad = n_.startswith("Delete")
            text(r, 12, 5, n_, c["bad"] if bad else c["text"], size=12.5)
            if k:
                text(r, 258 - 12 - len(k) * 7, 6, k, c["faint"], size=11.5)
            my += 28

    menu(64, "CONNECTION", [("New query tab", ""), ("Show DDL of everything", ""), ("-", ""),
                            ("Edit…", ""), ("Refresh", ""), ("-", ""), ("Delete…", "")])
    menu(350, "TABLE", [("Show rows", ""), ("New query tab", ""), ("-", ""),
                        ("Edit table…", ""), ("Show DDL", ""), ("Diagram", ""),
                        ("Import data…", ""), ("Compare with…", ""), ("-", ""),
                        ("Copy name", ""), ("Refresh", "")], hi="Edit table…")
    menu(636, "SCHEMA / TABLES", [("New table…", ""), ("Diagram", ""), ("Compare with…", ""),
                                  ("Show DDL", ""), ("-", ""), ("Refresh", "")])
    menu(922, "GRID CELL", [("Copy value", "⌘C"), ("Copy row", ""), ("-", ""),
                            ("Edit the value", ""), ("Set to NULL", ""), ("Undo this change", ""),
                            ("-", ""), ("Delete the row", "")])
    label_caps(root, 1208, 100, "TOASTS", c)
    toast(root, c, 1208, 124, "ok", "Saved 2 changes", "orders · in 12 ms", w=200)
    toast(root, c, 1208, 200, "info", "Nothing has changed.", w=200)
    toast(root, c, 1208, 256, "bad", "Export failed", "~/out.csv: permission denied", w=200)
    label_caps(root, 64, 440, "BANNERS", c)
    for i, (col, t) in enumerate([
            (c["accent"], "ClickHouse results are read-only here"),
            (c["muted"], "Read-only: this result isn't one table with a primary key"),
            (c["warn"], "Reconnecting to prod-db…"),
            (c["bad"], "Lost the connection to prod-db")]):
        bx = [64, 392, 804, 1132][i]
        bw = [316, 400, 316, 244][i]
        b = frame(root, bx, 464, bw, 40, fill=col + "16", r=8, stroke=col + "55",
                  name=f"Banner/{i}")
        ellipse(b, 14, 16, 8, col)
        text(b, 30, 11, t, c["text"], size=12.5, weight="Medium")
    label_caps(root, 64, 540, "EMPTY STATES", c)
    for i, (t, b) in enumerate([
            ("Run a query to see results", "Run a statement with ⌘↵ (or the selection), or the "
             "whole tab with ⇧⌘↵."),
            ("No rows", "The query ran in 4 ms and matched nothing."),
            ("Queries you run appear here", "Each run is kept with its connection, time and "
             "result.")]):
        e = frame(root, 64 + i * 400, 564, 380, 140, fill=c["panel"], r=12, stroke=c["border"],
                  name=f"Empty/{t}")
        ic = frame(e, 166, 18, 48, 48, fill=c["hover"], r=12, name="Icon")
        rect(ic, 12, 16, 24, 3, c["faint"], r=1.5)
        rect(ic, 12, 23, 18, 3, c["faint"], r=1.5)
        rect(ic, 12, 30, 22, 3, c["faint"], r=1.5)
        text(e, 0, 78, t, c["text"], size=13.5, weight="Semi Bold", width=380, align="center")
        text(e, 30, 100, b, c["muted"], size=12, width=320, align="center")
    label_caps(root, 64, 740, "CONNECTION STATUS", c)
    for i, (col, t) in enumerate([(c["ok"], "Connected"), (c["accent"], "Running"),
                                  (c["warn"], "Transaction open"), (c["bad"], "Disconnected"),
                                  (c["faint"], "Not connected")]):
        px = 64 + i * 200
        p = frame(root, px, 764, 184, 32, fill=c["panel"], r=16, stroke=c["border"],
                  name=f"Status/{t}")
        ellipse(p, 14, 12, 8, col)
        text(p, 30, 7, t, c["text"], size=12.5, weight="Medium")


# --- screens for what the app does that the first pass missed --------------------------------

ORDER_COLS = [("#", "", 48, "r"), ("id", "int8", 100, "r"), ("customer_id", "int8", 130, "r"),
              ("status", "text", 120, "l"), ("total", "numeric", 120, "r"),
              ("created_at", "timestamptz", 230, "l"), ("notes", "text", 300, "l")]
ORDERS = [
    ("40211", "118", "shipped", "140.39", "2026-10-21 09:12:44+00", "Leave at the side door"),
    ("40207", "9021", "shipped", "88.00", "2026-10-21 08:57:02+00", None),
    ("40198", "118", "shipped", "412.10", "2026-10-20 17:31:19+00", "Gift wrap, no receipt"),
    ("40176", "5530", "shipped", "61.25", "2026-10-20 11:04:51+00", None),
    ("40170", "771", "shipped", "1,204.00", "2026-10-20 09:48:30+00", "Call before deliver…"),
    ("40166", "118", "shipped", "19.99", "2026-10-19 22:13:07+00", None),
    ("40151", "3302", "shipped", "240.00", "2026-10-19 15:40:12+00", None),
    ("40139", "9021", "shipped", "75.50", "2026-10-19 10:02:58+00", "Fragile"),
]

MULTI = [
    [("com", "-- Ship last week's paid orders, then check them")],
    [("kw", "UPDATE"), ("ident", " orders "), ("kw", "SET"), ("ident", " status = "),
     ("str", "'shipped'")],
    [("kw", " WHERE"), ("ident", " status = "), ("str", "'paid'"), ("kw", " AND"),
     ("ident", " created_at < now() - "), ("kw", "interval"), ("str", " '7 days'"),
     ("punct", ";")],
    [],
    [("kw", "SELECT"), ("ident", " * "), ("kw", "FROM"), ("ident", " orders "), ("kw", "WHERE"),
     ("ident", " status = "), ("str", "'shipped'"), ("kw", " ORDER BY"),
     ("ident", " created_at "), ("kw", "DESC"), ("punct", ";")],
    [],
    [("kw", "SELECT"), ("ident", " count(*) "), ("kw", "FROM"), ("ident", " order_items"),
     ("punct", ";")],
]


def result_sets(rs, c, y, sets, active):
    """One entry per statement that ran; picking one shows it."""
    row = frame(rs, 0, y, MW, 38, fill=c["panel"], name="Result sets", strokeTopWeight=1,
                stroke=c["border"])
    x = 12
    for i, (kind, what, warn) in enumerate(sets):
        on = i == active
        label = f"{i + 1}   {kind}   ·   {what}"
        w = int(len(label) * 6.9 + (26 if warn else 0) + 28)
        b = frame(row, x, 5, w, 28, fill=c["bg"] if on else "none", r=7,
                  stroke=c["accent"] if on else c["border"], name=f"Result {i + 1}")
        text(b, 12, 6, f"{i + 1}", c["faint"], size=12, family=MONO)
        text(b, 30, 6, kind, c["text"] if on else c["muted"], size=12, weight="Semi Bold")
        text(b, 34 + len(kind) * 8, 6, f"·   {what}", c["muted"], size=12)
        if warn:
            text(b, w - 26, 5, "⚠", c["warn"], size=13, family="Noto Sans Symbols 2")
        x += w + 8
    return y + 38


def s_multi(x, y, c):
    root = screen("22 · Several statements", x, y, c)
    shell(root, c, right="Ln 5, Col 72   ·   UTF-8   ·   Spaces: 2")
    tabs_bar(root, c, tabs=[("ship_orders.sql", "Postgres", True)] + TABS[1:])
    editor(root, c, ED_Y, 196, lines=MULTI, cursor=None)
    rs = results_panel(root, c, ED_Y + 196, badge=None)
    text(rs, 200, 14, "3 statements ran   ·   96 ms", c["muted"], size=12.5)
    results_header(rs, c, timing="", where=None)
    gy = result_sets(rs, c, 46, [("UPDATE", "412 rows affected", False),
                                 ("SELECT", "10,000 rows", True),
                                 ("SELECT", "1 row", False)], active=1)
    warn = frame(rs, 0, gy, MW, 34, fill=c["warn"] + "14", name="Truncated",
                 strokeTopWeight=1, stroke=c["warn"] + "44")
    text(warn, 16, 9, "⚠", c["warn"], size=13, family="Noto Sans Symbols 2")
    text(warn, 36, 9, "10,000 rows shown, 2,318 more not shown.", c["text"], size=12.5,
         weight="Medium")
    text(warn, 316, 9, "Raise the row limit to see more, or export the query.", c["muted"],
         size=12.5)
    where_bar(rs, c, gy + 34)
    data_grid(rs, c, gy + 72, 400, cols=ORDER_COLS, data=ORDERS, sort="created_at")


def s_edit_mode(x, y, c):
    root = screen("23 · Editing rows", x, y, c)
    shell(root, c)
    tabs_bar(root, c, tabs=[("orders", "Postgres", True)] + TABS[:1])
    editor(root, c, ED_Y, 120, lines=[[("kw", "SELECT"), ("ident", " * "), ("kw", "FROM"),
                                       ("ident", " orders "), ("kw", "WHERE"),
                                       ("ident", " status = "), ("str", "'shipped'"),
                                       ("kw", " ORDER BY"), ("ident", " created_at "),
                                       ("kw", "DESC"), ("punct", ";")]], cursor=None)
    rs = results_panel(root, c, ED_Y + 120, badge="8 rows")
    results_header(rs, c, where=None)
    eb = frame(rs, 0, 46, MW, 44, fill=c["accent_soft"], name="Edit bar", strokeTopWeight=1,
               stroke=c["border"])
    lock = frame(eb, 14, 12, 20, 20, fill=c["accent"], r=6, name="Editable")
    text(lock, 0, 2, "✎", "#FFFFFF", size=12, width=20, align="center")
    text(eb, 44, 13, "Editing orders", c["text"], size=13, weight="Semi Bold")
    text(eb, 156, 14, "rows are matched on id  ·  double-click a cell to edit it", c["muted"],
         size=12.5)
    bx = MW - 16
    for lbl, kind in [("Review & save  3", "primary"), ("Revert", "secondary"),
                      ("+ Row", "secondary")]:
        w_ = int(len(lbl) * 7.0 + 26)
        bx -= w_
        button(eb, bx, 8, lbl, c, kind, w=w_)
        bx -= 8
    g = data_grid(rs, c, 90, 400, cols=ORDER_COLS, data=ORDERS[:7], sort="created_at",
                  edited=(1, 6, "Ring twice"))
    # A deleted row: struck through, red, with Restore.
    ry = 40 + 3 * 30
    dl = frame(g, 0, ry, MW, 30, fill=c["bg"], name="Deleted row")
    rect(dl, 0, 0, MW, 30, c["bad"] + "14")
    rect(dl, 0, 0, 2, 30, c["bad"])
    cx = 0
    for (n_, t_, cw, al), v in zip(ORDER_COLS, ("4",) + ORDERS[3]):
        if v is not None:
            tw = len(v) * 7.5
            xx = cx + cw - 12 - tw if al == "r" else cx + 12
            text(dl, xx, 7, v, c["muted"], size=12.5, family=MONO if al == "r" else SANS,
                 textDecoration="strikethrough")
        cx += cw
    text(dl, MW - 80, 7, "Restore", c["accent"], size=12.5, weight="Medium")
    # An added row being typed in.
    ry = 40 + 7 * 30
    nr = frame(g, 0, ry, MW, 30, fill=c["bg"], name="New row")
    rect(nr, 0, 0, MW, 30, c["ok"] + "14")
    rect(nr, 0, 0, 2, 30, c["ok"])
    text(nr, 14, 7, "new", c["ok"], size=11, weight="Semi Bold")
    text(nr, 48 + 100 - 12 - 7 * 7.5, 7, "DEFAULT", c["faint"], size=12, family=MONO)
    cell = frame(nr, 148, 2, 130, 26, fill=c["bg"], r=4, stroke=c["accent"], strokeWeight=2,
                 name="Cell editor")
    text(cell, 10, 5, "9021", c["text"], size=12.5, family=MONO)
    rect(cell, 10 + 4 * 7.5 + 1, 5, 1.5, 16, c["accent"])
    for cx_, v in [(290, "pending"), (390 + 120 - 12 - 4 * 7.5, "0.00")]:
        text(nr, cx_, 7, v, c["faint"], size=12.5, family=MONO if v[0].isdigit() else SANS)
    # The value editor, open on a long note.
    ve = frame(rs, 48 + 100 + 130 + 120 + 120 + 230 - 360, 90 + 40 + 2 * 30 + 4, 360, 196,
               fill=c["raised"], r=10, stroke=c["border2"], name="Edit the value",
               effects=shadow(c["shadow"], 10, 28))
    text(ve, 14, 12, "notes", c["text"], size=12.5, weight="Semi Bold")
    text(ve, 62, 13, "text  ·  row 3", c["faint"], size=11.5)
    ta = frame(ve, 14, 38, 332, 102, fill=c["bg"], r=6, stroke=c["accent"], name="Text area")
    text(ta, 10, 8, "Gift wrap, no receipt.\nIf nobody is in, leave with the\nneighbour at "
         "number 12.", c["text"], size=12.5, lineHeight="150%", width=312)
    button(ve, 14, 154, "Set to NULL", c, "ghost")
    button(ve, 360 - 14 - 60 - 8 - 70, 154, "Cancel", c, "secondary", w=70)
    button(ve, 360 - 14 - 60, 154, "Done", c, "primary", w=60)
    bar = frame(rs, MW - 470, 90 + 330, 458, 40, fill=c["raised"], r=10, stroke=c["border2"],
                name="Pending edits", effects=shadow(c["shadow"], 6, 20))
    ellipse(bar, 14, 16, 8, c["warn"])
    text(bar, 30, 11, "3 unsaved: 1 changed, 1 deleted, 1 added", c["text"], size=12.5,
         weight="Medium")
    button(bar, 334, 6, "Review", c, "primary", h=28, w=110)


def s_conn_full(x, y, c):
    root = screen("24 · Connection — URL, TLS and SSH", x, y, c)
    shell(root, c)
    frame(root, MX, TOP, MW, H - TOP - STATUS, fill=c["bg"], name="Main (behind)")
    scrim(root, c)
    DW, DH = 760, 840
    d = sheet(root, c, DW, DH, "Edit connection", "prod-db  ·  Postgres", y=(H - DH) // 2)
    rect(d, 0, 84, DW, 1, c["border"])
    fx, fw = 28, DW - 56

    def radio(px, py, label, on):
        o = frame(d, px, py, 16, 16, fill="none", r=8, stroke=c["accent"] if on else c["border2"],
                  strokeWeight=1.5, name=f"Radio/{label}")
        if on:
            ellipse(o, 4, 4, 8, c["accent"])
        text(d, px + 24, py - 1, label, c["text"], size=12.5)
        return int(len(label) * 7.2 + 44)

    def section(py, title):
        label_caps(d, fx, py, title, c)

    section(104, "CONNECT WITH")
    rx = fx
    rx += radio(rx, 128, "Host and port", False)
    radio(rx, 128, "URL", True)
    input_box(d, fx, 158, fw, "postgres://app_readonly@db.internal.example:5432/shop", c,
              mono=True, focus=True)
    text(d, fx, 196, "Host, port, user and database come from the URL; the password stays "
         "separate.", c["faint"], size=12)
    input_box(d, fx, 244, fw - 160, "••••••••••••", c, label="Password")
    text(d, fx + fw - 148, 224, "Keep passwords", c["muted"], size=12, weight="Medium")
    radio(fx + fw - 148, 248, "In the keychain", True)
    input_box(d, fx, 312, fw - 104, "optional, a PEM file", c, placeholder=True,
              label="CA certificate")
    button(d, fx + fw - 92, 314, "Browse…", c, "secondary", w=92)
    rect(d, 0, 368, DW, 1, c["border"])
    tog = frame(d, fx, 388, 34, 20, fill=c["accent"], r=10, name="Toggle")
    ellipse(tog, 16, 2, 16, "#FFFFFF")
    text(d, fx + 46, 389, "SSH tunnel", c["text"], size=13, weight="Semi Bold")
    text(d, fx + 132, 390, "Connect through an SSH server", c["muted"], size=12.5)
    input_box(d, fx, 446, fw - 220, "bastion.example", c, label="SSH host")
    input_box(d, fx + fw - 208, 446, 88, "22", c, label="SSH port", mono=True)
    input_box(d, fx + fw - 108, 446, 108, "deploy", c, label="SSH user")
    text(d, fx, 494, "Sign in with", c["muted"], size=12, weight="Medium")
    seg = frame(d, fx, 514, 300, 32, fill=c["bg"], r=7, stroke=c["border"], name="SSH method")
    frame(seg, 202, 2, 96, 28, fill=c["raised"], r=5, stroke=c["border2"])
    for i, m in enumerate(["SSH agent", "Password", "Key file"]):
        text(seg, i * 100, 8, m, c["text"] if i == 2 else c["muted"], size=12, weight="Medium",
             width=100, align="center")
    input_box(d, fx, 580, fw - 104, "~/.ssh/id_ed25519", c, label="Key file", mono=True)
    button(d, fx + fw - 92, 582, "Browse…", c, "secondary", w=92)
    input_box(d, fx, 648, 320, "••••••••", c, label="Passphrase")
    text(d, fx + 336, 656, "kept like the password above", c["faint"], size=12)
    text(d, fx, 706, "✓", c["ok"], size=14, weight="Bold")
    text(d, fx + 20, 706, "Connected", c["ok"], size=12.5, weight="Medium")
    footer(d, c, DW, DH, "Save", left="Test connection")


def s_te_fks(x, y, c):
    root = screen("25 · Table editor — foreign keys", x, y, c)
    shell(root, c, selected="orders", right="orders  ·  2 foreign keys")
    te_header(root, c, "Foreign keys", name="orders",
              counts=(("Columns", "7"), ("Indexes", "2"), ("Foreign keys", "2"),
                      ("Checks", "1"), ("DDL", None)))
    LW = MW - 400
    body = frame(root, MX, TOP + 92, LW, H - TOP - 92 - STATUS, fill=c["bg"], name="Foreign keys")
    fks = [("orders_customer_id_fkey", "customer_id", "customers", "id", "RESTRICT", None),
           ("orders_coupon_fkey", "", "", "", "NO ACTION",
            "Foreign key 2 needs its columns and the table it refers to.")]
    for i, (n_, cols, tbl, ref, ondel, err) in enumerate(fks):
        cy = 20 + i * 214
        card = frame(body, 16, cy, LW - 32, 196, fill=c["panel"], r=10,
                     stroke=c["bad"] + "88" if err else c["border"], name=f"FK/{n_}")
        text(card, 16, 14, f"Foreign key {i + 1}", c["muted"], size=12, weight="Semi Bold")
        text(card, LW - 32 - 150, 14, "Remove", c["bad"], size=12.5, weight="Medium")
        input_box(card, 16, 58, 300, n_, c, label="Name", mono=True)
        text(card, 332, 38, "On delete", c["muted"], size=12, weight="Medium")
        sel = frame(card, 332, 58, 180, 32, fill=c["bg"], r=6, stroke=c["border2"],
                    name="On delete")
        text(sel, 10, 7, ondel, c["text"], size=12.5, family=MONO)
        chevron(sel, 160, 11, c["faint"])
        text(card, 16, 106, "Its columns", c["muted"], size=12, weight="Medium")
        cb = frame(card, 16, 126, 220, 32, fill=c["bg"], r=6,
                   stroke=c["bad"] if err else c["border2"], name="Its columns")
        if cols:
            pill(cb, 8, 6, cols, c["text"], c["hover"], h=20, size=11.5)
        else:
            text(cb, 10, 7, "Pick columns…", c["faint"], size=12.5)
        text(card, 248, 132, "→", c["faint"], size=15)
        text(card, 276, 106, "Refers to table", c["muted"], size=12, weight="Medium")
        tb = frame(card, 276, 126, 236, 32, fill=c["bg"], r=6,
                   stroke=c["bad"] if err else c["border2"], name="Refers to")
        text(tb, 10, 7, f"{tbl} ({ref})" if tbl else "Pick a table…",
             c["text"] if tbl else c["faint"], size=12.5, family=MONO if tbl else SANS)
        chevron(tb, 216, 11, c["faint"])
        if err:
            text(card, 16, 170, err, c["bad"], size=12)
    a = frame(body, 16, 20 + 2 * 214, 170, 30, fill="none", r=6, name="Add foreign key")
    text(a, 10, 6, "+  Add foreign key", c["accent"], size=12.5, weight="Medium")
    # The open "On delete" list.
    lst = frame(body, 16 + 332, 20 + 58 + 34, 180, 6 + 5 * 28, fill=c["raised"], r=8,
                stroke=c["border2"], name="On delete options", effects=shadow(c["shadow"], 8, 24))
    for j, o in enumerate(["NO ACTION", "RESTRICT", "CASCADE", "SET NULL", "SET DEFAULT"]):
        r = frame(lst, 4, 3 + j * 28, 172, 26, fill=c["accent_soft"] if o == "RESTRICT" else "none",
                  r=5, name=f"Option/{o}")
        text(r, 10, 5, o, c["text"], size=12.5, family=MONO)
    pending_panel(root, c, LW, [("warn", "Change on delete of orders_customer_id_fkey"),
                                ("ok", "Add foreign key orders_coupon_fkey")], [
        [("kw", "ALTER TABLE"), ("ident", " public.orders")],
        [("kw", "  DROP CONSTRAINT"), ("ident", " orders_customer_id_fkey"), ("punct", ",")],
        [("kw", "  ADD CONSTRAINT"), ("ident", " orders_customer_id_fkey")],
        [("kw", "    FOREIGN KEY"), ("ident", " (customer_id)")],
        [("kw", "    REFERENCES"), ("ident", " public.customers (id)")],
        [("kw", "    ON DELETE RESTRICT"), ("punct", ";")]], "Apply 2 changes",
        errors=["Foreign key 2 needs its columns and the table it refers to."])


def s_te_indexes(x, y, c):
    root = screen("26 · Table editor — indexes and checks", x, y, c)
    shell(root, c, selected="orders", right="orders  ·  2 indexes  ·  1 check")
    te_header(root, c, "Indexes", name="orders",
              counts=(("Columns", "7"), ("Indexes", "3"), ("Foreign keys", "1"),
                      ("Checks", "2"), ("DDL", None)))
    LW = MW - 400
    body = frame(root, MX, TOP + 92, LW, H - TOP - 92 - STATUS, fill=c["bg"], name="Indexes")
    hdr = frame(body, 16, 16, LW - 32, 34, fill=c["panel"], name="Index header",
                strokeBottomWeight=1, stroke=c["border"])
    for n_, hx in [("Name", 12), ("Its columns", 300), ("Unique", 600)]:
        text(hdr, hx, 9, n_, c["muted"], size=12, weight="Semi Bold")
    idx = [("orders_pkey", ["id"], True, "pk"), ("orders_status_created_idx",
            ["status", "created_at DESC"], False, "new"),
           ("orders_customer_idx", ["customer_id"], False, None)]
    for i, (n_, cols, uniq, state) in enumerate(idx):
        r = frame(body, 16, 50 + i * 44, LW - 32, 44,
                  fill=c["ok"] + "14" if state == "new" else "none", name=f"Index/{n_}",
                  strokeBottomWeight=1, stroke=c["border"])
        if state == "new":
            rect(r, 0, 0, 2, 44, c["ok"])
        text(r, 12, 13, n_, c["text"], size=13, family=MONO)
        if state == "pk":
            pill(r, 12 + len(n_) * 7.8 + 10, 12, "PRIMARY", c["amber"], c["amber"] + "22")
        px = 300
        for col in cols:
            px += pill(r, px, 12, col, c["text"], c["hover"], h=20, size=11.5) + 6
        cb = frame(r, 610, 14, 16, 16, fill=c["accent"] if uniq else "none", r=4,
                   stroke=None if uniq else c["border2"], name="Unique")
        if uniq:
            text(cb, 3, -1, "✓", "#FFFFFF", size=12, weight="Bold")
        if state != "pk":
            text(r, LW - 32 - 70, 13, "Remove", c["bad"], size=12.5, weight="Medium")
    a = frame(body, 16, 50 + 3 * 44 + 8, 140, 30, fill="none", r=6, name="Add index")
    text(a, 10, 6, "+  Add index", c["accent"], size=12.5, weight="Medium")
    label_caps(body, 16, 268, "CHECKS  (ON THE CHECKS TAB)", c)
    for i, (n_, expr) in enumerate([("orders_total_check", "CHECK (total >= 0)"),
                                    ("orders_status_check",
                                     "CHECK (status IN ('paid', 'shipped', 'refunded'))")]):
        r = frame(body, 16, 292 + i * 48, LW - 32, 40, fill=c["panel"], r=8, stroke=c["border"],
                  name=f"Check/{n_}")
        text(r, 12, 11, n_, c["muted"], size=12.5, family=MONO)
        text(r, 230, 11, expr, c["text"], size=12.5, family=MONO)
    pending_panel(root, c, LW, [("ok", "Add index orders_status_created_idx")], [
        [("kw", "CREATE INDEX"), ("ident", " orders_status_created_idx")],
        [("kw", "  ON"), ("ident", " public.orders (status, created_at "), ("kw", "DESC"),
         ("ident", ")"), ("punct", ";")]], "Apply 1 change")


def s_new_table(x, y, c):
    root = screen("27 · New table", x, y, c)
    shell(root, c, selected="", right="shop  ·  new table")
    te_header(root, c, "Columns", name="loyalty_tiers", sub="in public", new=True,
              counts=(("Columns", "3"), ("Indexes", None), ("Foreign keys", None),
                      ("Checks", None)))
    LW = MW - 400
    body = frame(root, MX, TOP + 92, LW, H - TOP - 92 - STATUS, fill=c["bg"], name="Columns")
    cols = [("", 36), ("Name", 190), ("Type", 170), ("Null", 60), ("Default", 150),
            ("In the primary key", 130)]
    hdr = frame(body, 16, 16, LW - 32, 34, fill=c["panel"], name="Col header",
                strokeBottomWeight=1, stroke=c["border"])
    hx = 0
    for n_, w in cols:
        text(hdr, hx + 10, 9, n_, c["muted"], size=12, weight="Semi Bold")
        hx += w
    rows = [("id", "smallint", False, "", True, None),
            ("name", "", False, "", False, "Give name a type."),
            ("", "numeric(5,2)", False, "0", False, "Every column needs a name.")]
    for i, (n_, t_, null, dflt, pk, err) in enumerate(rows):
        r = frame(body, 16, 50 + i * 40, LW - 32, 40, fill=c["bad"] + "0F" if err else "none",
                  name=f"Column/{i}", strokeBottomWeight=1, stroke=c["border"])
        nb = frame(r, 40, 6, 176, 28, fill=c["bg"], r=6,
                   stroke=c["bad"] if err and not n_ else c["border"], name="Name")
        text(nb, 10, 6, n_ or "column name", c["text"] if n_ else c["faint"], size=12.5,
             family=MONO)
        tb = frame(r, 232, 6, 158, 28, fill=c["panel"], r=6,
                   stroke=c["bad"] if err and not t_ else c["border"], name="Type select")
        text(tb, 10, 6, t_ or "type…", c["text"] if t_ else c["faint"], size=12.5, family=MONO)
        chevron(tb, 140, 8, c["faint"])
        cb = frame(r, 416, 12, 16, 16, fill="none", r=4, stroke=c["border2"], name="Nullable")
        text(r, 466, 11, dflt or "—", c["text"] if dflt else c["faint"], size=12.5,
             family=MONO if dflt else SANS)
        pk_ = frame(r, 616, 12, 16, 16, fill=c["accent"] if pk else "none", r=4,
                    stroke=None if pk else c["border2"], name="In the primary key")
        if pk:
            text(pk_, 3, -1, "✓", "#FFFFFF", size=12, weight="Bold")
    a = frame(body, 16, 50 + 3 * 40 + 8, 140, 30, fill="none", r=6, name="Add column")
    text(a, 10, 6, "+  Add column", c["accent"], size=12.5, weight="Medium")
    text(body, 16, 230, "Comment", c["muted"], size=12, weight="Medium")
    input_box(body, 16, 250, LW - 32, "Optional, saved with the table", c, placeholder=True)
    pending_panel(root, c, LW, [("ok", "Create table loyalty_tiers")], [
        [("kw", "CREATE TABLE"), ("ident", " public.loyalty_tiers (")],
        [("ident", "    id "), ("kw", "smallint NOT NULL"), ("punct", ",")],
        [("ident", "    name "), ("param", "?")],
        [("ident", "    … "), ("kw", "numeric"), ("ident", "("), ("num", "5"), ("ident", ","),
         ("num", "2"), ("ident", ") "), ("kw", "NOT NULL DEFAULT"), ("num", " 0"),
         ("punct", ",")],
        [("kw", "    PRIMARY KEY"), ("ident", " (id)")],
        [("punct", ");")]], "Create",
        errors=["Give name a type.", "Every column needs a name."])


SCREENS = [s_style, s_welcome, s_results, s_running, s_error, s_plan, s_txn_find, s_review,
           s_params, s_palette, s_history, s_connection,
           lambda x, y, c: s_connection(x, y, c, file_db=True), s_ssh_trust, s_table_editor,
           lambda x, y, c: s_table_editor(x, y, c, tab="DDL"), s_import, s_diff, s_diagram,
           s_settings, s_confirms, s_states, s_multi, s_edit_mode, s_conn_full, s_te_fks,
           s_te_indexes, s_new_table]

GAP, PER_ROW = 160, 4
which = sys.argv[2] if len(sys.argv) > 2 else "both"
first = True
for page, pal in [("Dark", DARK), ("Light", LIGHT)]:
    if which not in ("both", page.lower()):
        continue
    if first:
        add("page.rename", {"name": page})
        first = False
    else:
        add("page.add", {"name": page})
        add("page.select", {"index": 1})
    from ddkit import THEME
    THEME["light"] = pal is LIGHT
    for i, fn in enumerate(SCREENS):
        fn((i % PER_ROW) * (W + GAP), (i // PER_ROW) * (H + GAP), pal)

steps.append({"command": "file.save", "params": {"path": sys.argv[1]}})
for s in steps:
    print(json.dumps(s))
