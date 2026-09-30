### FluidPaper

FluidPaper now has two layers:

- A Rust parser engine that converts PDFs into a versioned semantic document model.
- A native macOS reader prototype that renders that semantic model as a reflowed reading view with PDF fallback.

### Parser

The parser emits semantic JSON by default:

```bash
cargo run -- path/to/paper.pdf --format semantic-json
```

Debug text output is still available:

```bash
cargo run -- path/to/paper.pdf --format text
```

The semantic model includes:

- document metadata
- sections
- typed blocks with confidence and provenance
- assets for figures and tables
- page maps and unsupported regions for fallback rendering

### Corpus Check

Run the parser over a folder of papers and compare quality signals (fragmentation, missing word spaces, fallback regions) before and after parser changes:

```bash
python3 scripts/corpus_report.py ~/papers --out /tmp/fluidpaper-corpus
```

### Reader Prototype

The native macOS reader lives in [`reader/`](./reader) and builds with SwiftPM:

```bash
cd reader
swift build
```

You can launch it with either a semantic JSON file, a PDF file, or both:

```bash
open .build/debug/FluidPaperReader --args /path/to/paper.semantic.json /path/to/paper.pdf
```

### Current Gaps

This is still a baseline system. The current implementation adds:

- semantic block modeling
- first-pass two-column reading order
- heading, paragraph, list, quote, code, footnote, figure, table, and equation block typing
- parser diagnostics and unsupported-region fallback markers
- a native reader with typography controls and source-page preview

Still missing:

- real figure body extraction and page crops
- robust bibliography/footnote linking
- fixture corpus with actual PDFs
- OCR/scanned PDF handling
- voice reading integration on top of semantic blocks
