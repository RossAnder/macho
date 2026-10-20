#!/usr/bin/env python3
"""Renders the README screenshots from real macho output.

Builds the release binary, runs it, lays the ANSI output on a cell grid and draws
that as SVG in the eggshell palettes, then rasterises it with `rsvg-convert`.
Half blocks become exact rectangles rather than glyphs, so the logos have no seams
whatever the font's line metrics; everything else is set in IosevkaTerm.

Needs `cargo`, `fish`, `rsvg-convert` and the IosevkaTerm Nerd Font Mono font.

Charts docs/benchmarks.json too, when scripts/bench.sh has written it.

usage: scripts/screenshots.py [out_dir]     (default: assets)
"""

import json
import math
import os
import re
import shutil
import socket
import stat
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "release" / "macho"

FONT = "IosevkaTerm Nerd Font Mono"
FONT_SIZE = 16
CELL_W = FONT_SIZE // 2  # Iosevka's advance is exactly half an em
CELL_H = FONT_SIZE * 5 // 4  # and its line height 1.25 em
BASELINE = 15  # from the top of a cell
SCALE = 2  # rasterise at 2x for HiDPI screens

PAD_X, PAD_Y = 24, 18  # inside the window
MARGIN = 40  # around the window, room for the shadow
RADIUS = 12

# eggshell for Alacritty (eggshell/themes/alacritty/eggshell{,-light}.toml).
THEMES = {
    "dark": {
        "bg": "#0a0a0a",
        "fg": "#c8c8c8",
        "dim_fg": "#7a7a7a",
        "cursor": "#f2f2f2",
        "border": "#232323",
        "shadow": 0.45,
        "prompt": "#9e9e9e",
        "command": "#f2f2f2",
        "normal": ["#232323", "#e5715a", "#72bf89", "#f9cb92", "#5884f8", "#c8c8c8", "#c4d5ff", "#c8c8c8"],
        "bright": ["#5e5e5e", "#fb846c", "#77d695", "#fdd39c", "#c4d5ff", "#f2f2f2", "#c4d5ff", "#ffffff"],
        "dim": ["#1c1c1c", "#9a5143", "#4e7b5b", "#9a7a51", "#4e66a3", "#7a7a7a", "#5e73a4", "#9e9e9e"],
    },
    "light": {
        "bg": "#f1efed",
        "fg": "#302d2b",
        "dim_fg": "#6b6865",
        "cursor": "#171717",
        "border": "#e7e4e0",
        "shadow": 0.16,
        "prompt": "#4f4d49",
        "command": "#171717",
        "normal": ["#302d2b", "#8d2816", "#0d542b", "#9c721c", "#1a3bb3", "#4f4d49", "#092382", "#a19e9a"],
        "bright": ["#6b6865", "#8d2816", "#0d542b", "#9c721c", "#092382", "#4f4d49", "#092382", "#e7e4e0"],
        "dim": ["#4f4d49", "#b9827b", "#739f7c", "#a98e60", "#7e92be", "#a19e9a", "#7e92be", "#bab7b3"],
    },
}


@dataclass
class Cell:
    ch: str = " "
    fg: int | str | None = None  # palette index, a theme key, or None for the default
    bg: int | None = None
    bold: bool = False
    dim: bool = False


Grid = list[list[Cell]]

SGR = re.compile(r"\x1b\[([0-9;]*)m")


def parse_ansi(text: str) -> Grid:
    grid: Grid = []
    pen = Cell()
    for line in text.split("\n"):
        row: list[Cell] = []
        pos = 0
        for m in SGR.finditer(line):
            row += [Cell(ch, pen.fg, pen.bg, pen.bold, pen.dim) for ch in line[pos : m.start()]]
            pos = m.end()
            for code in [int(c) for c in m.group(1).split(";") if c] or [0]:
                if code == 0:
                    pen = Cell()
                elif code == 1:
                    pen.bold = True
                elif code == 2:
                    pen.dim = True
                elif 30 <= code <= 37:
                    pen.fg = code - 30
                elif 90 <= code <= 97:
                    pen.fg = code - 90 + 8
                elif code == 39:
                    pen.fg = None
                elif 40 <= code <= 47:
                    pen.bg = code - 40
                elif 100 <= code <= 107:
                    pen.bg = code - 100 + 8
                elif code == 49:
                    pen.bg = None
        row += [Cell(ch, pen.fg, pen.bg, pen.bold, pen.dim) for ch in line[pos:]]
        grid.append(row)
    return grid


def segments(*parts: tuple[str, str | None]) -> list[Cell]:
    """A row from (text, theme key) pairs, for lines macho does not print itself."""
    return [Cell(ch, key) for text, key in parts for ch in text]


def colour(theme: dict, c: int | str | None, dim: bool = False) -> str:
    if isinstance(c, str):
        return theme[c]
    if c is None:
        return theme["dim_fg" if dim else "fg"]
    if dim and c < 8:
        return theme["dim"][c]
    return theme["normal" if c < 8 else "bright"][c & 7]


def halves(cell: Cell, theme: dict) -> tuple[str | None, str | None]:
    """The fill of a cell's top and bottom half, if any."""
    fg = colour(theme, cell.fg)
    bg = None if cell.bg is None else colour(theme, cell.bg)
    match cell.ch:
        case "█":
            return fg, fg
        case "▀":
            return fg, bg
        case "▄":
            return bg, fg
        case _:
            return bg, bg


def window(inner_w: int, inner_h: int, theme: dict) -> list[str]:
    """An open SVG document holding an empty eggshell window; close it with </svg>."""
    win_w, win_h = inner_w + 2 * PAD_X, inner_h + 2 * PAD_Y
    width, height = win_w + 2 * MARGIN, win_h + 2 * MARGIN
    return [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}">',
        f'<style>text {{ font-family: "{FONT}"; font-size: {FONT_SIZE}px; }}</style>',
        '<defs><filter id="shadow" x="-20%" y="-20%" width="140%" height="140%">'
        f'<feDropShadow dx="0" dy="10" stdDeviation="14" flood-color="#000" '
        f'flood-opacity="{theme["shadow"]}"/></filter></defs>',
        f'<rect x="{MARGIN}" y="{MARGIN}" width="{win_w}" height="{win_h}" rx="{RADIUS}" '
        f'fill="{theme["bg"]}" filter="url(#shadow)"/>',
        f'<rect x="{MARGIN + 0.5}" y="{MARGIN + 0.5}" width="{win_w - 1}" height="{win_h - 1}" '
        f'rx="{RADIUS}" fill="none" stroke="{theme["border"]}"/>',
    ]


def escape(text: str) -> str:
    return text.replace("&", "&amp;").replace("<", "&lt;")


def svg(grid: Grid, theme: dict) -> str:
    cols = max((len(r) for r in grid), default=0)
    out = window(cols * CELL_W, len(grid) * CELL_H, theme)
    ox, oy = MARGIN + PAD_X, MARGIN + PAD_Y
    half = CELL_H // 2

    def run_rects(y: int, h: int, fills: list[str | None]) -> None:
        x = 0
        while x < len(fills):
            fill = fills[x]
            end = x + 1
            while end < len(fills) and fills[end] == fill:
                end += 1
            if fill:
                out.append(
                    f'<rect x="{ox + x * CELL_W}" y="{y}" width="{(end - x) * CELL_W}" height="{h}" fill="{fill}"/>'
                )
            x = end

    for r, row in enumerate(grid):
        top = oy + r * CELL_H
        pairs = [halves(c, theme) for c in row]
        run_rects(top, half, [p[0] for p in pairs])
        run_rects(top + half, CELL_H - half, [p[1] for p in pairs])
        # Box-drawing rules as Alacritty's builtin_box_drawing draws them: a crisp line.
        run_rects(
            top + half - 1,
            1,
            [colour(theme, c.fg, c.dim) if c.ch == "─" else None for c in row],
        )

        text_cells = [(x, c) for x, c in enumerate(row) if c.ch not in " █▀▄─"]
        i = 0
        while i < len(text_cells):
            x0, first = text_cells[i]
            style = (colour(theme, first.fg, first.dim), first.bold)
            # A run is adjacent cells of one style; each starts on its own grid column
            # (rsvg ignores per-glyph x lists) and Iosevka's fixed advance does the rest.
            j = i + 1
            while (
                j < len(text_cells)
                and text_cells[j][0] == x0 + (j - i)
                and (colour(theme, text_cells[j][1].fg, text_cells[j][1].dim), text_cells[j][1].bold) == style
            ):
                j += 1
            chars = escape("".join(c.ch for _, c in text_cells[i:j]))
            out.append(
                f'<text x="{ox + x0 * CELL_W}" y="{top + BASELINE}" fill="{style[0]}" '
                f'font-weight="{700 if style[1] else 500}">{chars}</text>'
            )
            i = j

    out.append("</svg>")
    return "\n".join(out)


def chart(bench: dict, theme: dict) -> str:
    """Mean time per tool as bars on a linear axis from zero, macho in the accent."""
    rows = bench["results"]
    label_w = max(len(r["name"]) for r in rows) * CELL_W
    bar_x, bar_w, value_w = label_w + 16, 560, 150
    pitch, bar_h, head = 30, 12, 66
    out = window(bar_x + bar_w + value_w, head + len(rows) * pitch - (pitch - bar_h) // 2, theme)
    ox, oy = MARGIN + PAD_X, MARGIN + PAD_Y

    cpu = re.sub(r"\s+\d+-Core Processor", "", bench["machine"].split(",")[0])
    out.append(f'<text x="{ox}" y="{oy + 14}" fill="{theme["fg"]}" font-weight="700">time to print one fetch</text>')
    out.append(
        f'<text x="{ox}" y="{oy + 36}" fill="{theme["dim_fg"]}" font-weight="500">'
        f"mean wall-clock, lower is better · {escape(cpu)} · {bench['date']}</text>"
    )

    scale = bar_w / max(r["mean_ms"] for r in rows)
    fastest = min(r["mean_ms"] for r in rows)
    for i, r in enumerate(rows):
        y = oy + head + i * pitch
        mid = y + bar_h // 2 + 5
        ours = r["name"] == "macho"
        weight = 700 if ours else 500
        fill = theme["normal"][4] if ours else theme["bright"][0]
        out.append(
            f'<text x="{ox + label_w}" y="{mid}" text-anchor="end" fill="{theme["fg"]}" '
            f'font-weight="{weight}">{escape(r["name"])}</text>'
        )
        # Square at the axis, rounded at the data end; never thinner than a visible sliver.
        w = max(r["mean_ms"] * scale, 3)
        rad = min(4, w / 2)
        x0 = ox + bar_x
        out.append(
            f'<path d="M{x0},{y} h{w - rad:.2f} a{rad},{rad} 0 0 1 {rad},{rad} v{bar_h - 2 * rad} '
            f'a{rad},{rad} 0 0 1 -{rad},{rad} h-{w - rad:.2f} z" fill="{fill}"/>'
        )
        value = f"{r['mean_ms']:.2f}" if r["mean_ms"] < 10 else f"{r['mean_ms']:.1f}"
        relative = r["mean_ms"] / fastest
        label = f"{value} ms"
        out.append(
            f'<text x="{x0 + w + 10:.2f}" y="{mid}" fill="{theme["fg"]}" font-weight="{weight}">{label}</text>'
        )
        out.append(
            f'<text x="{x0 + w + 10 + (len(label) + 2) * CELL_W:.2f}" y="{mid}" fill="{theme["dim_fg"]}" '
            f'font-weight="500">{relative:.{0 if relative >= 10 else 1}f}×</text>'
        )
    out.append(
        f'<rect x="{ox + bar_x - 1}" y="{oy + head - 6}" width="1" '
        f'height="{len(rows) * pitch}" fill="{theme["border"]}"/>'
    )
    out.append("</svg>")
    return "\n".join(out)


def macho(*args: str) -> str:
    """Runs macho from fish under a parent named `alacritty`, so the shell and
    terminal lines read as they do in an ordinary session rather than this script's."""
    with tempfile.TemporaryDirectory() as tmp:
        term = Path(tmp) / "alacritty"
        term.write_text('#!/bin/sh\nfish --no-config -c \'$argv\' -- "$@"\n')
        term.chmod(term.stat().st_mode | stat.S_IXUSR)
        env = {k: v for k, v in os.environ.items() if k != "NO_COLOR"}
        return subprocess.run([term, BIN, *args], env=env, capture_output=True, text=True, check=True).stdout


def prompt(command: str = "") -> list[Cell]:
    """fish's default prompt, as eggshell colours it."""
    cwd = str(ROOT).replace(os.environ.get("HOME", "\0"), "~", 1).split("/")
    cwd = "/".join([p[:1] for p in cwd[:-1]] + cwd[-1:])  # fish's prompt_pwd
    login = f"{os.environ.get('USER', 'user')}@{socket.gethostname().split('.')[0]} "
    row = segments((login, "prompt"), (cwd, "prompt"), ("> ", None), (command, "command"))
    if not command:
        row.append(Cell("█", "cursor"))
    return row


def session(*args: str) -> Grid:
    command = " ".join(["macho", *args])
    output = parse_ansi(macho(*args).rstrip("\n"))
    return [prompt(command), *output, [], prompt()]


def gallery() -> Grid:
    names = macho("--list-logos").split()
    arts = {}
    for name in names:
        lines = (ROOT / "logos" / f"{name}.txt").read_text().splitlines()
        art_w, art_h = max(map(len, lines)), math.ceil(len(lines) / 2)
        out = parse_ansi(macho("--logo", name).strip("\n"))
        top = (len(out) - art_h) // 2
        arts[name] = [row[2 : 2 + art_w] for row in out[top : top + art_h]]

    per_row = 4
    tile_w = max(len(a[0]) for a in arts.values()) + 6
    tile_h = max(len(a) for a in arts.values()) + 3
    grid: Grid = []
    for start in range(0, len(names), per_row):
        chunk = names[start : start + per_row]
        offset = (per_row - len(chunk)) * tile_w // 2  # centre a short last row
        band: Grid = [[Cell() for _ in range(per_row * tile_w)] for _ in range(tile_h)]
        for i, name in enumerate(chunk):
            art = arts[name]
            left = offset + i * tile_w
            x0 = left + (tile_w - len(art[0])) // 2
            y0 = tile_h - 3 - len(art)  # bottom-align so the labels share a baseline
            for y, row in enumerate(art):
                band[y0 + y][x0 : x0 + len(row)] = row
            lx = left + (tile_w - len(name)) // 2
            band[tile_h - 2][lx : lx + len(name)] = [Cell(ch, None, dim=True) for ch in name]
        grid += band
    return [[], *grid[:-1]]


def main() -> None:
    out_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "assets"
    out_dir.mkdir(parents=True, exist_ok=True)
    if not shutil.which("rsvg-convert"):
        sys.exit("screenshots: rsvg-convert not found (librsvg)")
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)

    scenes = {
        name: lambda theme, grid=grid: svg(grid, theme)
        for name, grid in [("macho", session()), ("egg", session("--logo", "egg")), ("logos", gallery())]
    }
    bench = ROOT / "docs" / "benchmarks.json"
    if bench.exists():  # written by scripts/bench.sh
        scenes["bench"] = lambda theme, data=json.loads(bench.read_text()): chart(data, theme)

    with tempfile.TemporaryDirectory() as tmp:
        for scene, draw in scenes.items():
            for variant, theme in THEMES.items():
                src = Path(tmp) / f"{scene}-{variant}.svg"
                src.write_text(draw(theme))
                dest = out_dir / f"{scene}-{variant}.png"
                subprocess.run(["rsvg-convert", "-z", str(SCALE), "-o", dest, src], check=True)
                print(dest.relative_to(ROOT) if dest.is_relative_to(ROOT) else dest)


if __name__ == "__main__":
    main()
