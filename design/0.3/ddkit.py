"""Building blocks for the DataDrop 0.3 mockups: emits FigCraft script steps."""
import json
import sys

steps = []
_n = [0]


def add(cmd, params):
    _n[0] += 1
    name = f"n{_n[0]}"
    steps.append({"command": cmd, "as": name, "params": params})
    return {"$ref": name}


SANS, MONO = "Inter", "Liberation Mono"
CW = 7.8  # Liberation Mono advance at 13 px

DARK = dict(
    bg="#0D1015", panel="#12161C", raised="#181D25", hover="#1E242E", border="#232A35",
    border2="#2C3441", text="#E6EAF0", muted="#8A94A4", faint="#5B6575",
    accent="#3B9EF5", accent_text="#FFFFFF", accent_soft="#3B9EF524", accent_line="#3B9EF5",
    ok="#3FB97A", warn="#E3A945", bad="#F06A6A", purple="#A78BFA", amber="#F0A45D",
    teal="#4FC1D6", pink="#F07C9A", sel_row="#3B9EF51F", zebra="#FFFFFF05",
    kw="#C68CF5", str="#9BD17B", num="#F0A45D", com="#6B7585", ident="#DDE1E6",
    punct="#9AA3B0", qid="#6CC4D8", param="#F07C9A", overlay="#05070AB3", shadow="#00000080",
)
LIGHT = dict(
    bg="#F6F7F9", panel="#FFFFFF", raised="#FFFFFF", hover="#EEF1F5", border="#E3E7ED",
    border2="#D5DBE3", text="#1A1F27", muted="#5F6877", faint="#97A0AD",
    accent="#1B6FC2", accent_text="#FFFFFF", accent_soft="#1B6FC21A", accent_line="#1B6FC2",
    ok="#1F9D5C", warn="#B7791F", bad="#D14343", purple="#7C4DDB", amber="#C2620A",
    teal="#0E7C93", pink="#B02452", sel_row="#1B6FC214", zebra="#0F172A05",
    kw="#8A2BC9", str="#237A1F", num="#B35600", com="#8A929E", ident="#1F2328",
    punct="#505866", qid="#0E6E86", param="#B02452", overlay="#0F172A66", shadow="#0F172A2E",
)


def frame(parent, x, y, w, h, fill=None, r=0, stroke=None, name="frame", clip=True, **props):
    p = {"fill": fill or "none", "cornerRadius": r, "clip": clip, **props}
    if stroke:
        p.update(stroke=stroke, strokeAlign="inside")
        if not any(k.startswith("stroke") and k.endswith("Weight") for k in props):
            p["strokeWeight"] = 1
    params = {"type": "frame", "x": x, "y": y, "width": w, "height": h, "name": name, "props": p,
              "select": False}
    if parent is not None:
        params["parent"] = parent
    return add("node.create", params)


def rect(parent, x, y, w, h, fill, r=0, stroke=None, name="rect", **props):
    p = {"fill": fill, "cornerRadius": r, **props}
    if stroke:
        p.update(stroke=stroke, strokeWeight=1, strokeAlign="inside")
    return add("node.create", {"type": "rectangle", "parent": parent, "x": x, "y": y, "width": w,
                               "height": h, "name": name, "props": p, "select": False})


def ellipse(parent, x, y, d, fill, name="dot"):
    return add("node.create", {"type": "ellipse", "parent": parent, "x": x, "y": y, "width": d,
                               "height": d, "name": name, "props": {"fill": fill}, "select": False})


def text(parent, x, y, s, color, size=13, weight="Regular", family=SANS, width=None, align=None,
         name=None, **props):
    p = {"fontFamily": family, "fontStyle": weight, "fontSize": size, "fill": color, **props}
    if width:
        p["width"] = width
        p.setdefault("textAutoResize", "height")
    if align:
        p["textAlign"] = align
    return add("node.create", {"type": "text", "parent": parent, "x": x, "y": y, "text": s,
                               "name": name or s[:24], "props": p, "select": False})


def shadow(color, y=8, blur=24):
    return [{"type": "dropShadow", "color": color, "offset": [0, y], "radius": blur,
             "visible": True}]


# --- components -------------------------------------------------------------------------------

def button(parent, x, y, label, c, kind="secondary", w=None, h=28, icon=None, hint=None):
    tw = len(label) * 7.0 + (18 if icon else 0) + (len(hint) * 6.6 + 10 if hint else 0)
    w = w or int(tw + 24)
    fill, stroke, fg = {
        "primary": (c["accent"], None, c["accent_text"]),
        "secondary": (c["raised"], c["border2"], c["text"]),
        "ghost": ("none", None, c["muted"]),
        "danger": (c["raised"], c["border2"], c["bad"]),
        "disabled": (c["raised"], c["border"], c["faint"]),
    }[kind]
    b = frame(parent, x, y, w, h, fill=fill, r=6, stroke=stroke, name=f"Button/{label}")
    tx = 12
    if icon:
        text(b, 11, (h - 14) / 2 - 1, icon, fg, size=12, weight="Semi Bold")
        tx = 28
    text(b, tx, (h - 16) / 2, label, fg, size=12.5, weight="Medium")
    if hint:
        hfg = "#FFFFFFB3" if kind == "primary" else c["faint"]
        text(b, w - 12 - len(hint) * 6.6, (h - 15) / 2, hint, hfg, size=11)
    return w


def pill(parent, x, y, label, fg, bg, h=20, size=11, weight="Medium", w=None):
    w = w or int(len(label) * (size * 0.6) + 14)
    p = frame(parent, x, y, w, h, fill=bg, r=h / 2, name=f"Pill/{label}")
    text(p, 7, (h - size - 3) / 2, label, fg, size=size, weight=weight)
    return w


def input_box(parent, x, y, w, value, c, placeholder=False, h=32, label=None, suffix=None,
              focus=False, mono=False):
    if label:
        text(parent, x, y - 20, label, c["muted"], size=12, weight="Medium")
    box = frame(parent, x, y, w, h, fill=c["bg"], r=6,
                stroke=c["accent"] if focus else c["border2"], name=f"Input/{label or value}")
    text(box, 11, (h - 17) / 2, value, c["faint"] if placeholder else c["text"], size=13,
         family=MONO if mono else SANS)
    if suffix:
        text(box, w - 11 - len(suffix) * 7.0, (h - 16) / 2, suffix, c["muted"], size=12)
    return box


def kind_icon(parent, x, y, kind, c):
    col = {"table": c["accent"], "view": c["purple"], "fn": c["amber"], "seq": c["teal"],
           "db": c["muted"], "schema": c["muted"], "idx": c["muted"]}[kind]
    if kind in ("db", "schema"):
        f = frame(parent, x, y, 14, 14, fill="none", r=3, stroke=col, name=f"Icon/{kind}")
        rect(f, 3, 4, 8, 1.5, col)
        rect(f, 3, 8, 8, 1.5, col)
        return
    f = frame(parent, x, y, 14, 14, fill=col + "26", r=3.5, name=f"Icon/{kind}")
    if kind == "table":
        rect(f, 3, 3.5, 8, 1.6, col)
        rect(f, 3, 6.5, 8, 1, col + "AA")
        rect(f, 3, 9, 8, 1, col + "AA")
    else:
        letter = {"view": "V", "fn": "ƒ", "seq": "#", "idx": "i"}[kind]
        text(f, 0, -0.5, letter, col, size=10, weight="Bold", width=14, align="center")


DRIVERS = {
    "Postgres": ("#5B8DEF", "Pg"), "MySQL": ("#E48E2C", "My"), "SQLite": ("#5FA8D3", "Sq"),
    "SQL Server": ("#E0525A", "Ms"), "ClickHouse": ("#E7C53A", "Ch"), "DuckDB": ("#F2C94C", "Dk"),
    "CockroachDB": ("#6E5BD8", "Cr"),
}


# Yellows that need more contrast on a light background.
DRIVERS_ON_LIGHT = {"ClickHouse": "#9A7B00", "DuckDB": "#9A7300", "SQLite": "#2F7FAF"}
THEME = {"light": False}


def driver_badge(parent, x, y, driver, size=18):
    col, ab = DRIVERS[driver]
    if THEME["light"]:
        col = DRIVERS_ON_LIGHT.get(driver, col)
    f = frame(parent, x, y, size, size, fill=col + "2E", r=size * 0.28, name=f"Driver/{driver}")
    text(f, 0, (size - size * 0.5 - 3) / 2, ab, col, size=size * 0.5, weight="Bold", width=size,
         align="center")


def vector(parent, x, y, w, h, path, color, weight=1.5, fill="none", name="icon"):
    return add("node.create", {"type": "vector", "parent": parent, "x": x, "y": y, "width": w,
                               "height": h, "path": path, "name": name, "select": False,
                               "props": {"fill": fill, "stroke": color, "strokeWeight": weight,
                                         "strokeCap": "round", "strokeJoin": "round"}})


def chevron(parent, x, y, color, open_=True):
    if open_:
        vector(parent, x, y + 4, 8, 5, "M0 0 L4 4 L8 0", color, name="Chevron down")
    else:
        vector(parent, x + 2, y + 2, 5, 8, "M0 0 L4 4 L0 8", color, name="Chevron right")


def search_icon(parent, x, y, color):
    vector(parent, x, y, 13, 13, "M5.5 0 A5.5 5.5 0 1 1 5.5 11 A5.5 5.5 0 1 1 5.5 0 M9.5 9.5 L13 13",
           color, name="Search")


def funnel_icon(parent, x, y, color):
    vector(parent, x, y, 11, 11, "M0 0 L11 0 L7 5 L7 10 L4 11 L4 5 Z", color, weight=1.3,
           name="Filter")


def lock_icon(parent, x, y, color):
    vector(parent, x + 2, y, 8, 6, "M0 6 L0 3.5 A4 3.5 0 0 1 8 3.5 L8 6", color, name="Shackle")
    rect(parent, x, y + 6, 12, 8, color, r=2, name="Lock body")


# --- shared chrome ----------------------------------------------------------------------------

W, H = 1440, 900
TOP, SIDE, STATUS = 48, 272, 26


def titlebar(root, c, txn="Auto", running=False, conn=("prod-db", "Postgres", "shop  ·  public")):
    bar = frame(root, 0, 0, W, TOP, fill=c["panel"], name="Title bar",
                strokeBottomWeight=1, stroke=c["border"])
    # Window controls space (macOS) and logo.
    for i, col in enumerate(["#FF5F57", "#FEBC2E", "#28C840"]):
        ellipse(bar, 16 + i * 20, 18, 12, col)
    drop = frame(bar, 84, 14, 20, 20, fill=c["accent"], r=6, name="Logo")
    ellipse(drop, 6, 7, 8, "#FFFFFF")
    text(bar, 112, 15, "DataDrop", c["text"], size=14, weight="Semi Bold")
    # The connection the active tab runs on (each tab has its own).
    text(bar, 206, 16, "Runs on", c["faint"], size=12)
    sw = frame(bar, 262, 9, 240, 30, fill=c["raised"], r=7, stroke=c["border2"],
               name="Tab connection")
    if conn[1]:
        driver_badge(sw, 8, 6, conn[1])
        text(sw, 34, 6, conn[0], c["text"], size=13, weight="Semi Bold")
    else:
        frame(sw, 8, 6, 18, 18, fill="none", r=5, stroke=c["border2"], dashes=[3, 2])
        text(sw, 34, 6, conn[0], c["muted"], size=13, weight="Medium")
    text(sw, 40 + len(conn[0]) * 7.6, 7, conn[2], c["muted"], size=12)
    ellipse(sw, 198, 12, 7, c["ok"] if conn[1] else c["faint"])
    chevron(sw, 220, 8, c["muted"])
    # The tab's actions: Run, Run all, Explain, Analyze, Stop.
    x = 516
    if running:
        x += button(bar, x, 10, "Running  4.2 s", c, "disabled", icon="◌", w=124) + 6
        for lbl in ("Run all", "Explain", "Analyze"):
            x += button(bar, x, 10, lbl, c, "disabled") + 6
        button(bar, x, 10, "Stop", c, "danger", icon="■")
    else:
        x += button(bar, x, 10, "Run", c, "primary", icon="▶", hint="⌘↵") + 6
        for lbl in ("Run all", "Explain", "Analyze"):
            x += button(bar, x, 10, lbl, c, "secondary") + 6
        button(bar, x, 10, "Stop", c, "disabled", icon="■")
    # Transaction mode (segmented).
    seg = frame(bar, 942, 10, 176, 28, fill=c["bg"], r=7, stroke=c["border"], name="Txn mode")
    on = 0 if txn == "Auto" else 100
    frame(seg, on + 2, 2, 98 if txn == "Auto" else 74, 24, fill=c["raised"], r=5,
          stroke=c["border2"])
    text(seg, 13, 6, "Auto-commit", c["text"] if txn == "Auto" else c["muted"], size=12,
         weight="Medium")
    text(seg, 114, 6, "Manual", c["text"] if txn != "Auto" else c["muted"], size=12,
         weight="Medium")
    # Command palette.
    pal = frame(bar, 1132, 10, 240, 28, fill=c["bg"], r=7, stroke=c["border"], name="Palette")
    search_icon(pal, 11, 7, c["faint"])
    text(pal, 32, 6, "Search tables, commands…", c["faint"], size=12.5)
    pill(pal, 202, 4, "⌘K", c["muted"], c["raised"], h=20)
    gear = frame(bar, 1384, 10, 40, 28, fill="none", r=7, name="Settings")
    text(gear, 13, 4, "⚙", c["muted"], size=16, family="Noto Sans Symbols 2")


def side_switch(sb, c, mode):
    seg = frame(sb, 12, 10, SIDE - 58, 28, fill=c["bg"], r=7, stroke=c["border"], name="Left view")
    half = (SIDE - 62) / 2
    on = 0 if mode == "explorer" else half
    frame(seg, on + 2, 2, half, 24, fill=c["raised"], r=5, stroke=c["border2"])
    for i, label in enumerate(["Explorer", "History"]):
        active = (i == 0) == (mode == "explorer")
        text(seg, 2 + i * half, 6, label, c["text"] if active else c["muted"], size=12,
             weight="Medium", width=half, align="center")
    add_b = frame(sb, SIDE - 40, 11, 26, 26, fill="none", r=6, name="Add connection")
    text(add_b, 7, 2, "+", c["muted"], size=17, weight="Medium")


HISTORY = [
    ("ok", "SELECT date_trunc('month', o.created_at) AS month, count(*) …", "prod-db", "Postgres",
     "now", "38 ms", "11 rows"),
    ("ok", "UPDATE orders SET top_region = 'EMEA · West' WHERE id = 4", "prod-db", "Postgres",
     "2 min", "4 ms", "1 row"),
    ("bad", "SELECT * FROM order_item WHERE order_id = :id", "prod-db", "Postgres", "6 min",
     "2 ms", "relation does not exist"),
    ("ok", "SELECT region, uniq(user_id) FROM events GROUP BY region", "analytics", "ClickHouse",
     "18 min", "112 ms", "6 rows"),
    ("ok", "ALTER TABLE customers ADD COLUMN marketing_opt_in boolean …", "prod-db", "Postgres",
     "1 h", "61 ms", "done"),
    ("warn", "SELECT count(*) FROM range(100000000000) WHERE hash(range) = 0", "local.duckdb",
     "DuckDB", "2 h", "9.1 s", "stopped"),
    ("ok", "EXPLAIN ANALYZE SELECT * FROM orders WHERE status = 'paid'", "prod-db", "Postgres",
     "Tue", "212 ms", "plan"),
    ("ok", "SELECT * FROM refunds ORDER BY created_at DESC", "prod-db", "Postgres", "Tue",
     "19 ms", "100 rows"),
]


def history_list(sb, c, y=84, selected=0):
    for i, (st, sql, conn, drv, when, took, res) in enumerate(HISTORY):
        r = frame(sb, 8, y, SIDE - 16, 68, fill=c["accent_soft"] if i == selected else "none",
                  r=8, name=f"History/{i}")
        dot = {"ok": c["ok"], "bad": c["bad"], "warn": c["warn"]}[st]
        ellipse(r, 10, 13, 7, dot)
        text(r, 24, 8, sql, c["text"], size=12, family=MONO, width=SIDE - 52, maxLines=2,
             textAutoResize="truncate", height=32)
        driver_badge(r, 24, 45, drv, size=14)
        text(r, 44, 45, f"{conn}  ·  {when}", c["faint"], size=11)
        res = f"{took}  ·  {res if st != 'bad' else 'error'}"
        text(r, SIDE - 28 - len(res) * 5.6, 45, res, c["bad"] if st == "bad" else c["muted"],
             size=11)
        y += 72


def sidebar(root, c, selected="orders", mode="explorer"):
    sb = frame(root, 0, TOP, SIDE, H - TOP - STATUS, fill=c["panel"], name="Explorer",
               strokeRightWeight=1, stroke=c["border"])
    side_switch(sb, c, mode)
    input_box(sb, 12, 46, SIDE - 24, "Search history…" if mode == "history" else "Filter objects…",
              c, placeholder=True, h=30)
    if mode == "history":
        history_list(sb, c)
        return sb

    rows = [
        (0, "conn", "prod-db", "Postgres", True, c["ok"]),
        (1, "db", "shop", None, True, None),
        (2, "group", "Tables", "12", True, None),
        (3, "table", "customers", "48k", False, None),
        (3, "table", "order_items", "1.2M", False, None),
        (3, "table", "orders", "312k", False, None),
        (3, "table", "products", "860", False, None),
        (3, "table", "refunds", "4.1k", False, None),
        (3, "table", "suppliers", "92", False, None),
        (2, "group", "Views", "3", False, None),
        (2, "group", "Functions", "5", False, None),
        (2, "group", "Sequences", "4", False, None),
        (0, "conn", "analytics", "ClickHouse", False, c["ok"]),
        (0, "conn", "local.duckdb", "DuckDB", False, c["faint"]),
        (0, "conn", "staging", "MySQL", False, c["faint"]),
        (0, "conn", "legacy-erp", "SQL Server", False, c["faint"]),
    ]
    y = 88
    for depth, kind, label, extra, open_, dot in rows:
        h = 28
        if kind == "conn" and depth == 0 and y > 90:
            y += 6
        sel = kind == "table" and label == selected
        row = frame(sb, 8, y, SIDE - 16, h, fill=c["accent_soft"] if sel else "none", r=6,
                    name=f"Row/{label}")
        x = 10 + depth * 16
        if kind in ("conn", "db", "group"):
            chevron(row, x, 7, c["faint"], open_)
        x += 14
        if kind == "conn":
            driver_badge(row, x, 5, extra)
            text(row, x + 26, 6, label, c["text"], size=13, weight="Semi Bold")
            ellipse(row, SIDE - 36, 11, 7, dot)
        elif kind == "group":
            text(row, x, 6, label, c["muted"], size=12.5, weight="Medium")
            text(row, SIDE - 40 - len(extra) * 7, 7, extra, c["faint"], size=11.5)
        else:
            kind_icon(row, x, 7, kind, c)
            text(row, x + 22, 6, label, c["accent"] if sel else c["text"], size=13,
                 weight="Medium" if sel else "Regular")
            if extra:
                text(row, SIDE - 36 - len(extra) * 6.6, 7, extra, c["faint"], size=11.5)
        y += h

    # Bottom: saved queries.
    rect(sb, 0, 610, SIDE, 1, c["border"])
    text(sb, 16, 626, "SAVED QUERIES", c["faint"], size=11, weight="Semi Bold", letterSpacing="6%")
    for i, (q, when) in enumerate([("orders_by_month.sql", "now"), ("churn_cohorts.sql", "2h"),
                                   ("refund_audit.sql", "Mon")]):
        r = frame(sb, 8, 650 + i * 30, SIDE - 16, 28, fill="none", r=6, name=f"Saved/{q}")
        text(r, 12, 6, "≡", c["faint"], size=13, family="Noto Sans Symbols 2")
        text(r, 32, 6, q, c["text"] if i == 0 else c["muted"], size=12.5)
        text(r, SIDE - 46, 7, when, c["faint"], size=11.5)
    return sb


def statusbar(root, c, right="Ln 9, Col 23   ·   UTF-8   ·   Spaces: 2", left_extra=None,
              info="PostgreSQL 17.2  ·  via SSH tunnel  ·  passwords in Keychain", state="ok"):
    st = frame(root, 0, H - STATUS, W, STATUS, fill=c["panel"], name="Status bar",
               strokeTopWeight=1, stroke=c["border"])
    label, dot = {"ok": ("Connected", c["ok"]), "none": ("No connection", c["faint"]),
                  "busy": ("Running", c["accent"]), "txn": ("Transaction open", c["warn"])}[state]
    ellipse(st, 14, 9, 8, dot)
    text(st, 28, 5, label, c["text"], size=12, weight="Medium")
    text(st, 36 + len(label) * 7.0, 5, info, c["muted"], size=12)
    if left_extra:
        text(st, 500, 5, left_extra, c["muted"], size=12)
    text(st, W - 16 - len(right) * 6.3, 5, right, c["muted"], size=12)


# --- SQL rendering ---------------------------------------------------------------------------

def sql_lines(parent, x, y, lines, c, lh=21, start=1, cursor=None):
    """lines: lists of (kind, text) tokens."""
    gutter_w = 44
    for i, toks in enumerate(lines):
        ly = y + i * lh
        n = start + i
        active = cursor and cursor[0] == n
        if active:
            rect(parent, 0, ly - 2, 2000, lh, c["hover"] + "80" if c is DARK else c["hover"])
        text(parent, x, ly, str(n).rjust(3), c["muted"] if active else c["faint"], size=12.5,
             family=MONO)
        col = 0
        for kind, s in toks:
            if s.strip():
                text(parent, x + gutter_w + col * CW, ly, s, c[kind], size=13, family=MONO,
                     **({"fontStyle": "Italic"} if kind == "com" and False else {}))
            col += len(s)
        if active:
            rect(parent, x + gutter_w + cursor[1] * CW, ly - 1, 2, 18, c["accent"])


QUERY = [
    [("com", "-- Revenue and order counts by month, paid orders only")],
    [("kw", "SELECT"), ("ident", " date_trunc("), ("str", "'month'"), ("punct", ","),
     ("ident", " o.created_at) "), ("kw", "AS"), ("ident", " month"), ("punct", ",")],
    [("ident", "       count"), ("punct", "(*)"), ("kw", " AS"), ("ident", " orders"),
     ("punct", ",")],
    [("ident", "       sum(o.total) "), ("kw", "AS"), ("ident", " revenue"), ("punct", ",")],
    [("ident", "       round(avg(o.total), "), ("num", "2"), ("ident", ") "), ("kw", "AS"),
     ("ident", " avg_order")],
    [("kw", "FROM"), ("ident", " orders o")],
    [("kw", "JOIN"), ("ident", " customers c "), ("kw", "ON"), ("ident", " c.id = o.customer_id")],
    [("kw", "WHERE"), ("ident", " o.status = "), ("str", "'paid'")],
    [("kw", "  AND"), ("ident", " o.created_at >= "), ("param", ":since")],
    [("kw", "GROUP BY"), ("ident", " "), ("num", "1")],
    [("kw", "ORDER BY"), ("ident", " "), ("num", "1"), ("kw", " DESC"), ("punct", ";")],
]

