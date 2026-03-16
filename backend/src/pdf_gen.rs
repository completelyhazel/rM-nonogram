// ============================================================================
//  pdf_gen.rs — Genera el PDF del nonograma para el reMarkable Paper Pro
//  Usa printpdf 0.5.x (API estable con Rust 1.75+)
// ============================================================================

use printpdf::*;
use std::fs;
use std::io::BufWriter;
use std::path::PathBuf;
use crate::nonogram::{NonogramPuzzle, ClueEntry};

const PAGE_W: f64 = 210.0;
const PAGE_H: f64 = 297.0;
const MARGIN_TOP:    f64 = 18.0;
const MARGIN_BOTTOM: f64 = 15.0;
const MARGIN_LEFT:   f64 = 15.0;
const MARGIN_RIGHT:  f64 = 15.0;
const TITLE_H:   f64 = 10.0;
const TITLE_GAP: f64 = 6.0;

pub fn generate_pdf(
    puzzle:     &NonogramPuzzle,
    output_dir: &str,
) -> Result<String, Box<dyn std::error::Error>> {

    let max_clue_cols = puzzle.col_clues.iter().map(|c| c.len()).max().unwrap_or(1);
    let max_clue_rows = puzzle.row_clues.iter().map(|r| r.len()).max().unwrap_or(1);

    let avail_w = PAGE_W - MARGIN_LEFT - MARGIN_RIGHT;
    let avail_h = PAGE_H - MARGIN_TOP - MARGIN_BOTTOM - TITLE_H - TITLE_GAP;

    let total_cols = (puzzle.width  + max_clue_rows) as f64;
    let total_rows = (puzzle.height + max_clue_cols) as f64;

    let cell: f64 = (avail_w / total_cols).min(avail_h / total_rows).min(14.0).max(2.5);

    let clue_w = max_clue_rows as f64 * cell;
    let clue_h = max_clue_cols as f64 * cell;
    let grid_w = puzzle.width  as f64 * cell;

    let block_w = clue_w + grid_w;
    let origin_x = MARGIN_LEFT + (avail_w - block_w) / 2.0;

    // En printpdf Y=0 está abajo, Y=297 arriba.
    // col_zone_top = borde superior de la zona de pistas de columnas
    let col_zone_top = PAGE_H - MARGIN_TOP - TITLE_H - TITLE_GAP;
    // top_y = borde superior del grid (= col_zone_top - clue_h)
    let top_y = col_zone_top - clue_h;

    let (doc, page1, layer1) = PdfDocument::new(
        &puzzle.title,
        Mm(PAGE_W), Mm(PAGE_H),
        "Layer 1",
    );
    let layer = doc.get_page(page1).get_layer(layer1);
    let font      = doc.add_builtin_font(BuiltinFont::Helvetica)?;
    let font_bold = doc.add_builtin_font(BuiltinFont::HelveticaBold)?;

    // Título
    layer.use_text(
        &puzzle.title, 16.0,
        Mm(MARGIN_LEFT), Mm(PAGE_H - MARGIN_TOP - 4.0),
        &font_bold,
    );
    let sub = format!("{}×{}  {}  — nonograms.org",
        puzzle.width, puzzle.height,
        if puzzle.is_bw { "Black & White" } else { "Color" }
    );
    layer.use_text(&sub, 8.0, Mm(MARGIN_LEFT), Mm(PAGE_H - MARGIN_TOP - 9.5), &font);

    // Pistas de columnas
    let grid_x = origin_x + clue_w;
    for (col_idx, clues) in puzzle.col_clues.iter().enumerate() {
        let cx = grid_x + col_idx as f64 * cell + cell * 0.5;
        let n  = clues.len();
        for (i, clue) in clues.iter().enumerate() {
            if clue.count == 0 { continue; }
            let row_off = max_clue_cols - n + i;
            let cy = col_zone_top - (row_off as f64 + 0.7) * cell;
            set_fill(&layer, clue, &puzzle.palette, puzzle.is_bw);
            layer.use_text(&clue.count.to_string(), clue_pt(cell), Mm(cx - 1.5), Mm(cy), &font);
        }
    }

    // Pistas de filas
    for (row_idx, clues) in puzzle.row_clues.iter().enumerate() {
        // row_idx=0 → fila de arriba → Y más alto
        let cy = top_y - row_idx as f64 * cell + cell * 0.5 - 1.5;
        let n  = clues.len();
        for (i, clue) in clues.iter().enumerate() {
            if clue.count == 0 { continue; }
            let col_off = max_clue_rows - n + i;
            let cx = origin_x + col_off as f64 * cell + 0.8;
            set_fill(&layer, clue, &puzzle.palette, puzzle.is_bw);
            layer.use_text(&clue.count.to_string(), clue_pt(cell), Mm(cx), Mm(cy), &font);
        }
    }

    layer.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));

    // Grid vacío
    draw_grid(&layer, grid_x, top_y, puzzle.width, puzzle.height, cell);

    // Números de referencia cada 5 (cuando las celdas son suficientemente grandes)
    if cell >= 5.0 {
        draw_grid_numbers(&layer, &font, grid_x, top_y, puzzle.width, puzzle.height, cell);
    }

    // Pie
    layer.set_fill_color(Color::Rgb(Rgb::new(0.55, 0.55, 0.55, None)));
    layer.use_text(
        &format!("nonograms.org  ·  puzzle #{}", puzzle.id),
        7.0, Mm(MARGIN_LEFT), Mm(8.0),
        &font,
    );
    layer.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));

    // Guardar con UUID para que xochitl lo indexe correctamente
    let uuid = gen_uuid();
    let output_path = PathBuf::from(output_dir).join(format!("{}.pdf", uuid));
    fs::create_dir_all(output_dir)?;
    doc.save(&mut BufWriter::new(fs::File::create(&output_path)?))?;

    // .metadata — xochitl lo requiere para mostrar el documento
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let visible_name = puzzle.title.replace('"', "\"");
    let metadata = format!(
        r#"{{"deleted":false,"lastModified":"{ts}","lastOpened":"","lastOpenedPage":0,"metadatamodified":false,"modified":false,"parent":"","pinned":false,"synced":false,"type":"DocumentType","version":1,"visibleName":"{visible_name}"}}"#
    );
    fs::write(PathBuf::from(output_dir).join(format!("{}.metadata", uuid)), &metadata)?;

    // .content — xochitl requiere este archivo
    let content_json = r#"{"dummyDocument":false,"extraMetadata":{},"fileType":"pdf","fontName":"","lastOpenedPage":0,"legacyEpub":false,"lineHeight":-1,"margins":180,"pageCount":1,"pages":[],"redirectionPageMap":[],"sizeInBytes":"0","tags":[],"textAlignment":"left","textScale":1,"transform":{}}"#;
    fs::write(PathBuf::from(output_dir).join(format!("{}.content", uuid)), content_json)?;

    Ok(output_path.to_string_lossy().to_string())
}

fn gen_uuid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    // UUID v4-like generado desde tiempo + pseudo-aleatorio
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (t & 0xffffffff) as u32,
        ((t >> 32) & 0xffff) as u16,
        ((t >> 48) & 0x0fff) as u16,
        (0x8000 | ((t >> 60) & 0x3fff)) as u16,
        (t.wrapping_mul(6364136223846793005)) & 0xffffffffffff_u128
    )
}

fn draw_grid(layer: &PdfLayerReference, ox: f64, oy: f64, w: usize, h: usize, cell: f64) {
    let right  = ox + w as f64 * cell;
    let bottom = oy - h as f64 * cell;

    for row in 0..=h {
        let y     = oy - row as f64 * cell;
        let major = row % 5 == 0;
        layer.set_outline_color(Color::Rgb(Rgb::new(
            if major { 0.0 } else { 0.6 },
            if major { 0.0 } else { 0.6 },
            if major { 0.0 } else { 0.6 },
            None,
        )));
        layer.set_outline_thickness(if major { 0.45 } else { 0.12 });
        layer.add_shape(Line {
            points: vec![(Point::new(Mm(ox), Mm(y)), false), (Point::new(Mm(right), Mm(y)), false)],
            is_closed: false, has_fill: false, has_stroke: true, is_clipping_path: false,
        });
    }

    for col in 0..=w {
        let x     = ox + col as f64 * cell;
        let major = col % 5 == 0;
        layer.set_outline_color(Color::Rgb(Rgb::new(
            if major { 0.0 } else { 0.6 },
            if major { 0.0 } else { 0.6 },
            if major { 0.0 } else { 0.6 },
            None,
        )));
        layer.set_outline_thickness(if major { 0.45 } else { 0.12 });
        layer.add_shape(Line {
            points: vec![(Point::new(Mm(x), Mm(oy)), false), (Point::new(Mm(x), Mm(bottom)), false)],
            is_closed: false, has_fill: false, has_stroke: true, is_clipping_path: false,
        });
    }

    // Borde exterior
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

fn draw_grid_numbers(layer: &PdfLayerReference, font: &IndirectFontRef, ox: f64, oy: f64, w: usize, h: usize, cell: f64) {
    let fpt = (cell * 0.28 * 2.835).max(4.0).min(7.0);
    layer.set_fill_color(Color::Rgb(Rgb::new(0.72, 0.72, 0.72, None)));
    for col in (4..w).step_by(5) {
        layer.use_text(&(col+1).to_string(), fpt, Mm(ox + col as f64 * cell + 0.5), Mm(oy + 1.0), font);
    }
    for row in (4..h).step_by(5) {
        layer.use_text(&(row+1).to_string(), fpt, Mm(ox - cell * 0.95), Mm(oy - row as f64 * cell - 1.5), font);
    }
    layer.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
}

fn clue_pt(cell: f64) -> f64 {
    (cell * 0.58 * 2.835).max(5.0).min(11.0)
}

fn set_fill(layer: &PdfLayerReference, clue: &ClueEntry, palette: &[String], is_bw: bool) {
    if !is_bw && clue.color_idx > 0 {
        if let Some(hex) = palette.get((clue.color_idx - 1) as usize) {
            let (r, g, b) = hex_rgb(hex);
            layer.set_fill_color(Color::Rgb(Rgb::new(r, g, b, None)));
            return;
        }
    }
    layer.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
}

fn hex_rgb(hex: &str) -> (f64, f64, f64) {
    let h = hex.trim_start_matches('#');
    if h.len() != 6 { return (0.0, 0.0, 0.0); }
    let r = u8::from_str_radix(&h[0..2], 16).unwrap_or(0) as f64 / 255.0;
    let g = u8::from_str_radix(&h[2..4], 16).unwrap_or(0) as f64 / 255.0;
    let b = u8::from_str_radix(&h[4..6], 16).unwrap_or(0) as f64 / 255.0;
    (r, g, b)
}
