// PDF generation: embed the nonogram print-page image into an A4 PDF,
// write xochitl sidecar files (.metadata, .content, .thumbnails/),
// and signal xochitl to rescan its library.

use printpdf::*;
use ::image::codecs::png::PngDecoder;
use std::fs;
use std::io::BufWriter;
use std::path::PathBuf;
use crate::nonogram::NonogramInfo;

const PAGE_W:   f64 = 210.0; // A4 width  (mm)
const PAGE_H:   f64 = 297.0; // A4 height (mm)
const MARGIN:   f64 = 12.0;
const HEADER_H: f64 = 14.0;  // reserved above the image for the title
const FOOTER_H: f64 = 10.0;  // reserved below the image for the puzzle ID

pub fn generate_pdf(
    info:       &NonogramInfo,
    output_dir: &str,
) -> Result<String, Box<dyn std::error::Error>> {

    // ── Decode image dimensions ───────────────────────────────────────────────
    let (img_w_px, img_h_px) = {
        use ::image::ImageDecoder;
        let dec = PngDecoder::new(std::io::Cursor::new(&info.image_bytes))?;
        dec.dimensions()
    };
    eprintln!("[pdf] image {}x{} px", img_w_px, img_h_px);

    // ── Scale the image to fill the available area (preserve aspect ratio) ────
    let avail_w = PAGE_W - 2.0 * MARGIN;
    let avail_h = PAGE_H - 2.0 * MARGIN - HEADER_H - FOOTER_H;

    let scale  = (avail_w / img_w_px as f64).min(avail_h / img_h_px as f64);
    let draw_w = img_w_px as f64 * scale;
    let draw_h = img_h_px as f64 * scale;

    // Center horizontally; printpdf uses Y=0 at the bottom of the page.
    let img_x = MARGIN + (avail_w - draw_w) / 2.0;
    let img_y = PAGE_H - MARGIN - HEADER_H - draw_h;

    // ── Build the PDF ─────────────────────────────────────────────────────────
    let (doc, page1, layer1) = PdfDocument::new(
        &info.title,
        Mm(PAGE_W),
        Mm(PAGE_H),
        "Layer 1",
    );
    let layer    = doc.get_page(page1).get_layer(layer1);
    let font     = doc.add_builtin_font(BuiltinFont::HelveticaBold)?;
    let font_reg = doc.add_builtin_font(BuiltinFont::Helvetica)?;

    // Title header
    layer.use_text(
        &info.title,
        13.0,
        Mm(MARGIN),
        Mm(PAGE_H - MARGIN - 5.0),
        &font,
    );

    // Compute the printpdf scale factors from a reference DPI of 96.
    let dpi          = 96.0_f64;
    let natural_w_mm = img_w_px as f64 / dpi * 25.4;
    let natural_h_mm = img_h_px as f64 / dpi * 25.4;
    let sx = draw_w / natural_w_mm;
    let sy = draw_h / natural_h_mm;

    let pdf_image = Image::try_from(
        PngDecoder::new(std::io::Cursor::new(&info.image_bytes))?
    )?;
    pdf_image.add_to_layer(layer.clone(), ImageTransform {
        translate_x: Some(Mm(img_x)),
        translate_y: Some(Mm(img_y)),
        scale_x:     Some(sx),
        scale_y:     Some(sy),
        dpi:         Some(dpi),
        ..Default::default()
    });

    // Footer with puzzle source and ID (useful for checking the solution online)
    layer.set_fill_color(Color::Rgb(Rgb::new(0.5, 0.5, 0.5, None)));
    layer.use_text(
        &format!("nonograms.org  |  puzzle #{}", info.id),
        7.5,
        Mm(MARGIN),
        Mm(FOOTER_H / 2.0),
        &font_reg,
    );

    // ── Write files to disk ───────────────────────────────────────────────────
    let uuid = gen_uuid();
    let base  = PathBuf::from(output_dir);
    fs::create_dir_all(&base)?;

    // Main PDF file
    let pdf_path = base.join(format!("{}.pdf", uuid));
    doc.save(&mut BufWriter::new(fs::File::create(&pdf_path)?))?;

    // .metadata — required by xochitl to index the document
    let ts           = now_ms();
    let visible_name = info.title.replace('"', "\\\"");
    let metadata = format!(
        r#"{{"deleted":false,"lastModified":"{ts}","lastOpened":"","lastOpenedPage":0,"metadatamodified":false,"modified":false,"parent":"","pinned":false,"synced":false,"type":"DocumentType","version":1,"visibleName":"{visible_name}"}}"#
    );
    fs::write(base.join(format!("{}.metadata", uuid)), &metadata)?;

    // .content — required by xochitl to know the file type
    let content = r#"{"dummyDocument":false,"extraMetadata":{},"fileType":"pdf","fontName":"","lastOpenedPage":0,"legacyEpub":false,"lineHeight":-1,"margins":180,"pageCount":1,"pages":[],"redirectionPageMap":[],"sizeInBytes":"0","tags":[],"textAlignment":"left","textScale":1,"transform":{}}"#;
    fs::write(base.join(format!("{}.content", uuid)), content)?;

    // .thumbnails/0.png — xochitl shows this as the document preview icon.
    // Reuse the puzzle image; it's already an appropriate size (~300 px wide).
    let thumb_dir = base.join(format!("{}.thumbnails", uuid));
    fs::create_dir_all(&thumb_dir)?;
    fs::write(thumb_dir.join("0.png"), &info.image_bytes)?;

    eprintln!("[pdf] saved {}.pdf", uuid);

    // NOTE: Do NOT send SIGHUP to xochitl here.
    // On reMarkable Paper Pro firmware (imx8mm), SIGHUP terminates xochitl
    // entirely instead of triggering a library reload, which kills the AppLoad
    // session and freezes the UI mid-use.
    // The new document will appear in the library on the next normal xochitl
    // startup (e.g. after the user exits AppLoad).
    eprintln!("[pdf] skipping xochitl notify (SIGHUP kills xochitl on this firmware)");

    Ok(pdf_path.to_string_lossy().to_string())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Current Unix time in milliseconds (for xochitl metadata timestamps).
fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// Generate a v4-ish UUID from the current nanosecond timestamp.
/// Not cryptographically random, but unique enough for a document filename.
fn gen_uuid() -> String {
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
        t.wrapping_mul(6_364_136_223_846_793_005) & 0xffff_ffff_ffff_u128,
    )
}
