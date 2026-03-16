// PDF generation: embed the nonogram print-page image into an A4 PDF,
// add a title header and puzzle-ID footer, write xochitl sidecar files,
// generate a thumbnail, and signal xochitl to rescan its library.

use printpdf::*;
use ::image::codecs::png::PngDecoder;
use std::fs;
use std::io::BufWriter;
use std::path::PathBuf;
use crate::nonogram::NonogramInfo;

const PAGE_W:    f64 = 210.0; // A4 mm
const PAGE_H:    f64 = 297.0;
const MARGIN:    f64 = 12.0;
const HEADER_H:  f64 = 14.0;  // space above image for title
const FOOTER_H:  f64 = 10.0;  // space below image for puzzle-ID

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

    // ── Scale image to fill available area (preserve aspect ratio) ────────────
    let avail_w = PAGE_W - 2.0 * MARGIN;
    let avail_h = PAGE_H - 2.0 * MARGIN - HEADER_H - FOOTER_H;

    let scale = (avail_w / img_w_px as f64).min(avail_h / img_h_px as f64);
    let draw_w = img_w_px as f64 * scale;
    let draw_h = img_h_px as f64 * scale;

    let img_x = MARGIN + (avail_w - draw_w) / 2.0;
    // printpdf: Y=0 is bottom of page
    let img_y = PAGE_H - MARGIN - HEADER_H - draw_h;

    // ── Build PDF ─────────────────────────────────────────────────────────────
    let (doc, page1, layer1) = PdfDocument::new(
        &info.title,
        Mm(PAGE_W), Mm(PAGE_H),
        "Layer 1",
    );
    let layer    = doc.get_page(page1).get_layer(layer1);
    let font     = doc.add_builtin_font(BuiltinFont::HelveticaBold)?;
    let font_reg = doc.add_builtin_font(BuiltinFont::Helvetica)?;

    // Title
    layer.use_text(
        &info.title,
        13.0,
        Mm(MARGIN),
        Mm(PAGE_H - MARGIN - 5.0),
        &font,
    );

    // Image — use reference DPI of 96; compute scale so rendered size == draw_w/h
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

    // Footer
    layer.set_fill_color(Color::Rgb(Rgb::new(0.5, 0.5, 0.5, None)));
    layer.use_text(
        &format!("nonograms.org  |  puzzle #{}", info.id),
        7.5,
        Mm(MARGIN),
        Mm(FOOTER_H / 2.0),
        &font_reg,
    );

    // ── Write files ───────────────────────────────────────────────────────────
    let uuid = gen_uuid();
    let base  = PathBuf::from(output_dir);
    fs::create_dir_all(&base)?;

    // PDF
    let pdf_path = base.join(format!("{}.pdf", uuid));
    doc.save(&mut BufWriter::new(fs::File::create(&pdf_path)?))?;

    // .metadata
    let ts           = now_ms();
    let visible_name = info.title.replace('"', "\\\"");
    let metadata = format!(
        r#"{{"deleted":false,"lastModified":"{ts}","lastOpened":"","lastOpenedPage":0,"metadatamodified":false,"modified":false,"parent":"","pinned":false,"synced":false,"type":"DocumentType","version":1,"visibleName":"{visible_name}"}}"#
    );
    fs::write(base.join(format!("{}.metadata", uuid)), &metadata)?;

    // .content
    let content = r#"{"dummyDocument":false,"extraMetadata":{},"fileType":"pdf","fontName":"","lastOpenedPage":0,"legacyEpub":false,"lineHeight":-1,"margins":180,"pageCount":1,"pages":[],"redirectionPageMap":[],"sizeInBytes":"0","tags":[],"textAlignment":"left","textScale":1,"transform":{}}"#;
    fs::write(base.join(format!("{}.content", uuid)), content)?;

    // .thumbnails/0.png  — xochitl requires this to show the document in the library
    let thumb_dir = base.join(format!("{}.thumbnails", uuid));
    fs::create_dir_all(&thumb_dir)?;
    // Reuse the original image as thumbnail — already ~300px wide, which is fine
    fs::write(thumb_dir.join("0.png"), &info.image_bytes)?;

    eprintln!("[pdf] saved {}.pdf", uuid);

    // ── Signal xochitl to rescan its document library ─────────────────────────
    // SIGHUP causes xochitl to reload without a full restart.
    let _ = std::process::Command::new("sh")
        .args(["-c", "kill -HUP $(pidof xochitl) 2>/dev/null || true"])
        .spawn();
    eprintln!("[pdf] sent SIGHUP to xochitl");

    Ok(pdf_path.to_string_lossy().to_string())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

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
