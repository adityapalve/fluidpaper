#!/usr/bin/env python3
import json
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: parser_fixture_summary.py <semantic-json>", file=sys.stderr)
        return 2

    path = Path(sys.argv[1])
    data = json.loads(path.read_text())

    print(f"path: {path}")
    print(f"title: {data.get('metadata', {}).get('title')}")
    print(f"authors: {data.get('metadata', {}).get('authors')}")
    print(f"sections: {len(data.get('sections', []))}")
    print(f"blocks: {len(data.get('blocks', []))}")
    print(f"warnings: {len(data.get('diagnostics', {}).get('warnings', []))}")
    print(f"stats: {data.get('diagnostics', {}).get('stats', {})}")
    print("first_blocks:")

    for block in data.get("blocks", [])[:20]:
        kind = block.get("kind", {})
        text = block.get("text", "").replace("\n", " ")
        print(f"  - {block.get('id')}: {kind} :: {text[:160]}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
