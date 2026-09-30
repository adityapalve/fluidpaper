# Fixture Scaffolding

Add golden PDFs and expected semantic JSON outputs here.

Suggested baseline set:

- `single-column-paper.pdf`
- `two-column-paper.pdf`
- `figures-and-captions.pdf`
- `tables.pdf`
- `footnotes-and-references.pdf`
- `scanned-ocr-poor.pdf`

For each PDF, add a matching expected output file such as:

- `single-column-paper.semantic.json`

The Rust parser now emits versioned semantic JSON via:

```bash
cargo run -- path/to/paper.pdf --format semantic-json > fixtures/paper.semantic.json
```

Keep fixture names stable so parser regressions can be diffed cleanly over time.
