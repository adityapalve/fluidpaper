#!/usr/bin/env python3
"""Run the parser over a directory of PDFs and print per-paper quality signals.

usage: corpus_report.py <pdf-dir> [--out <dir>]

Signals worth watching across parser changes:
  1line%  share of paragraphs that are a single line (fragmentation)
  avgw    average words per paragraph
  glued   words of 18+ letters, which usually means missing word spaces
  unsup   table-like / rotated regions left to PDF fallback
"""
import argparse
import collections
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("pdf_dir", type=Path)
    parser.add_argument("--out", type=Path, help="keep semantic JSON outputs here")
    args = parser.parse_args()

    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    binary = ROOT / "target" / "debug" / "fluidpaper"
    out_dir = args.out or Path(tempfile.mkdtemp(prefix="fluidpaper-corpus-"))
    out_dir.mkdir(parents=True, exist_ok=True)

    header = f"{'paper':36} {'pages':>5} {'blocks':>6} {'heads':>5} {'1line%':>6} {'avgw':>5} {'glued':>5} {'unsup':>5}  title"
    print(header)
    failures = 0
    for pdf in sorted(args.pdf_dir.glob("*.pdf")):
        result = subprocess.run([binary, pdf], capture_output=True, text=True)
        if result.returncode != 0:
            failures += 1
            print(f"{pdf.stem[:36]:36} FAILED: {result.stderr.strip()[-120:]}")
            continue
        (out_dir / f"{pdf.stem}.semantic.json").write_text(result.stdout)
        doc = json.loads(result.stdout)

        blocks = doc["blocks"]
        kinds = collections.Counter(block["kind"]["type"] for block in blocks)
        paragraphs = [block for block in blocks if block["kind"]["type"] == "paragraph"]
        single_line = sum(1 for block in paragraphs if len(block["bbox_refs"]) == 1)
        avg_words = sum(len(block["text"].split()) for block in paragraphs) / max(1, len(paragraphs))
        words = " ".join(block["text"] for block in blocks).split()
        glued = sum(1 for word in words if len(re.sub(r"[^A-Za-z]", "", word)) >= 18)
        title = (doc["metadata"]["title"] or "-")[:50]
        print(
            f"{pdf.stem[:36]:36} {doc['metadata']['page_count']:>5} {len(blocks):>6} {kinds['heading']:>5}"
            f" {100 * single_line / max(1, len(paragraphs)):>6.0f} {avg_words:>5.0f} {glued:>5}"
            f" {len(doc['diagnostics']['unsupported_regions']):>5}  {title}"
        )

    print(f"\nJSON outputs: {out_dir}", file=sys.stderr)
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
