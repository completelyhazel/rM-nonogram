use std::io::Read;
use scraper::{Html, Selector};
use serde::Deserialize;

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Deserialize, Debug, Clone)]
pub struct FetchRequest {
    pub type_bw:    bool,
    pub size:       String,   // "5", "10", "15", "20", "25"
    pub difficulty: u32,      // 0=any (filtering not yet implemented)
}

/// Everything we need to produce a PDF.
pub struct NonogramInfo {
    pub id:          u32,
    pub title:       String,
    pub image_bytes: Vec<u8>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Return a list of puzzle IDs from the listing page matching the request.
pub fn search_nonograms(req: &FetchRequest) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let base = if req.type_bw {
        "https://www.nonograms.org/nonograms"
    } else {
        "https://www.nonograms.org/nonograms2"
    };

    let size_path = match req.size.as_str() {
        "5"  => "/size/xsmall",
        "10" => "/size/small",
        "15" => "/size/medium",
        "20" => "/size/large",
        "25" => "/size/xlarge",
        _    => "",
    };

    // Rotate through pages 1-8 so repeated fetches return different puzzles
    let page = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() % 8) + 1;

    let url = format!("{}{}/p/{}", base, size_path, page);
    eprintln!("[nonogram] search url: {}", url);

    let body = fetch_html(&url)?;
    eprintln!("[nonogram] search page: {} bytes", body.len());

    parse_ids(&body)
}

/// Download the print-page for `id`, extract the puzzle image URL, fetch it,
/// and return a NonogramInfo ready for PDF generation.
pub fn fetch_nonogram(id: u32, is_bw: bool) -> Result<NonogramInfo, Box<dyn std::error::Error>> {
    // The print page contains the fully-rendered nonogram (clues + empty grid)
    let print_url = if is_bw {
        format!("https://www.nonograms.org/nonogramprint/i/{}", id)
    } else {
        format!("https://www.nonograms.org/nonogramprint2/i/{}", id)
    };

    eprintln!("[nonogram] fetching print page: {}", print_url);
    let html = fetch_html(&print_url)?;
    eprintln!("[nonogram] print page: {} bytes", html.len());

    let doc = Html::parse_document(&html);

    // Title — try h1, h2, then <title> tag (print pages are minimal HTML)
    let title = ["h1", "h2", "h3"]
        .iter()
        .find_map(|sel| {
            doc.select(&Selector::parse(sel).unwrap())
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            // Fall back to <title> tag, stripping common suffixes
            doc.select(&Selector::parse("title").unwrap())
                .next()
                .map(|e| e.text().collect::<String>())
                .map(|s| s.split('|').next().unwrap_or(&s).trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| format!("Nonogram #{}", id));
    eprintln!("[nonogram] title: {}", title);

    // Find the puzzle image — hosted on static.nonograms.org
    let img_sel = Selector::parse("img[src*='static.nonograms.org']").unwrap();
    let img_url = doc
        .select(&img_sel)
        .next()
        .and_then(|el| el.value().attr("src"))
        .ok_or("Could not find puzzle image on print page")?
        .replace("_12_1_", "_19_4_")
        .to_string();

    eprintln!("[nonogram] image url: {}", img_url);

    let image_bytes = fetch_bytes(&img_url)?;
    eprintln!("[nonogram] image: {} bytes", image_bytes.len());

    Ok(NonogramInfo { id, title, image_bytes })
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn parse_ids(html: &str) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("a[href*='/i/']").unwrap();
    let mut ids: Vec<u32> = Vec::new();

    for el in doc.select(&sel) {
        if let Some(href) = el.value().attr("href") {
            if let Some(id_str) = href.split("/i/").nth(1) {
                let clean = id_str.split('?').next().unwrap_or("").trim();
                if let Ok(id) = clean.parse::<u32>() {
                    if id > 0 { ids.push(id); }
                }
            }
        }
    }

    ids.dedup();
    eprintln!("[nonogram] found {} puzzle IDs", ids.len());

    if ids.is_empty() {
        return Err("No puzzles found on search page".into());
    }
    Ok(ids)
}

fn build_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(8))
        .timeout_read(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(25))
        .build()
}

fn fetch_html(url: &str) -> Result<String, Box<dyn std::error::Error>> {
    let resp = build_agent()
        .get(url)
        .set("User-Agent", "Mozilla/5.0 (compatible; NonogramFetcher/1.0)")
        .set("Accept", "text/html,application/xhtml+xml")
        .call()?;
    Ok(resp.into_string()?)
}

fn fetch_bytes(url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let resp = build_agent()
        .get(url)
        .set("User-Agent", "Mozilla/5.0 (compatible; NonogramFetcher/1.0)")
        .call()?;
    let mut bytes = Vec::new();
    resp.into_reader().read_to_end(&mut bytes)?;
    Ok(bytes)
}
