// ============================================================================
//  pdf_gen.rs — Generates a nonogram PDF for the reMarkable Paper Pro.
//  Uses printpdf 0.5.x (stable API, compatible with Rust 1.75+).
// ============================================================================

use printpdf::*;
use std::fs;
use std::io::BufWriter;
use std::path::PathBuf;
use crate::nonogram::{NonogramPuzzle, ClueEntry};

// Page dimensions (A4, mm)
const PAGE_W: f64 = 210.0;
const PAGE_H: f64 = 297.0;

const MARGIN_TOP:    f64 = 18.0;
const MARGIN_BOTTOM: f64 = 15.0;
const MARGIN_LEFT:   f64 = 15.0;
const MARGIN_RIGHT:  f64 = 15.0;
const TITLE_H:       f64 = 10.0;  // vertical space reserved for the title block
const TITLE_GAP:     f64 = 6.0;   // gap between title and the clue area

pub fn generate_pdf(
    puzzle:     &NonogramPuzzle,
    output_dir: &str,
) -> Result<String, Box<dyn std::error::Error>> {

    let max_clue_cols = puzzle.col_clues.iter().map(|c| c.len()).max().unwrap_or(1);
    let max_clue_rows = puzzle.row_clues.iter().map(|r| r.len()).max().unwrap_or(1);

    let avail_w = PAGE_W - MARGIN_LEFT - MARGIN_RIGHT;
    let avail_h = PAGE_H - MARGIN_TOP - MARGIN_BOTTOM - TITLE_H - TITLE_GAP;

    // Total logical columns / rows including the clue areas
    let total_cols = (puzzle.width  + max_clue_rows) as f64;
    let total_rows = (puzzle.height + max_clue_cols) as f64;

    // Cell size: fit everything on the page, capped at 14 mm, minimum 2.5 mm
    let cell: f64 = (avail_w / total_cols).min(avail_h / total_rows).min(14.0).max(2.5);

    let clue_w = max_clue_rows as f64 * cell;
    let clue_h = max_clue_cols as f64 * cell;
    let grid_w = puzzle.width  as f64 * cell;

    let block_w = clue_w + grid_w;
    // Centre the block horizontally
    let origin_x = MARGIN_LEFT + (avail_w - block_w) / 2.0;

    // In printpdf Y=0 is at the bottom, Y=PAGE_H is at the top.
    // col_zone_top: top edge of the column-clue area
    let col_zone_top = PAGE_H - MARGIN_TOP - TITLE_H - TITLE_GAP;
    // top_y: top edge of the empty grid
    let top_y = col_zone_top - clue_h;

    let (doc, page1, layer1) = PdfDocument::new(
        &puzzle.title,
        Mm(PAGE_W), Mm(PAGE_H),
        "Layer 1",
    );
    let layer = doc.get_page(page1).get_layer(layer1);
    let font      = doc.add_builtin_font(BuiltinFont::Helvetica)?;
    let font_bold = doc.add_builtin_font(BuiltinFont::HelveticaBold)?;

    // ── Title block ───────────────────────────────────────────────────────────
    layer.use_text(
        &puzzle.title, 16.0,
        Mm(MARGIN_LEFT), Mm(PAGE_H - MARGIN_TOP - 4.0),
        &font_bold,
    );
    let subtitle = format!(
        "{}×{}  {}  — nonograms.org",
        puzzle.width, puzzle.height,
        if puzzle.is_bw { "Black & White" } else { "Color" }
    );
    layer.use_text(&subtitle, 8.0, Mm(MARGIN_LEFT), Mm(PAGE_H - MARGIN_TOP - 9.5), &font);

    // ── Column clues ──────────────────────────────────────────────────────────
    let grid_x = origin_x + clue_w;
    for (col_idx, clues) in puzzle.col_clues.iter().enumerate() {
        let cx = grid_x + col_idx as f64 * cell + cell * 0.5;
        let n  = clues.len();
        for (i, clue) in clues.iter().enumerate() {
            if clue.count == 0 { continue; }
            // Align clues to the bottom of the clue zone
            let row_off = max_clue_cols - n + i;
            let cy = col_zone_top - (row_off as f64 + 0.7) * cell;
            set_fill(&layer, clue, &puzzle.palette, puzzle.is_bw);
            layer.use_text(&clue.count.to_string(), clue_pt(cell), Mm(cx - 1.5), Mm(cy), &font);
        }
    }

    // ── Row clues ─────────────────────────────────────────────────────────────
    for (row_idx, clues) in puzzle.row_clues.iter().enumerate() {
        // row_idx=0 → top row → highest Y value
        let cy = top_y - row_idx as f64 * cell + cell * 0.5 - 1.5;
        let n  = clues.len();
        for (i, clue) in clues.iter().enumerate() {
            if clue.count == 0 { continue; }
            // Align clues to the right of the clue zone
            let col_off = max_clue_rows - n + i;
            let cx = origin_x + col_off as f64 * cell + 0.8;
            set_fill(&layer, clue, &puzzle.palette, puzzle.is_bw);
            layer.use_text(&clue.count.to_string(), clue_pt(cell), Mm(cx), Mm(cy), &font);
        }
    }

    layer.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));

    // ── Empty grid ────────────────────────────────────────────────────────────
    draw_grid(&layer, grid_x, top_y, puzzle.width, puzzle.height, cell);

    // Reference numbers every 5 cells (only when cells are large enough to read)
    if cell >= 5.0 {
        draw_grid_numbers(&layer, &font, grid_x, top_y, puzzle.width, puzzle.height, cell);
    }

    // ── Footer ────────────────────────────────────────────────────────────────
    layer.set_fill_color(Color::Rgb(Rgb::new(0.55, 0.55, 0.55, None)));
    layer.use_text(
        &format!("nonograms.org  ·  puzzle #{}", puzzle.id),
        7.0, Mm(MARGIN_LEFT), Mm(8.0),
        &font,
    );
    layer.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));

    // ── Save to disk ──────────────────────────────────────────────────────────
    // Use a UUID-like filename so xochitl indexes the document correctly
    let uuid = gen_uuid();
    let output_path = PathBuf::from(output_dir).join(format!("{}.pdf", uuid));
    fs::create_dir_all(output_dir)?;
    doc.save(&mut BufWriter::new(fs::File::create(&output_path)?))?;

    // .metadata — required by xochitl to display the document in the library
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let visible_name = puzzle.title.replace('"', "\"");
    let metadata = format!(
        r#"{{"deleted":false,"lastModified":"{ts}","lastOpened":"","lastOpenedPage":0,"metadatamodified":false,"modified":false,"parent":"","pinned":false,"synced":false,"type":"DocumentType","version":1,"visibleName":"{visible_name}"}}"#
    );
    fs::write(PathBuf::from(output_dir).join(format!("{}.metadata", uuid)), &metadata)?;

    // .content — also required by xochitl
    let content_json = r#"{"dummyDocument":false,"extraMetadata":{},"fileType":"pdf","fontName":"","lastOpenedPage":0,"legacyEpub":false,"lineHeight":-1,"margins":180,"pageCount":1,"pages":[],"redirectionPageMap":[],"sizeInBytes":"0","tags":[],"textAlignment":"left","textScale":1,"transform":{}}"#;
    fs::write(PathBuf::from(output_dir).join(format!("{}.content", uuid)), content_json)?;

    Ok(output_path.to_string_lossy().to_string())
}

// ─── UUID-like filename generator ────────────────────────────────────────────

fn gen_uuid() -> String {
    // UUID v4-flavoured name built from nanosecond timestamp (good enough for
    // our purposes — we don't need cryptographic randomness here)
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (t & 0xffff_ffff) as u32,
        ((t >> 32) & 0xffff) as u16,
        ((t >> 48) & 0x0fff) as u16,
        (0x8000 | ((t >> 60) & 0x3fff)) as u16,
        (t.wrapping_mul(6364136223846793005)) & 0xffffffffffff_u128
    )
}

// ─── Grid drawing ─────────────────────────────────────────────────────────────

fn draw_grid(
    layer: &PdfLayerReference,
    ox: f64, oy: f64,
    w: usize, h: usize,
    cell: f64,
) {
    let right  = ox + w as f64 * cell;
    let bottom = oy - h as f64 * cell;

    // Horizontal lines
    for row in 0..=h {
        let y     = oy - row as f64 * cell;
        let major = row % 5 == 0;
        let grey  = if major { 0.0 } else { 0.6 };
        layer.set_outline_color(Color::Rgb(Rgb::new(grey, grey, grey, None)));
        layer.set_outline_thickness(if major { 0.45 } else { 0.12 });
        layer.add_shape(Line {
            points: vec![
                (Point::new(Mm(ox),    Mm(y)), false),
                (Point::new(Mm(right), Mm(y)), false),
            ],
            is_closed: false, has_fill: false, has_stroke: true, is_clipping_path: false,
        });
    }

    // Vertical lines
    for col in 0..=w {
        let x     = ox + col as f64 * cell;
        let major = col % 5 == 0;
        let grey  = if major { 0.0 } else { 0.6 };
        layer.set_outline_color(Color::Rgb(Rgb::new(grey, grey, grey, None)));
        layer.set_outline_thickness(if major { 0.45 } else { 0.12 });
        layer.add_shape(Line {
            points: vec![
                (Point::new(Mm(x), Mm(oy)),     false),
                (Point::new(Mm(x), Mm(bottom)), false),
            ],
            is_closed: false, has_fill: false, has_stroke: true, is_clipping_path: false,
        });
    }

    // Outer border (heavier stroke)
    layer.set_outline_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
    layer.set_outline_thickness(0.8);
    layer.add_shape(Line {
        points: vec![
            (Point::new(Mm(ox),    Mm(oy)),     false),
            (Point::new(Mm(right), Mm(oy)),     false),
            (Point::new(Mm(right), Mm(bottom)), false),
            (Point::new(Mm(ox),    Mm(bottom)), false),
        ],
        is_closed: true, has_fill: false, has_stroke: true, is_clipping_path: false,
    });
}

fn draw_grid_numbers(
    layer: &PdfLayerReference,
    font: &IndirectFontRef,
    ox: f64, oy: f64,
    w: usize, h: usize,
    cell: f64,
) {
    let fpt = (cell * 0.28 * 2.835).max(4.0).min(7.0);
    layer.set_fill_color(Color::Rgb(Rgb::new(0.72, 0.72, 0.72, None)));

    // Column numbers above the grid (every 5 columns)
    for col in (4..w).step_by(5) {
        layer.use_text(
            &(col + 1).to_string(), fpt,
            Mm(ox + col as f64 * cell + 0.5), Mm(oy + 1.0),
            font,
        );
    }

    // Row numbers to the left of the grid (every 5 rows)
    for row in (4..h).step_by(5) {
        layer.use_text(
            &(row + 1).to_string(), fpt,
            Mm(ox - cell * 0.95), Mm(oy - row as f64 * cell - 1.5),
            font,
        );
    }

    layer.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Font size (in PDF points) for clue numbers, scaled to cell size.
fn clue_pt(cell: f64) -> f64 {
    (cell * 0.58 * 2.835).max(5.0).min(11.0)
}

/// Set the fill colour to match a clue entry's colour index.
fn set_fill(
    layer:   &PdfLayerReference,
    clue:    &ClueEntry,
    palette: &[String],
    is_bw:   bool,
) {
    if !is_bw && clue.color_idx > 0 {
        if let Some(hex) = palette.get((clue.color_idx - 1) as usize) {
            let (r, g, b) = hex_to_rgb(hex);
            layer.set_fill_color(Color::Rgb(Rgb::new(r, g, b, None)));
            return;
        }
    }
    layer.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
}

/// Parse a CSS hex colour string (e.g. `"#1a2b3c"`) into normalised RGB floats.
fn hex_to_rgb(hex: &str) -> (f64, f64, f64) {
    let h = hex.trim_start_matches('#');
    if h.len() != 6 { return (0.0, 0.0, 0.0); }
    let r = u8::from_str_radix(&h[0..2], 16).unwrap_or(0) as f64 / 255.0;
    let g = u8::from_str_radix(&h[2..4], 16).unwrap_or(0) as f64 / 255.0;
    let b = u8::from_str_radix(&h[4..6], 16).unwrap_or(0) as f64 / 255.0;
    (r, g, b)
}
