use anyhow::{Context, Result};
use pdfium_auto::bind_pdfium_silent;
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const HEADER_MARGIN: f32 = 40.0;
const FOOTER_MARGIN: f32 = 40.0;
const MIN_COLUMN_LINES: usize = 6;
const WORD_GAP_EM: f32 = 0.15;
const GUTTER_GAP_EM: f32 = 1.5;
const FRONT_MATTER_PAGES: usize = 3;
const PAGE_NUMBER_BAND: f32 = 0.12;
const RUNNING_HEADER_BAND: f32 = 0.1;
const MAX_TEXT_ANGLE: f32 = 5.0;
const SOFT_HYPHEN: char = '\u{AD}';
const DEFAULT_MODEL_VERSION: &str = "1.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticDocument {
    pub version: String,
    pub metadata: DocumentMetadata,
    pub sections: Vec<Section>,
    pub blocks: Vec<Block>,
    pub assets: Vec<Asset>,
    pub page_map: Vec<PageMap>,
    pub diagnostics: Diagnostics,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocumentMetadata {
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub abstract_text: Option<String>,
    pub page_count: usize,
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub level: u8,
    pub block_ids: Vec<String>,
    pub page_start: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub id: String,
    pub kind: BlockKind,
    pub text: String,
    pub spans: Vec<InlineSpan>,
    pub page_start: usize,
    pub page_end: usize,
    pub bbox_refs: Vec<RegionRef>,
    pub confidence: f32,
    pub warnings: Vec<String>,
}

impl Block {
    fn kind_discriminant(&self) -> &'static str {
        match self.kind {
            BlockKind::Heading { .. } => "heading",
            BlockKind::Paragraph => "paragraph",
            BlockKind::List => "list",
            BlockKind::Quote => "quote",
            BlockKind::Code => "code",
            BlockKind::Figure { .. } => "figure",
            BlockKind::Table { .. } => "table",
            BlockKind::Equation => "equation",
            BlockKind::Footnote => "footnote",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum BlockKind {
    Heading { level: u8 },
    Paragraph,
    List,
    Quote,
    Code,
    Figure { asset_id: Option<String> },
    Table { asset_id: Option<String> },
    Equation,
    Footnote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineSpan {
    pub text: String,
    pub style: InlineStyle,
    pub source_page: usize,
    pub bbox_refs: Vec<RegionRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InlineStyle {
    Body,
    Heading,
    Emphasis,
    Strong,
    Monospace,
    Caption,
    Footnote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    pub kind: AssetKind,
    pub source_page: usize,
    pub bbox: BBox,
    pub caption: Option<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Figure,
    Table,
    Equation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageMap {
    pub page: usize,
    pub width: f32,
    pub height: f32,
    pub block_ids: Vec<String>,
    pub regions: Vec<PageRegion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageRegion {
    pub kind: RegionKind,
    pub bbox: BBox,
    pub block_id: Option<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionKind {
    Text,
    Heading,
    Figure,
    Table,
    Equation,
    Footnote,
    Unsupported,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RegionRef {
    pub page: usize,
    pub bbox: BBox,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BBox {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Diagnostics {
    pub warnings: Vec<String>,
    pub unsupported_regions: Vec<UnsupportedRegion>,
    pub stats: ParserStats,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ParserStats {
    pub metadata_consumed_lines: usize,
    pub duplicate_blocks_dropped: usize,
    pub heading_promotions: usize,
    pub heading_rejections: usize,
    pub paragraph_merges: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsupportedRegion {
    pub page: usize,
    pub bbox: BBox,
    pub reason: String,
}

#[derive(Debug, Clone)]
struct TextChar {
    ch: char,
    x0: f32,
    x1: f32,
    top: f32,
    bottom: f32,
    font_size: f32,
    bold: bool,
}

#[derive(Debug, Clone)]
struct Line {
    page: usize,
    text: String,
    top: f32,
    bottom: f32,
    font_size: f32,
    x0: f32,
    x1: f32,
    bold: bool,
    /// Number of gutter-separated segments on this line's row; >1 for columns and table cells.
    row_cells: usize,
}

#[derive(Debug, Clone)]
struct PageLayout {
    page: usize,
    width: f32,
    height: f32,
    lines: Vec<Line>,
    /// Bounds of text drawn at an angle (rotated tables, axis labels), which is not reflowed.
    rotated_text: Option<BBox>,
}

#[derive(Debug, Clone)]
struct BlockCandidate {
    block: Block,
    page_regions: Vec<PageRegion>,
    asset: Option<Asset>,
}

#[derive(Debug, Clone)]
struct ParagraphCandidate {
    lines: Vec<Line>,
    text: String,
    font_size: f32,
    x0: f32,
    x1: f32,
    continuation_confidence: f32,
}

#[derive(Debug, Default)]
struct FirstPagePass {
    title: Option<String>,
    authors: Vec<String>,
    abstract_text: Option<String>,
    consumed_lines: HashSet<String>,
}

pub fn init_pdfium(pdfium_path: Option<&Path>) -> Result<Pdfium> {
    if let Some(path) = pdfium_path {
        let library_path = if path.is_dir() {
            Pdfium::pdfium_platform_library_name_at_path(path)
        } else {
            path.to_path_buf()
        };
        let bindings = Pdfium::bind_to_library(library_path).context("Failed to bind to Pdfium library")?;
        return Ok(Pdfium::new(bindings));
    }

    for candidate in discover_pdfium_candidates() {
        if let Ok(bindings) = Pdfium::bind_to_library(&candidate) {
            return Ok(Pdfium::new(bindings));
        }
    }

    if let Ok(bindings) = Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./"))
        .or_else(|_| Pdfium::bind_to_system_library())
    {
        return Ok(Pdfium::new(bindings));
    }

    bind_pdfium_silent().context("Failed to bind to Pdfium library; automatic download also failed")
}

pub fn parse_pdf(
    pdf_path: &Path,
    pdfium_path: Option<&Path>,
    max_pages: Option<usize>,
) -> Result<SemanticDocument> {
    let pdfium = init_pdfium(pdfium_path)?;
    let document = pdfium
        .load_pdf_from_file(pdf_path, None)
        .with_context(|| format!("Failed to load PDF {}", pdf_path.display()))?;

    let mut semantic = extract_document(&document, max_pages)?;
    semantic.metadata.source_path = Some(pdf_path.display().to_string());

    Ok(semantic)
}

pub fn extract_document(document: &PdfDocument, max_pages: Option<usize>) -> Result<SemanticDocument> {
    let page_count = document.pages().len() as usize;
    let limit = max_pages.unwrap_or(page_count).min(page_count);
    let mut pages = Vec::new();

    for (page_index, page) in document.pages().iter().take(limit).enumerate() {
        pages.push(extract_page_layout(&page, page_index)?);
    }

    strip_running_headers(&mut pages);
    Ok(build_semantic_document(pages))
}

/// Removes header/footer lines that repeat (ignoring page numbers) across many pages.
fn strip_running_headers(pages: &mut [PageLayout]) {
    let min_pages = 3.max(pages.len() / 4);
    if pages.len() < min_pages {
        return;
    }

    let running_key = |line: &Line, height: f32| -> Option<String> {
        let in_band = line.top >= height * (1.0 - RUNNING_HEADER_BAND) || line.bottom <= height * RUNNING_HEADER_BAND;
        let key = line
            .text
            .chars()
            .filter(|ch| ch.is_alphabetic())
            .collect::<String>()
            .to_lowercase();
        (in_band && key.len() >= 3).then_some(key)
    };

    let mut page_counts: std::collections::HashMap<String, HashSet<usize>> = std::collections::HashMap::new();
    for page in pages.iter() {
        for line in &page.lines {
            if let Some(key) = running_key(line, page.height) {
                page_counts.entry(key).or_default().insert(page.page);
            }
        }
    }

    for page in pages.iter_mut() {
        let height = page.height;
        page.lines.retain(|line| {
            running_key(line, height)
                .map(|key| page_counts.get(&key).map_or(0, HashSet::len) < min_pages)
                .unwrap_or(true)
        });
    }
}

pub fn render_text(document: &SemanticDocument) -> String {
    let mut buffer = String::new();

    if let Some(title) = &document.metadata.title {
        buffer.push_str(title);
        buffer.push_str("\n\n");
    }

    for section in &document.sections {
        buffer.push_str(&section.title);
        buffer.push('\n');

        for block_id in &section.block_ids {
            if let Some(block) = document.blocks.iter().find(|candidate| &candidate.id == block_id) {
                if matches!(block.kind, BlockKind::Heading { .. }) && block.text == section.title {
                    continue;
                }
                buffer.push_str(&block.text);
                buffer.push_str("\n\n");
            }
        }
    }

    if document.sections.is_empty() {
        for block in &document.blocks {
            buffer.push_str(&block.text);
            buffer.push_str("\n\n");
        }
    }

    buffer
}

fn extract_page_layout(page: &PdfPage, page_index: usize) -> Result<PageLayout> {
    let (chars, rotated_text) = extract_chars(page)?;
    let height = page.height().value;
    let width = page.width().value;
    let mut lines = chars_to_lines(chars, page_index);
    lines.retain(|line| !is_header_footer(line, height));

    Ok(PageLayout {
        page: page_index + 1,
        width,
        height,
        lines: reorder_lines(lines, width),
        rotated_text,
    })
}

fn extract_chars(page: &PdfPage) -> Result<(Vec<TextChar>, Option<BBox>)> {
    let text = page.text()?;
    let mut chars: Vec<TextChar> = Vec::new();
    let mut rotated_text: Option<BBox> = None;

    for char in text.chars().iter() {
        let Some(ch) = char.unicode_char() else {
            continue;
        };
        // Pdfium reports line-end hyphenation as U+0002; keep it as a soft hyphen so
        // paragraph assembly can rejoin the word.
        let ch = if ch == '\u{2}' { SOFT_HYPHEN } else { ch };
        if ch.is_control() {
            continue;
        }

        if ch.is_whitespace() {
            // Generated spaces carry zero-area bounds on the baseline, which would put them on
            // a separate line. Anchor them to the preceding glyph so they stay in its line.
            if let Some(previous) = chars.last().filter(|previous| !previous.ch.is_whitespace()) {
                chars.push(TextChar {
                    ch: ' ',
                    x0: previous.x1,
                    x1: previous.x1,
                    ..previous.clone()
                });
            }
            continue;
        }

        let bounds = char.loose_bounds()?;
        let angle = char.angle_degrees().unwrap_or(0.0).rem_euclid(360.0);
        if angle > MAX_TEXT_ANGLE && angle < 360.0 - MAX_TEXT_ANGLE {
            let char_box = BBox {
                x0: bounds.left().value,
                y0: bounds.bottom().value,
                x1: bounds.right().value,
                y1: bounds.top().value,
            };
            rotated_text = Some(rotated_text.map_or(char_box, |region| union_bbox(region, char_box)));
            continue;
        }
        let top = bounds.top().value;
        let bottom = bounds.bottom().value;
        let mut font_size = char.scaled_font_size().value;
        if font_size <= 1.0 {
            font_size = (top - bottom).max(1.0);
        }
        chars.push(TextChar {
            ch,
            x0: bounds.left().value,
            x1: bounds.right().value,
            top,
            bottom,
            font_size,
            bold: is_bold_font(&char),
        });
    }

    Ok((chars, rotated_text))
}

fn is_bold_font(char: &PdfPageTextChar) -> bool {
    let heavy = char.font_weight().is_some_and(|weight| {
        !matches!(
            weight,
            PdfFontWeight::Weight100
                | PdfFontWeight::Weight200
                | PdfFontWeight::Weight300
                | PdfFontWeight::Weight400Normal
                | PdfFontWeight::Weight500
        ) && !matches!(weight, PdfFontWeight::Custom(value) if value < 600)
    });
    if heavy {
        return true;
    }

    // Many embedded fonts (notably TeX's CMBX) report a normal weight; fall back to the name.
    let name = char.font_name().to_ascii_lowercase();
    ["bold", "black", "heavy", "semibold", "demi", "cmbx", "-bd", "_bd"]
        .iter()
        .any(|marker| name.contains(marker))
}

fn union_bbox(a: BBox, b: BBox) -> BBox {
    BBox {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}

fn chars_to_lines(mut chars: Vec<TextChar>, page_index: usize) -> Vec<Line> {
    if chars.is_empty() {
        return Vec::new();
    }

    let line_tolerance = median_font_size(chars.iter().map(|c| c.font_size).collect()).unwrap_or(12.0) * 0.4;

    chars.sort_by(|a, b| {
        b.top
            .partial_cmp(&a.top)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.x0.partial_cmp(&b.x0).unwrap_or(Ordering::Equal))
    });

    let mut buckets: Vec<Vec<TextChar>> = Vec::new();

    for char in chars {
        if let Some(last) = buckets.last_mut() {
            let last_top = average_top(last);
            if (last_top - char.top).abs() <= line_tolerance {
                last.push(char);
                continue;
            }
        }
        buckets.push(vec![char]);
    }

    buckets
        .into_iter()
        .flat_map(|bucket| {
            let mut row = split_at_gutters(bucket)
                .into_iter()
                .filter_map(|segment| build_line(segment, page_index))
                .collect::<Vec<_>>();
            let cells = row.len();
            for line in &mut row {
                line.row_cells = cells;
            }
            row
        })
        .collect()
}

/// Splits a row of glyphs wherever the horizontal gap is far wider than a word space, so text
/// from side-by-side columns (or table cells) at the same height becomes separate lines.
fn split_at_gutters(mut chars: Vec<TextChar>) -> Vec<Vec<TextChar>> {
    chars.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap_or(Ordering::Equal));

    let mut segments: Vec<Vec<TextChar>> = Vec::new();
    let mut current: Vec<TextChar> = Vec::new();
    let mut last_glyph_x1: Option<f32> = None;

    for char in chars {
        if !char.ch.is_whitespace() {
            if let Some(x1) = last_glyph_x1 {
                if char.x0 - x1 > char.font_size * GUTTER_GAP_EM {
                    segments.push(std::mem::take(&mut current));
                }
            }
            last_glyph_x1 = Some(char.x1);
        }
        current.push(char);
    }

    segments.push(current);
    segments
}

fn build_line(mut chars: Vec<TextChar>, page_index: usize) -> Option<Line> {
    if chars.is_empty() {
        return None;
    }

    chars.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap_or(Ordering::Equal));

    let font_size = median_font_size(chars.iter().map(|c| c.font_size).collect()).unwrap_or(12.0);
    // Loose bounds of adjacent glyphs inside a word touch, so any visible gap is a word break.
    let spacing_threshold = (font_size * WORD_GAP_EM).max(0.45);
    let mut text = String::new();
    let mut last_x1: Option<f32> = None;

    for char in &chars {
        if char.ch.is_whitespace() {
            if !text.ends_with(' ') && !text.is_empty() {
                text.push(' ');
            }
            continue;
        }

        if let Some(prev_x1) = last_x1 {
            let gap = char.x0 - prev_x1;
            if gap > spacing_threshold && !text.ends_with(' ') {
                text.push(' ');
            }
        }

        text.push(char.ch);
        last_x1 = Some(char.x1);
    }

    let text = normalize_whitespace(&text);
    if text.is_empty() {
        return None;
    }

    let glyphs = chars.iter().filter(|c| !c.ch.is_whitespace());
    let glyph_count = glyphs.clone().count();
    let bold_count = glyphs.clone().filter(|c| c.bold).count();
    let bold = glyph_count > 0 && bold_count * 10 >= glyph_count * 8;
    let top = glyphs.clone().map(|c| c.top).fold(f32::MIN, f32::max);
    let bottom = glyphs.clone().map(|c| c.bottom).fold(f32::MAX, f32::min);
    let x0 = glyphs.clone().map(|c| c.x0).fold(f32::MAX, f32::min);
    let x1 = glyphs.map(|c| c.x1).fold(f32::MIN, f32::max);

    Some(Line {
        page: page_index + 1,
        text,
        top,
        bottom,
        font_size,
        x0,
        x1,
        bold,
        row_cells: 1,
    })
}

fn reorder_lines(mut lines: Vec<Line>, page_width: f32) -> Vec<Line> {
    lines.sort_by(top_then_left);

    let Some(split_x) = detect_column_split(&lines, page_width) else {
        return lines;
    };

    // Walk the page top-down in bands. Lines that straddle the gutter (titles, abstracts,
    // wide captions) close the current band; inside a band the left column precedes the right.
    let mut ordered = Vec::with_capacity(lines.len());
    let mut left = Vec::new();
    let mut right = Vec::new();

    for line in lines {
        if line.x1 <= split_x {
            left.push(line);
        } else if line.x0 >= split_x {
            right.push(line);
        } else {
            ordered.append(&mut left);
            ordered.append(&mut right);
            ordered.push(line);
        }
    }

    ordered.append(&mut left);
    ordered.append(&mut right);
    ordered
}

fn top_then_left(a: &Line, b: &Line) -> Ordering {
    b.top
        .partial_cmp(&a.top)
        .unwrap_or(Ordering::Equal)
        .then_with(|| a.x0.partial_cmp(&b.x0).unwrap_or(Ordering::Equal))
}

/// Finds a vertical gutter in the middle of the page that separates two populated columns.
/// Single-column pages have most lines crossing the middle, so they never produce a split.
fn detect_column_split(lines: &[Line], page_width: f32) -> Option<f32> {
    if lines.len() < MIN_COLUMN_LINES {
        return None;
    }

    let min_side = (MIN_COLUMN_LINES / 2).max(lines.len() / 5);
    let max_crossing = (lines.len() / 5).max(2);
    let mut best: Option<(usize, f32, f32)> = None;
    let start = (page_width * 0.3) as i32;
    let end = (page_width * 0.7) as i32;

    for x in (start..=end).step_by(2) {
        let x = x as f32;
        let crossing = lines.iter().filter(|line| line.x0 < x && line.x1 > x).count();
        if crossing > max_crossing {
            continue;
        }
        let left = lines.iter().filter(|line| line.x1 <= x).count();
        let right = lines.iter().filter(|line| line.x0 >= x).count();
        if left < min_side || right < min_side {
            continue;
        }
        let distance_from_center = (x - page_width / 2.0).abs();
        let better = match best {
            None => true,
            Some((best_crossing, best_distance, _)) => {
                crossing < best_crossing || (crossing == best_crossing && distance_from_center < best_distance)
            }
        };
        if better {
            best = Some((crossing, distance_from_center, x));
        }
    }

    best.map(|(_, _, x)| x)
}

fn build_semantic_document(pages: Vec<PageLayout>) -> SemanticDocument {
    let all_lines: Vec<Line> = pages.iter().flat_map(|page| page.lines.clone()).collect();
    let body_font_size = body_font_size(&all_lines);
    let first_page_pass = classify_front_matter(
        &pages.iter().take(FRONT_MATTER_PAGES).cloned().collect::<Vec<_>>(),
        body_font_size,
    );
    let title = first_page_pass
        .title
        .clone()
        .or_else(|| detect_title(&all_lines, body_font_size));
    let authors = if first_page_pass.authors.is_empty() {
        detect_authors(&all_lines, title.as_deref(), body_font_size)
    } else {
        first_page_pass.authors.clone()
    };
    let abstract_text = first_page_pass
        .abstract_text
        .clone()
        .or_else(|| detect_abstract(&pages, body_font_size));

    let mut blocks = Vec::new();
    let mut assets = Vec::new();
    let mut sections = Vec::new();
    let mut diagnostics = Diagnostics::default();
    let mut page_maps: Vec<PageMap> = pages
        .iter()
        .map(|page| PageMap {
            page: page.page,
            width: page.width,
            height: page.height,
            block_ids: Vec::new(),
            regions: Vec::new(),
        })
        .collect();

    let mut section_title = String::from("Front Matter");
    let mut section_level = 1;
    let mut section_block_ids = Vec::new();
    let mut section_page_start = 1;
    let mut block_counter = 0usize;
    let mut asset_counter = 0usize;
    let mut bibliography_seen = false;
    let mut last_block_fingerprint: Option<String> = None;
    diagnostics.stats.metadata_consumed_lines = first_page_pass.consumed_lines.len();
    let body_lines = pages
        .iter()
        .flat_map(|page| page.lines.iter())
        .filter(|line| !first_page_pass.consumed_lines.contains(&line_identity(line)))
        .cloned()
        .collect::<Vec<_>>();
    let (table_cells, body_lines): (Vec<Line>, Vec<Line>) =
        body_lines.into_iter().partition(is_table_cell);
    for (page, bbox) in cluster_table_regions(&table_cells, body_font_size) {
        diagnostics.unsupported_regions.push(UnsupportedRegion {
            page,
            bbox,
            reason: "table_like_region".to_string(),
        });
        diagnostics
            .warnings
            .push(format!("Page {} contains a table-like region rendered as fallback", page));
    }
    let paragraph_candidates = build_paragraph_candidates(
        &body_lines,
        body_font_size,
        &mut diagnostics,
    );

    for candidate in paragraph_candidates {
        let candidate_text = candidate.text.clone();
        if let Some(BlockKind::Heading { level }) = heading_kind_for_candidate(&candidate, body_font_size) {
            if !section_block_ids.is_empty() || section_title != "Front Matter" {
                sections.push(Section {
                    id: slugify(&section_title),
                    title: section_title.clone(),
                    level: section_level,
                    block_ids: section_block_ids.clone(),
                    page_start: section_page_start,
                });
                section_block_ids.clear();
            }

            section_title = candidate_text.clone();
            section_level = level;
            section_page_start = candidate.lines.first().map(|line| line.page).unwrap_or(1);
            bibliography_seen = is_bibliography_heading(&candidate_text);

            let block = block_from_lines(
                next_block_id(&mut block_counter, "block"),
                BlockKind::Heading { level },
                candidate.lines.clone(),
                InlineStyle::Heading,
                0.96,
                Vec::new(),
            );
            diagnostics.stats.heading_promotions += 1;
            attach_candidate(
                BlockCandidate {
                    page_regions: vec![region_for_lines(
                        &candidate.lines,
                        RegionKind::Heading,
                        0.96,
                        Some(block.id.clone()),
                    )],
                    asset: None,
                    block,
                },
                &mut blocks,
                &mut assets,
                &mut section_block_ids,
                &mut page_maps,
                &mut last_block_fingerprint,
                &mut diagnostics,
            );
            continue;
        }

        if is_figure_or_table_caption(&candidate_text) {
            let asset_id = next_block_id(&mut asset_counter, "asset");
            let kind = if candidate_text.to_ascii_lowercase().starts_with("table ") {
                AssetKind::Table
            } else {
                AssetKind::Figure
            };
            let block_kind = match kind {
                AssetKind::Figure => BlockKind::Figure {
                    asset_id: Some(asset_id.clone()),
                },
                AssetKind::Table => BlockKind::Table {
                    asset_id: Some(asset_id.clone()),
                },
                AssetKind::Equation => BlockKind::Equation,
            };
            let confidence = 0.78;
            let block = block_from_lines(
                next_block_id(&mut block_counter, "block"),
                block_kind,
                candidate.lines.clone(),
                InlineStyle::Caption,
                confidence,
                vec!["Caption promoted to asset placeholder".to_string()],
            );
            let asset = Asset {
                id: asset_id.clone(),
                kind,
                source_page: candidate.lines.first().map(|line| line.page).unwrap_or(1),
                bbox: bbox_for_lines(&candidate.lines),
                caption: Some(candidate_text.clone()),
                confidence,
            };
            diagnostics.warnings.push(format!(
                "Page {} caption promoted to {:?} placeholder; figure body extraction not implemented yet",
                asset.source_page, asset_id
            ));
            attach_candidate(
                BlockCandidate {
                    page_regions: vec![region_for_lines(
                        &candidate.lines,
                        region_kind_for_asset(&asset.kind),
                        confidence,
                        Some(asset_id.clone()),
                    )],
                    asset: Some(asset),
                    block,
                },
                &mut blocks,
                &mut assets,
                &mut section_block_ids,
                &mut page_maps,
                &mut last_block_fingerprint,
                &mut diagnostics,
            );
            continue;
        }

        if is_table_line(&candidate_text) {
            let page = candidate.lines.first().map(|line| line.page).unwrap_or(1);
            diagnostics.unsupported_regions.push(UnsupportedRegion {
                page,
                bbox: bbox_for_lines(&candidate.lines),
                reason: "table_like_region".to_string(),
            });
            diagnostics
                .warnings
                .push(format!("Page {} contains a table-like region rendered as fallback", page));
            continue;
        }

        let block_kind = classify_paragraph(&candidate.lines, &candidate.text, body_font_size, bibliography_seen);
        let mut warnings = Vec::new();
        let confidence = block_confidence(&candidate.lines, &block_kind, body_font_size, &mut warnings)
            * (0.85 + 0.15 * candidate.continuation_confidence.clamp(0.0, 1.0));
        let region_kind = match block_kind {
            BlockKind::Paragraph | BlockKind::List | BlockKind::Quote | BlockKind::Code => RegionKind::Text,
            BlockKind::Equation => RegionKind::Equation,
            BlockKind::Footnote => RegionKind::Footnote,
            _ => RegionKind::Text,
        };
        let block = block_from_lines(
            next_block_id(&mut block_counter, "block"),
            block_kind,
            candidate.lines.clone(),
            inline_style_for_lines(&candidate.lines, body_font_size),
            confidence.clamp(0.2, 0.99),
            warnings,
        );
        attach_candidate(
            BlockCandidate {
                page_regions: vec![region_for_lines(
                    &candidate.lines,
                    region_kind,
                    confidence.clamp(0.2, 0.99),
                    Some(block.id.clone()),
                )],
                asset: None,
                block,
            },
            &mut blocks,
            &mut assets,
            &mut section_block_ids,
            &mut page_maps,
            &mut last_block_fingerprint,
            &mut diagnostics,
        );
    }

    if !section_block_ids.is_empty() || section_title != "Front Matter" {
        sections.push(Section {
            id: slugify(&section_title),
            title: section_title.clone(),
            level: section_level,
            block_ids: section_block_ids,
            page_start: section_page_start,
        });
    }

    if sections.is_empty() && !blocks.is_empty() {
        sections.push(Section {
            id: "front-matter".to_string(),
            title: "Front Matter".to_string(),
            level: 1,
            block_ids: blocks.iter().map(|block| block.id.clone()).collect(),
            page_start: 1,
        });
    }

    for page in &pages {
        if let Some(bbox) = page.rotated_text {
            diagnostics.unsupported_regions.push(UnsupportedRegion {
                page: page.page,
                bbox,
                reason: "rotated_text".to_string(),
            });
        }
    }

    for unsupported in &diagnostics.unsupported_regions {
        if let Some(page_map) = page_maps.iter_mut().find(|page_map| page_map.page == unsupported.page) {
            page_map.regions.push(PageRegion {
                kind: RegionKind::Unsupported,
                bbox: unsupported.bbox,
                block_id: None,
                confidence: 0.2,
            });
        }
    }

    SemanticDocument {
        version: DEFAULT_MODEL_VERSION.to_string(),
        metadata: DocumentMetadata {
            title,
            authors,
            abstract_text,
            page_count: pages.len(),
            source_path: None,
        },
        sections,
        blocks,
        assets,
        page_map: page_maps,
        diagnostics,
    }
}

/// Character-weighted median font size, so short table/cover lines do not outvote body text.
fn body_font_size(lines: &[Line]) -> f32 {
    let mut weighted = lines
        .iter()
        .map(|line| (line.font_size, line.text.chars().count()))
        .collect::<Vec<_>>();
    weighted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
    let total = weighted.iter().map(|(_, count)| count).sum::<usize>();
    let mut seen = 0;
    for (font_size, count) in weighted {
        seen += count;
        if seen * 2 >= total {
            return font_size;
        }
    }
    12.0
}

fn attach_candidate(
    candidate: BlockCandidate,
    blocks: &mut Vec<Block>,
    assets: &mut Vec<Asset>,
    section_block_ids: &mut Vec<String>,
    page_maps: &mut [PageMap],
    last_block_fingerprint: &mut Option<String>,
    diagnostics: &mut Diagnostics,
) {
    let fingerprint = block_fingerprint(&candidate.block);
    if last_block_fingerprint.as_ref() == Some(&fingerprint)
        || blocks
            .last()
            .map(|previous| blocks_are_near_duplicates(previous, &candidate.block))
            .unwrap_or(false)
    {
        diagnostics.stats.duplicate_blocks_dropped += 1;
        return;
    }

    let block_id = candidate.block.id.clone();
    let page_start = candidate.block.page_start;

    if let Some(page_map) = page_maps.iter_mut().find(|map| map.page == page_start) {
        page_map.block_ids.push(block_id.clone());
        for region in &candidate.page_regions {
            page_map.regions.push(region.clone());
        }
    }

    section_block_ids.push(block_id);
    blocks.push(candidate.block);
    *last_block_fingerprint = Some(fingerprint);

    if let Some(asset) = candidate.asset {
        assets.push(asset);
    }
}

fn build_paragraph_candidates(
    lines: &[Line],
    body_font_size: f32,
    diagnostics: &mut Diagnostics,
) -> Vec<ParagraphCandidate> {
    let mut raw_candidates = Vec::new();
    let mut current: Vec<Line> = Vec::new();
    let line_pitch = typical_line_pitch(lines, body_font_size);
    let line_width = typical_line_width(lines, body_font_size);

    for line in lines {
        let text = normalize_whitespace(&line.text);
        if text.is_empty() {
            continue;
        }

        if let Some(previous) = current.last() {
            let block_right = current.iter().map(|line| line.x1).fold(f32::MIN, f32::max);
            let block_right = block_right.max(previous.x0 + line_width);
            // A wrapped numbered heading ends at its first short line.
            let heading_wrap_ended = current.len() <= 2
                && looks_like_numbered_heading(&current[0].text)
                && is_heading_line(&current[0], body_font_size)
                && previous.x1 < block_right - previous.font_size * 4.0;
            if heading_wrap_ended || should_flush_paragraph(previous, line, block_right, line_pitch, body_font_size) {
                if let Some(candidate) = paragraph_candidate_from_lines(std::mem::take(&mut current), body_font_size) {
                    raw_candidates.push(candidate);
                }
            }
        }

        current.push(line.clone());
    }

    if let Some(candidate) = paragraph_candidate_from_lines(current, body_font_size) {
        raw_candidates.push(candidate);
    }

    let merged = merge_paragraph_candidates(raw_candidates, body_font_size, diagnostics);
    rejoin_paragraphs_split_by_footnotes(merged, body_font_size, diagnostics)
}

/// A body paragraph that runs past the footnotes at the bottom of a page continues after them.
/// Pull the continuation back up so the paragraph reads as one block, with footnotes following.
fn rejoin_paragraphs_split_by_footnotes(
    candidates: Vec<ParagraphCandidate>,
    body_font_size: f32,
    diagnostics: &mut Diagnostics,
) -> Vec<ParagraphCandidate> {
    let mut result: Vec<ParagraphCandidate> = Vec::with_capacity(candidates.len());
    let mut open_paragraph: Option<usize> = None;

    for candidate in candidates {
        let is_body = same_font_size(candidate.font_size, body_font_size);
        let is_small = candidate.font_size < body_font_size * 0.9;

        if let Some(index) = open_paragraph {
            let starts_lowercase = candidate.text.chars().next().is_some_and(char::is_lowercase);
            if is_body && starts_lowercase && heading_kind_for_candidate(&candidate, body_font_size).is_none() {
                let target = &mut result[index];
                target.lines.extend(candidate.lines);
                target.text = lines_to_paragraph_text(&target.lines);
                target.x1 = target.x1.max(candidate.x1);
                diagnostics.stats.paragraph_merges += 1;
                open_paragraph = (!ends_sentence(&target.text)).then_some(index);
                continue;
            }
            if !is_small {
                open_paragraph = None;
            }
        }

        if is_body && !ends_sentence(&candidate.text) && heading_kind_for_candidate(&candidate, body_font_size).is_none() {
            open_paragraph = Some(result.len());
        }
        result.push(candidate);
    }

    result
}

fn paragraph_candidate_from_lines(lines: Vec<Line>, body_font_size: f32) -> Option<ParagraphCandidate> {
    if lines.is_empty() {
        return None;
    }

    let text = lines_to_paragraph_text(&lines);
    if text.is_empty() {
        return None;
    }

    let font_size = median_font_size(lines.iter().map(|line| line.font_size).collect()).unwrap_or(body_font_size);
    let x0 = lines.iter().map(|line| line.x0).fold(f32::MAX, f32::min);
    let x1 = lines.iter().map(|line| line.x1).fold(f32::MIN, f32::max);
    let continuation_confidence = paragraph_continuation_confidence(&lines, body_font_size);

    Some(ParagraphCandidate {
        lines,
        text,
        font_size,
        x0,
        x1,
        continuation_confidence,
    })
}

fn merge_paragraph_candidates(
    candidates: Vec<ParagraphCandidate>,
    body_font_size: f32,
    diagnostics: &mut Diagnostics,
) -> Vec<ParagraphCandidate> {
    let mut merged: Vec<ParagraphCandidate> = Vec::new();

    for candidate in candidates {
        if let Some(previous) = merged.last_mut() {
            if should_merge_candidates(previous, &candidate, body_font_size) {
                previous.lines.extend(candidate.lines.clone());
                previous.text = lines_to_paragraph_text(&previous.lines);
                previous.font_size = median_font_size(previous.lines.iter().map(|line| line.font_size).collect())
                    .unwrap_or(previous.font_size);
                previous.x0 = previous.x0.min(candidate.x0);
                previous.x1 = previous.x1.max(candidate.x1);
                previous.continuation_confidence =
                    previous.continuation_confidence.max(candidate.continuation_confidence);
                diagnostics.stats.paragraph_merges += 1;
                continue;
            }
        }

        merged.push(candidate);
    }

    merged
}

fn should_merge_candidates(previous: &ParagraphCandidate, current: &ParagraphCandidate, body_font_size: f32) -> bool {
    if previous.lines.last().map(|line| line.page) != current.lines.first().map(|line| line.page) {
        return false;
    }

    if heading_kind_for_candidate(previous, body_font_size).is_some()
        || heading_kind_for_candidate(current, body_font_size).is_some()
    {
        return false;
    }

    if is_figure_or_table_caption(&previous.text) || is_figure_or_table_caption(&current.text) {
        return false;
    }

    if !same_font_size(previous.font_size, current.font_size) {
        return false;
    }

    let prev_last = previous.lines.last().unwrap();
    let curr_first = current.lines.first().unwrap();
    let same_column = (previous.x0 - current.x0).abs() <= body_font_size * 3.0;
    let gap = prev_last.bottom - curr_first.top;
    let continuation = previous.text.ends_with('-')
        || previous
            .text
            .chars()
            .last()
            .map(|ch| !".:?!".contains(ch))
            .unwrap_or(false)
        || current
            .text
            .chars()
            .next()
            .map(|ch| ch.is_lowercase())
            .unwrap_or(false)
        || current.text.split_whitespace().count() <= 3;

    same_column && gap <= body_font_size * 2.2 && continuation
}

fn paragraph_continuation_confidence(lines: &[Line], body_font_size: f32) -> f32 {
    if lines.len() <= 1 {
        return 0.55;
    }

    let first = lines.first().unwrap();
    let last = lines.last().unwrap();
    let alignment_score = if (first.x0 - last.x0).abs() <= body_font_size * 2.0 { 0.2 } else { 0.0 };
    let punctuation_score = if lines_to_paragraph_text(lines)
        .chars()
        .last()
        .map(|ch| ".:?!".contains(ch))
        .unwrap_or(false)
    {
        0.2
    } else {
        0.0
    };

    let score: f32 = 0.45 + alignment_score + punctuation_score;
    score.clamp(0.0, 1.0)
}

fn heading_kind_for_candidate(candidate: &ParagraphCandidate, body_font_size: f32) -> Option<BlockKind> {
    if candidate.lines.len() > 2 {
        return None;
    }

    let first = candidate.lines.first()?;
    // A two-line heading must be visibly styled; otherwise it is a short numbered paragraph.
    let styled = |line: &Line| line.bold || line.font_size >= body_font_size * 1.15;
    let wrapped_numbered = looks_like_numbered_heading(&first.text)
        && candidate.text.split_whitespace().count() <= 20
        && !ends_statement(&candidate.text);
    if candidate.lines.len() == 2 && !candidate.lines.iter().all(styled) && !wrapped_numbered {
        return None;
    }
    if is_figure_or_table_caption(&candidate.text) {
        return None;
    }
    if is_heading_line(first, body_font_size) && !looks_like_metadata_line(&candidate.text) {
        return Some(BlockKind::Heading {
            level: heading_level(first, body_font_size),
        });
    }

    None
}

fn blocks_are_near_duplicates(previous: &Block, current: &Block) -> bool {
    if previous.page_start != current.page_start || previous.kind_discriminant() != current.kind_discriminant() {
        return false;
    }

    let prev = normalized_similarity_text(&previous.text);
    let curr = normalized_similarity_text(&current.text);
    prev == curr || overlap_ratio(&prev, &curr) > 0.92
}

fn normalized_similarity_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn overlap_ratio(a: &str, b: &str) -> f32 {
    let min_len = a.len().min(b.len());
    let max_len = a.len().max(b.len());
    if min_len == 0 || max_len == 0 {
        return 0.0;
    }

    let shared_prefix = a
        .chars()
        .zip(b.chars())
        .take_while(|(left, right)| left == right)
        .count();

    shared_prefix as f32 / max_len as f32
}

fn block_from_lines(
    id: String,
    kind: BlockKind,
    lines: Vec<Line>,
    style: InlineStyle,
    confidence: f32,
    warnings: Vec<String>,
) -> Block {
    let text = lines_to_paragraph_text(&lines);
    let bbox_refs = lines
        .iter()
        .map(|line| RegionRef {
            page: line.page,
            bbox: BBox {
                x0: line.x0,
                y0: line.bottom,
                x1: line.x1,
                y1: line.top,
            },
        })
        .collect::<Vec<_>>();
    let spans = vec![InlineSpan {
        text: text.clone(),
        style,
        source_page: lines.first().map(|line| line.page).unwrap_or(1),
        bbox_refs: bbox_refs.clone(),
    }];

    Block {
        id,
        kind,
        text,
        spans,
        page_start: lines.first().map(|line| line.page).unwrap_or(1),
        page_end: lines.last().map(|line| line.page).unwrap_or(1),
        bbox_refs,
        confidence,
        warnings,
    }
}

fn classify_paragraph(lines: &[Line], text: &str, body_font_size: f32, bibliography_seen: bool) -> BlockKind {
    if is_footnote_block(lines, body_font_size) {
        return BlockKind::Footnote;
    }
    if bibliography_seen || looks_like_reference_entry(text) {
        return BlockKind::Quote;
    }
    if looks_like_list(text) {
        return BlockKind::List;
    }
    if looks_like_code(lines, text, body_font_size) {
        return BlockKind::Code;
    }
    if looks_like_equation(text) {
        return BlockKind::Equation;
    }
    BlockKind::Paragraph
}

fn block_confidence(lines: &[Line], kind: &BlockKind, body_font_size: f32, warnings: &mut Vec<String>) -> f32 {
    let mut confidence: f32 = 0.88;
    let font_size = median_font_size(lines.iter().map(|line| line.font_size).collect()).unwrap_or(body_font_size);

    if lines.len() == 1 && matches!(kind, BlockKind::Paragraph) {
        confidence -= 0.1;
        warnings.push("single-line paragraph".to_string());
    }

    if matches!(kind, BlockKind::Code | BlockKind::Equation | BlockKind::Footnote) {
        confidence -= 0.08;
    }

    if font_size < body_font_size * 0.8 {
        confidence -= 0.08;
    }

    confidence.clamp(0.2, 0.99)
}

fn detect_title(lines: &[Line], body_font_size: f32) -> Option<String> {
    detect_title_lines(lines, &[1], body_font_size)
        .map(|(title, _)| title)
        .or_else(|| fallback_title(lines, body_font_size))
}

/// Returns the title text together with the lines it was assembled from, so callers can
/// remove every title line (not just single-line titles) from the body.
fn detect_title_lines(lines: &[Line], pages: &[usize], body_font_size: f32) -> Option<(String, Vec<Line>)> {
    // The earliest page with a plausible title wins; later pages only repeat it or start the body.
    pages.iter().enumerate().find_map(|(index, page)| {
        detect_title_on_page(lines, *page, body_font_size)
            .or_else(|| (index == 0).then(|| detect_caps_title(lines, *page)).flatten())
    })
}

/// Cover pages such as NBER's set the title in body-size capitals: take the first run of
/// all-caps lines near the top of the page.
fn detect_caps_title(lines: &[Line], page: usize) -> Option<(String, Vec<Line>)> {
    let mut page_lines = lines.iter().filter(|line| line.page == page).collect::<Vec<_>>();
    page_lines.sort_by(|a, b| top_then_left(a, b));

    let is_caps_line = |line: &Line| {
        let text = line.text.trim();
        is_all_caps(text) && text.split_whitespace().count() >= 2 && !looks_like_metadata_line(text)
    };
    let start = page_lines
        .iter()
        .take(6)
        .position(|line| is_caps_line(line) && is_probable_title_text(line.text.trim()))?;

    let mut title_lines = vec![page_lines[start].clone()];
    for line in &page_lines[start + 1..] {
        let previous = title_lines.last().unwrap();
        if !is_caps_line(line)
            || !same_font_size(line.font_size, previous.font_size)
            || previous.top - line.top > previous.font_size * 2.5
        {
            break;
        }
        title_lines.push((*line).clone());
    }

    let title = title_lines
        .iter()
        .map(|line| strip_note_markers(&normalize_whitespace(&line.text)))
        .collect::<Vec<_>>()
        .join(" ");
    Some((title, title_lines))
}

fn detect_title_on_page(lines: &[Line], page: usize, body_font_size: f32) -> Option<(String, Vec<Line>)> {
    let page_set = HashSet::from([page]);
    let mut opening_lines: Vec<&Line> = lines
        .iter()
        .filter(|line| page_set.contains(&line.page))
        .collect();
    opening_lines.sort_by(|a, b| a.page.cmp(&b.page).then_with(|| top_then_left(a, b)));

    let is_title_start = |line: &Line| {
        let text = line.text.trim();
        let word_count = text.split_whitespace().count();
        word_count >= 4
            && !(is_all_caps(text) && word_count <= 5)
            && !looks_like_numbered_heading(text)
            && is_probable_title_text(text)
    };

    let max_title_font = opening_lines
        .iter()
        .filter(|line| is_title_start(line))
        .map(|line| line.font_size)
        .fold(0.0, f32::max);
    // Titles are set larger than body text, or (e.g. NBER covers) in bold at body size.
    let bold_only = max_title_font < body_font_size * 1.1;
    let font_threshold = if bold_only {
        body_font_size * 0.95
    } else {
        (body_font_size * 1.1).max(max_title_font * 0.9)
    };
    let qualifies = |line: &Line| line.font_size >= font_threshold && (!bold_only || line.bold);

    let start = opening_lines
        .iter()
        .position(|line| qualifies(line) && is_title_start(line))?;
    let mut title_lines = vec![opening_lines[start].clone()];

    for line in &opening_lines[start + 1..] {
        let previous = title_lines.last().unwrap();
        let text = line.text.trim();
        let continues = line.page == previous.page
            && qualifies(line)
            && line.bold == previous.bold
            && previous.top - line.top <= previous.font_size * 2.5
            && !text.is_empty()
            && !looks_like_metadata_line(text);
        if !continues {
            break;
        }
        title_lines.push((*line).clone());
    }

    let title = title_lines
        .iter()
        .map(|line| strip_note_markers(&normalize_whitespace(&line.text)))
        .collect::<Vec<_>>()
        .join(" ");
    Some((title, title_lines))
}

fn strip_note_markers(text: &str) -> String {
    text.trim_end_matches(|ch: char| "*†‡§¶∗".contains(ch))
        .trim()
        .to_string()
}

fn detect_authors(lines: &[Line], title: Option<&str>, body_font_size: f32) -> Vec<String> {
    let title = title.unwrap_or_default();
    let mut author_candidates = Vec::new();

    for line in lines.iter().filter(|line| line.page == 1) {
        let text = normalize_whitespace(&line.text);
        if text.is_empty() || text == title || text.contains('@') {
            continue;
        }
        let word_count = text.split_whitespace().count();
        let looks_like_name_line =
            line.font_size >= body_font_size * 0.85 && line.font_size <= body_font_size * 1.2 && word_count <= 12;
        if looks_like_name_line && text.chars().all(|ch| ch.is_alphanumeric() || ",.-'& ".contains(ch)) {
            for candidate in text.split(',').map(str::trim).filter(|candidate| !candidate.is_empty()) {
                if !is_probable_author_name(candidate) {
                    continue;
                }
                author_candidates.push(candidate.to_string());
            }
        }
        if !author_candidates.is_empty() {
            break;
        }
    }

    author_candidates
}

fn detect_abstract(pages: &[PageLayout], body_font_size: f32) -> Option<String> {
    let first_page = pages.first()?;
    let mut collecting = false;
    let mut abstract_lines = Vec::new();

    for line in &first_page.lines {
        let text = normalize_whitespace(&line.text);
        if text.eq_ignore_ascii_case("abstract") {
            collecting = true;
            continue;
        }
        if collecting && is_heading_line(line, body_font_size) {
            break;
        }
        if collecting {
            abstract_lines.push(text);
        }
    }

    let joined = abstract_lines.join(" ").trim().to_string();
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

fn region_for_lines(lines: &[Line], kind: RegionKind, confidence: f32, block_id: Option<String>) -> PageRegion {
    PageRegion {
        kind,
        bbox: bbox_for_lines(lines),
        block_id,
        confidence,
    }
}

fn bbox_for_lines(lines: &[Line]) -> BBox {
    let x0 = lines.iter().map(|line| line.x0).fold(f32::MAX, f32::min);
    let y0 = lines.iter().map(|line| line.bottom).fold(f32::MAX, f32::min);
    let x1 = lines.iter().map(|line| line.x1).fold(f32::MIN, f32::max);
    let y1 = lines.iter().map(|line| line.top).fold(f32::MIN, f32::max);
    BBox { x0, y0, x1, y1 }
}

fn heading_level(line: &Line, body_font_size: f32) -> u8 {
    if line.font_size >= body_font_size * 1.6 {
        1
    } else if line.font_size >= body_font_size * 1.3 {
        2
    } else {
        3
    }
}

fn inline_style_for_lines(lines: &[Line], body_font_size: f32) -> InlineStyle {
    if is_footnote_block(lines, body_font_size) {
        InlineStyle::Footnote
    } else if looks_like_code(lines, &lines_to_paragraph_text(lines), body_font_size) {
        InlineStyle::Monospace
    } else {
        InlineStyle::Body
    }
}

fn is_header_footer(line: &Line, page_height: f32) -> bool {
    let in_top_band = line.top >= page_height - HEADER_MARGIN;
    let in_bottom_band = line.bottom <= FOOTER_MARGIN;
    let in_page_number_band =
        line.bottom <= page_height * PAGE_NUMBER_BAND || line.top >= page_height * (1.0 - PAGE_NUMBER_BAND);

    if in_page_number_band && is_page_number(line) {
        return true;
    }

    if line.page == 1 {
        return false;
    }

    if in_top_band {
        return line.text.len() < 80;
    }

    in_bottom_band && line.text.len() < 10
}

fn is_page_number(line: &Line) -> bool {
    let trimmed = line.text.trim().trim_matches(|ch: char| ch == '-' || ch == '–' || ch.is_whitespace());
    let lower = trimmed.to_ascii_lowercase();
    let lower = lower.strip_prefix("page ").unwrap_or(&lower);
    let first = lower.split(" of ").next().unwrap_or_default();

    !first.is_empty()
        && first.len() <= 6
        && (first.chars().all(|c| c.is_ascii_digit()) || first.chars().all(|c| "ivxlc".contains(c)))
}

fn is_heading_line(line: &Line, body_font_size: f32) -> bool {
    let text = line.text.trim();
    if text.is_empty() {
        return false;
    }

    let lowercase = text.to_ascii_lowercase();
    let heading_keywords = [
        "abstract",
        "introduction",
        "conclusion",
        "references",
        "acknowledgements",
        "acknowledgments",
        "appendix",
        "data",
        "methods",
        "results",
        "discussion",
    ];

    if heading_keywords.contains(&lowercase.as_str()) {
        return true;
    }

    let word_count = text.split_whitespace().count();
    let letters = text.chars().filter(|ch| ch.is_alphabetic()).count();
    let non_space = text.chars().filter(|ch| !ch.is_whitespace()).count();
    let longest_word = text
        .split_whitespace()
        .map(|word| word.chars().filter(|ch| ch.is_alphabetic()).count())
        .max()
        .unwrap_or(0);
    if word_count == 0
        || letters < 4
        || longest_word < 3
        || (letters as f32) < non_space as f32 * 0.6
        || word_count > 14
        || looks_like_metadata_line(text)
        || (is_probable_author_name(text) && !looks_like_numbered_heading(text))
        || text.contains('†')
        || text.contains('‡')
        || text.contains('*')
    {
        return false;
    }

    if looks_like_numbered_heading(text)
        && line.font_size >= body_font_size * 0.95
        && word_count <= 16
        && !ends_statement(text)
    {
        return true;
    }

    // Same-size bold lines are how most papers set section and subsection headings.
    if line.bold
        && line.font_size >= body_font_size * 0.95
        && word_count <= 12
        && !ends_statement(text)
        && text.chars().next().is_some_and(|ch| ch.is_uppercase() || ch.is_ascii_digit())
    {
        return true;
    }

    if line.font_size >= body_font_size * 1.35
        && word_count <= 10
        && is_heading_style_text(text)
        && !text.contains(',')
    {
        return true;
    }

    is_all_caps(text) && word_count <= 6 && !text.contains(',')
}

fn is_bibliography_heading(text: &str) -> bool {
    matches!(
        text.to_ascii_lowercase().as_str(),
        "references" | "bibliography" | "works cited"
    )
}

fn is_all_caps(text: &str) -> bool {
    let mut letters = 0;
    let mut uppercase = 0;

    for ch in text.chars() {
        if ch.is_alphabetic() {
            letters += 1;
            if ch.is_uppercase() {
                uppercase += 1;
            }
        }
    }

    letters > 0 && letters == uppercase
}

fn is_figure_or_table_caption(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    ["figure ", "fig. ", "table "].iter().any(|prefix| {
        lower.strip_prefix(prefix).is_some_and(|rest| {
            // Require a label like "3", "A.2" or "IV" so prose such as "Table salt is..." is not a caption.
            let label = rest.split_whitespace().next().unwrap_or_default();
            let label = label.trim_end_matches([':', '.', '|', '—', '-']);
            let roman = label.chars().all(|ch| "ivxlc".contains(ch));
            let numbered = label.chars().any(|ch| ch.is_ascii_digit())
                && label.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '.');
            !label.is_empty() && label.len() <= 6 && (roman || numbered)
        })
    })
}

/// A line that is one cell of a multi-cell row: three or more cells side by side, or a
/// label/value pair where one side is numeric. Two-column body text has two long, wordy cells.
fn is_table_cell(line: &Line) -> bool {
    match line.row_cells {
        0 | 1 => false,
        2 => is_numeric_cell(&line.text),
        _ => line.text.split_whitespace().count() <= 12 || is_numeric_cell(&line.text),
    }
}

fn is_numeric_cell(text: &str) -> bool {
    let tokens = text.split_whitespace().collect::<Vec<_>>();
    !tokens.is_empty()
        && tokens
            .iter()
            .filter(|token| token.chars().any(|ch| ch.is_ascii_digit()))
            .count()
            * 2
            >= tokens.len()
}

/// Groups table cells that sit close together on a page into rectangular fallback regions.
fn cluster_table_regions(cells: &[Line], body_font_size: f32) -> Vec<(usize, BBox)> {
    let mut sorted = cells.iter().collect::<Vec<_>>();
    sorted.sort_by(|a, b| a.page.cmp(&b.page).then_with(|| top_then_left(a, b)));

    let mut regions: Vec<(usize, BBox)> = Vec::new();
    for cell in sorted {
        let bbox = BBox {
            x0: cell.x0,
            y0: cell.bottom,
            x1: cell.x1,
            y1: cell.top,
        };
        match regions.last_mut() {
            Some((page, region)) if *page == cell.page && region.y0 - cell.top <= body_font_size * 3.0 => {
                *region = union_bbox(*region, bbox);
            }
            _ => regions.push((cell.page, bbox)),
        }
    }
    regions
}

fn is_table_line(text: &str) -> bool {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.len() < 4 {
        return false;
    }
    let numeric_tokens = tokens
        .iter()
        .filter(|token| token.chars().all(|ch| ch.is_ascii_digit() || ".-%()".contains(ch)))
        .count();

    numeric_tokens * 2 >= tokens.len()
}

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn lines_to_paragraph_text(lines: &[Line]) -> String {
    let mut paragraph = String::new();
    let mut last_normalized_line: Option<String> = None;
    for line in lines {
        let normalized = normalize_whitespace(&line.text);
        if normalized.is_empty() {
            continue;
        }
        if last_normalized_line.as_ref() == Some(&normalized) {
            continue;
        }
        append_line_text(&mut paragraph, &normalized);
        last_normalized_line = Some(normalized);
    }
    // A soft hyphen that was not consumed by a line join is a real hyphen at a block boundary.
    paragraph.trim().replace(SOFT_HYPHEN, "-")
}

fn append_line_text(paragraph: &mut String, line_text: &str) {
    if paragraph.is_empty() {
        paragraph.push_str(line_text);
        return;
    }

    if paragraph.ends_with(SOFT_HYPHEN) {
        paragraph.pop();
        // Keep the hyphen in compounds like "Low-Skilled"; drop it inside a split word.
        if !line_text.chars().next().is_some_and(char::is_lowercase) {
            paragraph.push('-');
        }
        paragraph.push_str(line_text);
        return;
    }

    if paragraph.ends_with('-')
        && line_text
            .chars()
            .next()
            .map(|ch| ch.is_lowercase())
            .unwrap_or(false)
    {
        paragraph.pop();
        paragraph.push_str(line_text);
        return;
    }

    paragraph.push(' ');
    paragraph.push_str(line_text);
}

/// Median vertical distance between consecutive body-text lines, used as the baseline for
/// deciding whether a larger gap marks a paragraph break (robust to double spacing).
fn typical_line_pitch(lines: &[Line], body_font_size: f32) -> f32 {
    let pitches = lines
        .windows(2)
        .filter(|pair| {
            let (previous, current) = (&pair[0], &pair[1]);
            previous.page == current.page
                && same_font_size(previous.font_size, body_font_size)
                && same_font_size(current.font_size, body_font_size)
                && previous.x0 < current.x1
                && current.x0 < previous.x1
        })
        .map(|pair| pair[0].top - pair[1].top)
        .filter(|pitch| *pitch > 0.0 && *pitch < body_font_size * 4.0)
        .collect::<Vec<_>>();

    median_font_size(pitches).unwrap_or(body_font_size * 1.2)
}

/// Median width of body-text lines; a line well short of it is the end of a paragraph or a heading.
fn typical_line_width(lines: &[Line], body_font_size: f32) -> f32 {
    let widths = lines
        .iter()
        .filter(|line| same_font_size(line.font_size, body_font_size))
        .map(|line| line.x1 - line.x0)
        .collect::<Vec<_>>();
    median_font_size(widths).unwrap_or(0.0)
}

fn same_font_size(a: f32, b: f32) -> bool {
    (a - b).abs() <= a.max(b) * 0.12
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end_matches(|ch: char| ch.is_ascii_digit() || "*†‡∗)\"”'’".contains(ch))
        .ends_with(['.', ':', '?', '!'])
}

/// Like `ends_sentence`, but questions are allowed since headings are often phrased as one.
fn ends_statement(text: &str) -> bool {
    ends_sentence(text) && !text.trim_end().ends_with(['?', '!'])
}

fn should_flush_paragraph(
    previous: &Line,
    current: &Line,
    block_right: f32,
    line_pitch: f32,
    body_font_size: f32,
) -> bool {
    let em = previous.font_size.max(current.font_size).max(1.0);
    let previous_text = previous.text.trim();
    let block_right = block_right.max(current.x1);

    // Page turns and column jumps: continue only when the sentence is visibly unfinished.
    if !same_font_size(previous.font_size, current.font_size) {
        return true;
    }

    // Page turns and column jumps: continue only when the sentence is visibly unfinished.
    if previous.page != current.page || current.top > previous.top + em * 0.5 {
        return ends_sentence(previous_text) || is_heading_line(current, body_font_size);
    }

    if (previous.x0 - current.x0).abs() > em * 8.0 {
        return true;
    }

    let pitch = previous.top - current.top;
    let scale = (current.font_size / body_font_size).max(0.5);
    if pitch > line_pitch * scale * 1.35 {
        return true;
    }

    if is_heading_line(previous, body_font_size) && !ends_statement(previous_text) && previous.x1 < block_right - em * 4.0 {
        return true;
    }

    // A short heading line starts a new block when the preceding text visibly ended.
    if is_heading_line(current, body_font_size)
        && current.x1 < block_right - em * 4.0
        && (ends_sentence(previous_text) || pitch > line_pitch * scale * 1.1 || current.bold != previous.bold)
    {
        return true;
    }

    // First-line indent after a line that ended a sentence.
    let indent = current.x0 - previous.x0;
    if indent > em * 0.8 && indent < em * 4.0 && ends_sentence(previous_text) {
        return true;
    }

    // Short line that ends a sentence closes the paragraph.
    ends_sentence(previous_text) && previous.x1 < block_right - em * 2.0
}

fn average_top(chars: &[TextChar]) -> f32 {
    let total: f32 = chars.iter().map(|c| c.top).sum();
    total / chars.len() as f32
}

fn median_font_size(mut values: Vec<f32>) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let mid = values.len() / 2;
    if values.len() % 2 == 0 {
        Some((values[mid - 1] + values[mid]) / 2.0)
    } else {
        Some(values[mid])
    }
}

fn looks_like_list(text: &str) -> bool {
    let trimmed = text.trim();
    ["- ", "* ", "• ", "◦ ", "▪ ", "– ", "● "].iter().any(|bullet| trimmed.starts_with(bullet))
        || trimmed
            .split_once('.')
            .map(|(prefix, _)| prefix.chars().all(|ch| ch.is_ascii_digit()))
            .unwrap_or(false)
}

fn looks_like_reference_entry(text: &str) -> bool {
    text.contains("doi")
        || text.contains("Proceedings")
        || text.contains("arXiv")
        || text.contains(" et al.")
        || text.starts_with('[')
}

fn is_probable_title_text(text: &str) -> bool {
    let lowercase = text.to_ascii_lowercase();
    if lowercase.contains("available from the author")
        || lowercase.contains("working paper")
        || lowercase.starts_with("copyright")
        || lowercase.starts_with("seminar")
    {
        return false;
    }

    let alphabetic_count = text.chars().filter(|ch| ch.is_alphabetic()).count();
    let word_count = text.split_whitespace().count();

    alphabetic_count > 12 && word_count >= 4
}

fn fallback_title(lines: &[Line], body_font_size: f32) -> Option<String> {
    lines.iter()
        .filter(|line| line.page <= 2)
        .filter(|line| line.font_size >= body_font_size * 1.25)
        .map(|line| normalize_whitespace(&line.text))
        .find(|text| is_probable_title_text(text))
}

fn is_probable_author_name(text: &str) -> bool {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.len() < 2 || tokens.len() > 4 {
        return false;
    }

    tokens.iter().all(|token| {
        let cleaned = token.trim_matches(|ch: char| !ch.is_alphanumeric());
        let mut chars = cleaned.chars();
        match chars.next() {
            Some(first) if first.is_uppercase() => chars.all(|ch| ch.is_lowercase() || ch == '-' || ch == '\''),
            _ => false,
        }
    })
}

fn classify_front_matter(pages: &[PageLayout], body_font_size: f32) -> FirstPagePass {
    let mut pass = FirstPagePass::default();
    if pages.is_empty() {
        return pass;
    }

    let opening_lines = pages.iter().flat_map(|page| page.lines.iter()).collect::<Vec<_>>();
    let page_numbers = pages.iter().map(|page| page.page).collect::<Vec<_>>();
    let owned_lines = opening_lines.iter().map(|line| (*line).clone()).collect::<Vec<_>>();
    let title_result = detect_title_lines(&owned_lines, &page_numbers, body_font_size);

    let mut abstract_start_page = pages[0].page;
    if let Some((title, title_lines)) = &title_result {
        pass.title = Some(title.clone());
        let normalized_title = title.to_lowercase();
        for line in &opening_lines {
            let text = strip_note_markers(&normalize_whitespace(&line.text)).to_lowercase();
            let is_title_line = title_lines.iter().any(|title_line| line_identity(title_line) == line_identity(line));
            // Cover pages often repeat the title with a different line wrap.
            let repeats_title = text.split_whitespace().count() >= 2 && normalized_title.contains(&text);
            if is_title_line || repeats_title {
                pass.consumed_lines.insert(line_identity(line));
            }
        }

        let last_title_line = title_lines.last().unwrap();
        abstract_start_page = last_title_line.page;
        for line in opening_lines.iter().filter(|line| {
            line.page == last_title_line.page
                && line.top < last_title_line.top
                && last_title_line.top - line.top <= body_font_size * 10.0
                && is_probable_author_line(line, body_font_size)
        }) {
            for author in split_author_line(&line.text) {
                if !pass.authors.contains(&author) {
                    pass.authors.push(author);
                }
            }
            pass.consumed_lines.insert(line_identity(line));
        }
    }

    let mut abstract_lines = Vec::new();
    let mut abstract_font: Option<f32> = None;
    let mut collecting = false;
    for line in opening_lines.iter().filter(|line| line.page >= abstract_start_page) {
        let normalized = normalize_whitespace(&line.text);
        let lower = normalized.to_ascii_lowercase();
        if !collecting {
            if lower == "abstract" {
                collecting = true;
                pass.consumed_lines.insert(line_identity(line));
            } else if let Some(rest) = lower
                .strip_prefix("abstract")
                .filter(|rest| rest.starts_with([':', '.', ' ', '—', '-']))
            {
                collecting = true;
                abstract_font = Some(line.font_size);
                let offset = normalized.len() - rest.len();
                abstract_lines.push(normalized[offset..].trim_start_matches([':', '.', ' ', '—', '-']).to_string());
                pass.consumed_lines.insert(line_identity(line));
            }
            continue;
        }

        if is_heading_line(line, body_font_size)
            || lower.starts_with("jel")
            || lower.starts_with("keywords")
            || lower.starts_with("key words")
        {
            break;
        }
        // Footnotes interleaved with the abstract belong to the body, not the abstract.
        if abstract_font.is_some_and(|font| !same_font_size(font, line.font_size)) {
            continue;
        }
        abstract_font.get_or_insert(line.font_size);
        abstract_lines.push(normalized);
        pass.consumed_lines.insert(line_identity(line));
    }

    let abstract_text = join_text_lines(&abstract_lines);
    if !abstract_text.is_empty() {
        pass.abstract_text = Some(abstract_text);
    }

    pass
}

fn join_text_lines(lines: &[String]) -> String {
    let mut text = String::new();
    for line in lines {
        append_line_text(&mut text, line);
    }
    text.trim().replace(SOFT_HYPHEN, "-")
}

/// Splits an author line on commas, conjunctions and affiliation markers.
fn split_author_line(text: &str) -> Vec<String> {
    let normalized = normalize_whitespace(text)
        .replace(" and ", ",")
        .replace(" & ", ",")
        .replace(';', ",");
    normalized
        .split(|ch: char| ch == ',' || "*†‡§¶∗".contains(ch))
        .map(|part| part.trim().trim_end_matches(|ch: char| ch.is_ascii_digit()).trim())
        .filter(|part| is_probable_author_name(part))
        .map(str::to_string)
        .collect()
}

fn line_identity(line: &Line) -> String {
    format!("{}:{:.1}:{:.1}:{}", line.page, line.top, line.x0, normalize_whitespace(&line.text))
}

fn is_probable_author_line(line: &Line, body_font_size: f32) -> bool {
    let text = normalize_whitespace(&line.text);
    let word_count = text.split_whitespace().count();
    line.font_size >= body_font_size * 0.85
        && line.font_size <= body_font_size * 1.2
        && (2..=12).contains(&word_count)
        && (text.contains(',') || is_probable_author_name(&text))
        && !looks_like_metadata_line(&text)
}

fn looks_like_numbered_heading(text: &str) -> bool {
    let text = text.trim();
    let mut parts = text.split_whitespace();
    let raw_token = parts.next().unwrap_or_default();
    let token = raw_token.trim_end_matches('.');
    let rest = parts.collect::<Vec<_>>().join(" ");
    let arabic = token
        .split('.')
        .all(|part| !part.is_empty() && part.len() <= 2 && part.chars().all(|ch| ch.is_ascii_digit()));
    // "IV." — the trailing period keeps a sentence starting with "I" from matching.
    let roman = raw_token.ends_with('.') && token.len() <= 5 && token.chars().all(|ch| "IVXLC".contains(ch));
    // Appendix numbering such as "A.2" or "F.3.1".
    let appendix = token.split_once('.').is_some_and(|(letter, number)| {
        letter.len() == 1
            && letter.chars().all(|ch| ch.is_ascii_uppercase())
            && !number.is_empty()
            && number.split('.').all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
    });
    let token_is_numbering = !token.is_empty() && (arabic || roman || appendix);

    if !token_is_numbering {
        return false;
    }

    rest.chars().next().is_some_and(char::is_uppercase)
}

fn is_heading_style_text(text: &str) -> bool {
    let first = text.chars().next().unwrap_or_default();
    first.is_ascii_digit() || text.split_whitespace().all(|word| {
        let cleaned = word.trim_matches(|ch: char| !ch.is_alphanumeric());
        cleaned.chars().next().map(|ch| ch.is_uppercase()).unwrap_or(false)
    })
}

fn looks_like_metadata_line(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("jel")
        || lower.contains("version")
        || lower.contains("working paper")
        || lower.contains("classification")
        || lower.contains("department")
        || lower.contains("university")
        || lower.contains("mit")
        || lower.contains("november")
        || lower.contains("october")
}

fn looks_like_code(lines: &[Line], text: &str, body_font_size: f32) -> bool {
    let avg_font = median_font_size(lines.iter().map(|line| line.font_size).collect()).unwrap_or(body_font_size);
    avg_font <= body_font_size * 0.95
        && (text.contains("::") || text.contains("fn ") || text.contains('{') || text.contains(';'))
}

fn looks_like_equation(text: &str) -> bool {
    let symbols = ['=', '+', '∑', '∫', '≤', '≥'];
    text.chars().filter(|ch| symbols.contains(ch)).count() >= 2
}

fn is_footnote_block(lines: &[Line], body_font_size: f32) -> bool {
    let avg_font = median_font_size(lines.iter().map(|line| line.font_size).collect()).unwrap_or(body_font_size);
    let first_text = lines.first().map(|line| line.text.trim()).unwrap_or_default();
    avg_font <= body_font_size * 0.9
        && first_text
            .chars()
            .next()
            .map(|ch| ch.is_ascii_digit() || "*†‡§¶∗".contains(ch))
            .unwrap_or(false)
}

fn region_kind_for_asset(kind: &AssetKind) -> RegionKind {
    match kind {
        AssetKind::Figure => RegionKind::Figure,
        AssetKind::Table => RegionKind::Table,
        AssetKind::Equation => RegionKind::Equation,
    }
}

fn next_block_id(counter: &mut usize, prefix: &str) -> String {
    *counter += 1;
    format!("{prefix}-{:04}", *counter)
}

fn block_fingerprint(block: &Block) -> String {
    let text = block
        .text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let text = text.chars().take(240).collect::<String>();
    format!("{:?}:p{}:{}", block.kind_discriminant(), block.page_start, text)
}

fn slugify(text: &str) -> String {
    let mut slug = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

#[allow(dead_code)]
pub fn default_pdfium_path() -> Option<PathBuf> {
    let path = PathBuf::from("./");
    path.exists().then_some(path)
}

fn discover_pdfium_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(env_path) = std::env::var("PDFIUM_LIB_PATH") {
        candidates.push(PathBuf::from(env_path));
    }

    candidates.push(PathBuf::from("vendor/pdfium/lib/libpdfium.dylib"));

    if let Ok(executable) = std::env::current_exe() {
        for ancestor in executable.ancestors() {
            candidates.push(ancestor.join("vendor/pdfium/lib/libpdfium.dylib"));
        }
    }

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(page: usize, text: &str, x0: f32, x1: f32, top: f32, bottom: f32, font_size: f32) -> Line {
        Line {
            page,
            text: text.to_string(),
            x0,
            x1,
            top,
            bottom,
            font_size,
            bold: false,
            row_cells: 1,
        }
    }

    #[test]
    fn builds_semantic_document_with_sections_and_metadata() {
        let pages = vec![PageLayout {
            page: 1,
            width: 600.0,
            height: 800.0,
            rotated_text: None,
            lines: vec![
                line(1, "A Better PDF Reader for Papers", 80.0, 420.0, 760.0, 740.0, 22.0),
                line(1, "Jane Example, John Example", 120.0, 380.0, 720.0, 706.0, 12.0),
                line(1, "Abstract", 80.0, 150.0, 660.0, 648.0, 15.0),
                line(1, "This system reconstructs semantic reading order.", 80.0, 420.0, 636.0, 624.0, 11.0),
                line(1, "Introduction", 80.0, 180.0, 580.0, 566.0, 16.0),
                line(1, "PDFs are page-native and require reconstruction.", 80.0, 460.0, 552.0, 540.0, 11.0),
            ],
        }];

        let document = build_semantic_document(pages);

        assert_eq!(document.metadata.title.as_deref(), Some("A Better PDF Reader for Papers"));
        assert_eq!(document.metadata.authors.len(), 2);
        assert_eq!(document.sections.len(), 1);
        assert!(document.blocks.iter().any(|block| matches!(block.kind, BlockKind::Heading { .. })));
        assert!(document.metadata.abstract_text.as_deref().unwrap_or_default().contains("reconstructs"));
    }

    #[test]
    fn detects_two_column_reading_order() {
        let lines = vec![
            line(1, "Left one", 60.0, 220.0, 700.0, 688.0, 11.0),
            line(1, "Right one", 340.0, 520.0, 700.0, 688.0, 11.0),
            line(1, "Left two", 60.0, 220.0, 670.0, 658.0, 11.0),
            line(1, "Right two", 340.0, 520.0, 670.0, 658.0, 11.0),
            line(1, "Left three", 60.0, 220.0, 640.0, 628.0, 11.0),
            line(1, "Right three", 340.0, 520.0, 640.0, 628.0, 11.0),
        ];

        let reordered = reorder_lines(lines, 580.0);
        let texts = reordered.into_iter().map(|line| line.text).collect::<Vec<_>>();

        assert_eq!(
            texts,
            vec!["Left one", "Left two", "Left three", "Right one", "Right two", "Right three"]
        );
    }

    #[test]
    fn flags_table_like_regions_as_unsupported() {
        let pages = vec![PageLayout {
            page: 1,
            width: 600.0,
            height: 800.0,
            rotated_text: None,
            lines: vec![
                line(1, "Results", 80.0, 180.0, 760.0, 748.0, 15.0),
                line(1, "12 53 19 80", 80.0, 260.0, 700.0, 688.0, 11.0),
                line(1, "More discussion after the table.", 80.0, 320.0, 640.0, 628.0, 11.0),
            ],
        }];

        let document = build_semantic_document(pages);

        assert_eq!(document.diagnostics.unsupported_regions.len(), 1);
        assert!(document
            .diagnostics
            .warnings
            .iter()
            .any(|warning| warning.contains("table-like region")));
    }

    fn glyphs(text: &str, x0: f32, top: f32, advance: f32) -> Vec<TextChar> {
        text.chars()
            .enumerate()
            .map(|(index, ch)| TextChar {
                ch,
                x0: x0 + index as f32 * advance,
                x1: x0 + (index + 1) as f32 * advance,
                top,
                bottom: top - 12.0,
                font_size: 10.0,
                bold: false,
            })
            .collect()
    }

    #[test]
    fn inserts_word_spaces_from_glyph_gaps() {
        let mut chars = glyphs("We", 100.0, 700.0, 5.0);
        chars.extend(glyphs("develop", 113.0, 700.0, 5.0));

        let lines = chars_to_lines(chars, 0);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "We develop");
    }

    #[test]
    fn keeps_letter_digit_tokens_intact() {
        let lines = chars_to_lines(glyphs("21st D73 McDonald", 100.0, 700.0, 5.0), 0);

        assert_eq!(lines[0].text, "21st D73 McDonald");
    }

    #[test]
    fn splits_side_by_side_columns_into_separate_lines() {
        let mut chars = glyphs("left column text", 50.0, 700.0, 5.0);
        chars.extend(glyphs("right column text", 320.0, 700.0, 5.0));

        let lines = chars_to_lines(chars, 0);

        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|line| line.row_cells == 2));
    }

    #[test]
    fn joins_soft_hyphenated_words_but_keeps_compound_hyphens() {
        let lines = vec![
            line(1, &format!("electoral pres{SOFT_HYPHEN}"), 80.0, 500.0, 700.0, 688.0, 11.0),
            line(1, "sures on Low{SOFT_HYPHEN}".replace("{SOFT_HYPHEN}", &SOFT_HYPHEN.to_string()).as_str(), 80.0, 500.0, 686.0, 674.0, 11.0),
            line(1, "Skilled workers.", 80.0, 300.0, 672.0, 660.0, 11.0),
        ];

        assert_eq!(lines_to_paragraph_text(&lines), "electoral pressures on Low-Skilled workers.");
    }

    #[test]
    fn keeps_single_column_page_in_top_down_order() {
        // Full-width lines with short paragraph-final lines must not be read as two columns.
        let mut lines = Vec::new();
        for index in 0..12 {
            let top = 700.0 - index as f32 * 14.0;
            let x1 = if index % 4 == 3 { 180.0 } else { 530.0 };
            lines.push(line(1, &format!("line {index}"), 72.0, x1, top, top - 12.0, 11.0));
        }

        let texts = reorder_lines(lines, 612.0).into_iter().map(|line| line.text).collect::<Vec<_>>();

        assert_eq!(texts, (0..12).map(|index| format!("line {index}")).collect::<Vec<_>>());
    }

    #[test]
    fn keeps_full_width_title_above_two_columns() {
        let mut lines = vec![line(1, "A Full Width Title", 100.0, 500.0, 760.0, 740.0, 18.0)];
        for index in 0..4 {
            let top = 700.0 - index as f32 * 14.0;
            lines.push(line(1, &format!("left {index}"), 60.0, 290.0, top, top - 12.0, 11.0));
            lines.push(line(1, &format!("right {index}"), 320.0, 550.0, top, top - 12.0, 11.0));
        }

        let texts = reorder_lines(lines, 612.0).into_iter().map(|line| line.text).collect::<Vec<_>>();

        assert_eq!(texts[0], "A Full Width Title");
        assert_eq!(texts[1..5], ["left 0", "left 1", "left 2", "left 3"]);
        assert_eq!(texts[5..9], ["right 0", "right 1", "right 2", "right 3"]);
    }

    #[test]
    fn recognises_numbered_headings_but_not_addresses_or_sentences() {
        assert!(looks_like_numbered_heading("2.1 Assumptions"));
        assert!(looks_like_numbered_heading("IV. What is the Right Estimate?"));
        assert!(looks_like_numbered_heading("F.3 Multiple equilibria"));
        assert!(!looks_like_numbered_heading("1050 Massachusetts Avenue"));
        assert!(!looks_like_numbered_heading("I think this is right"));
        assert!(!looks_like_numbered_heading("7 n , , , t"));
    }

    #[test]
    fn heading_detection_rejects_numeric_fragments() {
        let body = 11.0;
        assert!(is_heading_line(&line(3, "I. Introduction", 72.0, 135.0, 720.0, 706.0, 11.0), body));
        assert!(!is_heading_line(&line(3, "09-006", 72.0, 135.0, 720.0, 706.0, 18.0), body));
        assert!(!is_heading_line(&line(3, "N 6", 72.0, 135.0, 720.0, 706.0, 11.0), body));
    }

    #[test]
    fn detects_page_numbers_and_captions() {
        assert!(is_page_number(&line(2, "- 12 -", 300.0, 320.0, 60.0, 50.0, 10.0)));
        assert!(is_page_number(&line(2, "Page 3 of 10", 300.0, 360.0, 60.0, 50.0, 10.0)));
        assert!(is_page_number(&line(2, "xiv", 300.0, 320.0, 60.0, 50.0, 10.0)));
        assert!(is_figure_or_table_caption("Table 7: The Effects of the Minimum Wage"));
        assert!(is_figure_or_table_caption("Figure A.2. Distribution of returns"));
        assert!(!is_figure_or_table_caption("Table salt is cheap"));
    }

    #[test]
    fn detects_capitalised_cover_title() {
        let lines = vec![
            line(1, "NBER WORKING PAPER SERIES", 200.0, 400.0, 720.0, 706.0, 12.0),
            line(1, "PRODUCTIVITY AND PAY IN THE US AND CANADA", 150.0, 460.0, 620.0, 606.0, 12.0),
            line(1, "Jacob Greenspon", 240.0, 360.0, 560.0, 546.0, 12.0),
        ];

        let (title, _) = detect_title_lines(&lines, &[1], 12.0).expect("title");

        assert_eq!(title, "PRODUCTIVITY AND PAY IN THE US AND CANADA");
    }

    #[test]
    fn splits_author_lines_on_affiliation_markers() {
        assert_eq!(split_author_line("Lakshmi Iyer† Anandi Mani‡"), vec!["Lakshmi Iyer", "Anandi Mani"]);
        assert_eq!(
            split_author_line("Jane Example, John Example and Ann Other"),
            vec!["Jane Example", "John Example", "Ann Other"]
        );
    }
}
