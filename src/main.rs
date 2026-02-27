use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use pdfium_render::prelude::*;
use serde::Serialize;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

const HEADER_MARGIN: f32 = 40.0;
const FOOTER_MARGIN: f32 = 40.0;
const FRONT_MATTER_TITLE: &str = "Front Matter";

#[derive(Parser)]
#[command(name = "voicereader", version, about = "Extracts structured text from PDFs")]
struct Args {
    pdf: PathBuf,
    #[arg(long, default_value = "json")]
    format: OutputFormat,
    #[arg(long)]
    max_pages: Option<usize>,
    #[arg(long)]
    pdfium_path: Option<PathBuf>,
}

#[derive(Clone, ValueEnum)]
enum OutputFormat {
    Json,
    Text,
}

#[derive(Serialize)]
struct DocumentOutput {
    title: Option<String>,
    sections: Vec<Section>,
}

#[derive(Serialize)]
struct Section {
    title: String,
    paragraphs: Vec<Paragraph>,
}

#[derive(Serialize)]
struct Paragraph {
    text: String,
    page: usize,
}

#[derive(Clone)]
struct TextChar {
    ch: char,
    x0: f32,
    x1: f32,
    top: f32,
    bottom: f32,
    font_size: f32,
}

#[derive(Clone)]
struct Line {
    page: usize,
    text: String,
    top: f32,
    bottom: f32,
    font_size: f32,
    x0: f32,
    x1: f32,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let pdfium = init_pdfium(args.pdfium_path.as_deref())?;
    let document = pdfium
        .load_pdf_from_file(&args.pdf, None)
        .with_context(|| format!("Failed to load PDF {}", args.pdf.display()))?;

    let output = extract_document(&document, args.max_pages)?;

    match args.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        OutputFormat::Text => {
            println!("{}", render_text(&output));
        }
    }

    Ok(())
}

fn init_pdfium(pdfium_path: Option<&Path>) -> Result<Pdfium> {
    let bindings = if let Some(path) = pdfium_path {
        let library_path = if path.is_dir() {
            Pdfium::pdfium_platform_library_name_at_path(path)
        } else {
            path.to_path_buf()
        };
        Pdfium::bind_to_library(library_path)
            .context("Failed to bind to Pdfium library")?
    } else {
        Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./"))
            .or_else(|_| Pdfium::bind_to_system_library())
            .context("Failed to bind to Pdfium library")?
    };

    Ok(Pdfium::new(bindings))
}

fn extract_document(document: &PdfDocument, max_pages: Option<usize>) -> Result<DocumentOutput> {
    let page_count = document.pages().len() as usize;
    let limit = max_pages.unwrap_or(page_count).min(page_count);

    let mut lines = Vec::new();

    for (page_index, page) in document.pages().iter().take(limit).enumerate() {
        let page_lines = extract_page_lines(&page, page_index)?;
        lines.extend(page_lines);
    }

    lines.sort_by(|a, b| {
        a.page
            .cmp(&b.page)
            .then_with(|| {
                b.top
                    .partial_cmp(&a.top)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| a.x0.partial_cmp(&b.x0).unwrap_or(Ordering::Equal))
    });

    let body_font_size = median_font_size(lines.iter().map(|line| line.font_size).collect())
        .unwrap_or(12.0);
    let title = detect_title(&lines, body_font_size);
    let sections = build_sections(&lines, body_font_size);

    Ok(DocumentOutput { title, sections })
}

fn extract_page_lines(page: &PdfPage, page_index: usize) -> Result<Vec<Line>> {
    let chars = extract_chars(page)?;
    let page_height = page.height().value;
    let mut lines = chars_to_lines(chars, page_index);

    lines.retain(|line| !is_header_footer(line, page_height));

    Ok(lines)
}

fn extract_chars(page: &PdfPage) -> Result<Vec<TextChar>> {
    let text = page.text()?;
    let mut chars = Vec::new();

    for char in text.chars().iter() {
        let Some(ch) = char.unicode_char() else {
            continue;
        };
        if ch == '\0' || ch == '\n' || ch == '\r' || ch == '\t' {
            continue;
        }
        let bounds = char.loose_bounds()?;
        chars.push(TextChar {
            ch,
            x0: bounds.left().value,
            x1: bounds.right().value,
            top: bounds.top().value,
            bottom: bounds.bottom().value,
            font_size: char.unscaled_font_size().value,
        });
    }

    Ok(chars)
}

fn chars_to_lines(mut chars: Vec<TextChar>, page_index: usize) -> Vec<Line> {
    if chars.is_empty() {
        return Vec::new();
    }

    let line_tolerance = median_font_size(chars.iter().map(|c| c.font_size).collect())
        .unwrap_or(12.0)
        * 0.4;

    chars.sort_by(|a, b| {
        b.top
            .partial_cmp(&a.top)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.x0.partial_cmp(&b.x0).unwrap_or(Ordering::Equal))
    });

    let mut buckets: Vec<Vec<TextChar>> = Vec::new();

    for char in chars {
        if char.ch.is_whitespace() {
            continue;
        }
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
        .filter_map(|line_chars| build_line(line_chars, page_index))
        .collect()
}

fn build_line(mut chars: Vec<TextChar>, page_index: usize) -> Option<Line> {
    if chars.is_empty() {
        return None;
    }

    chars.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap_or(Ordering::Equal));

    let font_size = median_font_size(chars.iter().map(|c| c.font_size).collect())
        .unwrap_or(12.0);
    let mut text = String::new();
    let mut last_x1: Option<f32> = None;
    let spacing_threshold = (font_size * 0.35).max(1.5);

    for char in &chars {
        if let Some(prev_x1) = last_x1 {
            let gap = char.x0 - prev_x1;
            if gap > spacing_threshold && !text.ends_with(' ') {
                text.push(' ');
            }
        }
        text.push(char.ch);
        last_x1 = Some(char.x1);
    }

    let text = text.trim().to_string();
    if text.is_empty() {
        return None;
    }

    let top = chars
        .iter()
        .map(|c| c.top)
        .fold(f32::MIN, f32::max);
    let bottom = chars
        .iter()
        .map(|c| c.bottom)
        .fold(f32::MAX, f32::min);
    let x0 = chars.iter().map(|c| c.x0).fold(f32::MAX, f32::min);
    let x1 = chars.iter().map(|c| c.x1).fold(f32::MIN, f32::max);

    Some(Line {
        page: page_index,
        text,
        top,
        bottom,
        font_size,
        x0,
        x1,
    })
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

fn is_header_footer(line: &Line, page_height: f32) -> bool {
    if line.page == 0 {
        return is_page_number(line) && line.bottom <= FOOTER_MARGIN;
    }

    if line.top >= page_height - HEADER_MARGIN {
        return line.text.len() < 80;
    }

    if line.bottom <= FOOTER_MARGIN {
        return is_page_number(line) || line.text.len() < 10;
    }

    false
}

fn is_page_number(line: &Line) -> bool {
    let trimmed = line.text.trim();
    !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit())
}

fn detect_title(lines: &[Line], body_font_size: f32) -> Option<String> {
    let mut first_page_lines: Vec<&Line> = lines.iter().filter(|line| line.page == 0).collect();
    if first_page_lines.is_empty() {
        return None;
    }

    let max_font = first_page_lines
        .iter()
        .map(|line| line.font_size)
        .fold(0.0, f32::max);

    if max_font < body_font_size * 1.3 {
        return None;
    }

    first_page_lines.sort_by(|a, b| {
        b.top
            .partial_cmp(&a.top)
            .unwrap_or(Ordering::Equal)
    });

    let mut title_parts = Vec::new();
    for line in first_page_lines {
        if line.font_size >= max_font * 0.9 {
            title_parts.push(line.text.trim().to_string());
        }
    }

    if title_parts.is_empty() {
        None
    } else {
        Some(title_parts.join(" "))
    }
}

fn build_sections(lines: &[Line], body_font_size: f32) -> Vec<Section> {
    let paragraph_gap = body_font_size * 1.4;
    let mut sections = Vec::new();
    let mut current_section = Section {
        title: FRONT_MATTER_TITLE.to_string(),
        paragraphs: Vec::new(),
    };

    let mut current_paragraph = String::new();
    let mut current_page = 1;
    let mut last_line: Option<&Line> = None;
    let mut in_table = false;

    for line in lines {
        let text = normalize_whitespace(&line.text);
        if text.is_empty() {
            continue;
        }

        if is_table_start(&text) || is_table_line(&text) {
            flush_paragraph(&mut current_section, &mut current_paragraph, &mut current_page);
            in_table = true;
            last_line = None;
            continue;
        }

        if in_table {
            if is_table_line(&text) {
                continue;
            }
            if let Some(prev) = last_line {
                let gap = prev.bottom - line.top;
                if gap < paragraph_gap {
                    continue;
                }
            }
            in_table = false;
        }

        if is_heading_line(line, body_font_size) {
            flush_paragraph(&mut current_section, &mut current_paragraph, &mut current_page);
            if !current_section.paragraphs.is_empty()
                || current_section.title != FRONT_MATTER_TITLE
            {
                sections.push(current_section);
            }
            current_section = Section {
                title: text,
                paragraphs: Vec::new(),
            };
            last_line = None;
            continue;
        }

        if let Some(prev) = last_line {
            let gap = prev.bottom - line.top;
            if gap > paragraph_gap {
                flush_paragraph(&mut current_section, &mut current_paragraph, &mut current_page);
            }
        }

        if current_paragraph.is_empty() {
            current_page = line.page + 1;
        }

        append_line_text(&mut current_paragraph, &text);
        last_line = Some(line);
    }

    flush_paragraph(&mut current_section, &mut current_paragraph, &mut current_page);

    if !current_section.paragraphs.is_empty() || current_section.title != FRONT_MATTER_TITLE {
        sections.push(current_section);
    }

    sections
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
    ];

    if heading_keywords.contains(&lowercase.as_str()) {
        return true;
    }

    let word_count = text.split_whitespace().count();

    if line.font_size >= body_font_size * 1.2 && word_count <= 20 {
        return true;
    }

    is_all_caps(text) && word_count <= 12
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

fn is_table_start(text: &str) -> bool {
    let lowercase = text.to_ascii_lowercase();
    lowercase.starts_with("table ") || lowercase.starts_with("figure ")
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

fn append_line_text(paragraph: &mut String, line_text: &str) {
    if paragraph.is_empty() {
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

fn flush_paragraph(section: &mut Section, paragraph: &mut String, page: &mut usize) {
    if paragraph.is_empty() {
        return;
    }

    section.paragraphs.push(Paragraph {
        text: paragraph.trim().to_string(),
        page: *page,
    });
    paragraph.clear();
}

fn render_text(output: &DocumentOutput) -> String {
    let mut buffer = String::new();

    if let Some(title) = &output.title {
        buffer.push_str(title);
        buffer.push_str("\n\n");
    }

    for section in &output.sections {
        buffer.push_str(&section.title);
        buffer.push_str("\n");
        for paragraph in &section.paragraphs {
            buffer.push_str(&paragraph.text);
            buffer.push_str("\n\n");
        }
    }

    buffer
}
