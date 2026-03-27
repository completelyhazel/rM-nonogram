use scraper::{Html, Selector};
use serde::Deserialize;
use std::io::Read;

#[derive(Deserialize, Debug, Clone)]
pub struct FetchRequest {
    pub type_bw: bool,
    pub min_size: u8,
    pub max_size: u8,
    pub five_multiple: bool,
}

pub struct NonogramInfo {
    pub id: u32,
    pub title: String,
    pub image_bytes: Vec<u8>,
}

const SEARCH_URL: &str = "https://www.nonograms.org/search";

pub fn search_nonograms(req: &FetchRequest) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    // get page 1 to get the max page amount
    let search_params = format!(
        "?name=&colors={}&width_min={}&width_max={}&height_min={}&height_max={}{}",
        if req.type_bw { "1" } else { "2" },
        req.min_size,
        req.max_size,
        req.min_size,
        req.max_size,
        if req.five_multiple { "&size5=1" } else { "" }
    );

    let first_url: String = format!("{SEARCH_URL}{search_params}");

    eprintln!("[nonogram] fetching page 1 to count pages: {}", first_url);
    let first_body = fetch_html(&first_url)?;

    let total_pages = parse_total_pages(&first_body).unwrap_or(1);
    eprintln!("[nonogram] total pages: {}", total_pages);

    let page = if total_pages <= 1 {
        1
    } else {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as usize;
        (seed % total_pages) + 1
    };
    eprintln!("[nonogram] selected page {}/{}", page, total_pages);

    // avoid more requests if were lucky!!!
    let body = if page == 1 {
        first_body
    } else {
        let url = format!("{}/p/{}{}", SEARCH_URL, page, search_params);
        eprintln!("[nonogram] fetching page: {}", url);
        fetch_html(&url)?
    };

    parse_ids(&body)
}

fn parse_total_pages(html: &str) -> Option<usize> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("a[href*='/p/']").unwrap();
    let max = doc
        .select(&sel)
        .filter_map(|el| {
            let href = el.value().attr("href")?;
            let part = href.split("/p/").nth(1)?;
            part.split('?').next()?.trim().parse::<usize>().ok()
        })
        .max();
    max.or(Some(1))
}

pub fn fetch_nonogram(id: u32, is_bw: bool) -> Result<NonogramInfo, Box<dyn std::error::Error>> {
    let print_url = if is_bw {
        format!("https://www.nonograms.org/nonogramprint/i/{}", id)
    } else {
        format!("https://www.nonograms.org/nonogramprint2/i/{}", id)
    };

    eprintln!("[nonogram] fetching print page: {}", print_url);
    let html = fetch_html(&print_url)?;
    eprintln!("[nonogram] print page: {} bytes", html.len());

    let doc = Html::parse_document(&html);

    // title, try h1, h2, then <title> tag (print pages are minimal HTML)
    // let title = ["h1", "h2", "h3"]
    //     .iter()
    //     .find_map(|sel| {
    //         doc.select(&Selector::parse(sel).unwrap())
    //             .next()
    //             .map(|e| e.text().collect::<String>().trim().to_string())
    //             .filter(|s| !s.is_empty())
    //     })
    //     .or_else(|| {
    //         // Fall back to <title> tag, stripping common suffixes
    //         doc.select(&Selector::parse("title").unwrap())
    //             .next()
    //             .map(|e| e.text().collect::<String>())
    //             .map(|s| s.split('|').next().unwrap_or(&s).trim().to_string())
    //             .filter(|s| !s.is_empty())
    //     })
    //     .unwrap_or_else(|| format!("Nonogram #{}", id));
    let title: String = format!("Nonogram #{}", id);
    eprintln!("[nonogram] title: {}", title);

    // find da image
    let img_sel = Selector::parse("img[src*='static.nonograms.org']").unwrap();
    let img_url = doc
        .select(&img_sel)
        .next()
        .and_then(|el| el.value().attr("src"))
        .ok_or("Could not find puzzle image on print page")?
        .replace("_12_1_", "_19_4_") // highest res version
        .to_string();

    eprintln!("[nonogram] image url: {}", img_url);

    let image_bytes = fetch_bytes(&img_url)?;
    eprintln!("[nonogram] image: {} bytes", image_bytes.len());

    Ok(NonogramInfo {
        id,
        title,
        image_bytes,
    })
}

fn parse_ids(html: &str) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("a[href*='/i/']").unwrap();
    let mut ids: Vec<u32> = Vec::new();

    for el in doc.select(&sel) {
        if let Some(href) = el.value().attr("href") {
            if let Some(id_str) = href.split("/i/").nth(1) {
                let clean = id_str.split('?').next().unwrap_or("").trim();
                if let Ok(id) = clean.parse::<u32>() {
                    if id > 0 {
                        ids.push(id);
                    }
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
        .set(
            "User-Agent",
            "Mozilla/5.0 (compatible; NonogramFetcher/1.0)",
        )
        .set("Accept", "text/html,application/xhtml+xml")
        .call()?;
    Ok(resp.into_string()?)
}

fn fetch_bytes(url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let resp = build_agent()
        .get(url)
        .set(
            "User-Agent",
            "Mozilla/5.0 (compatible; NonogramFetcher/1.0)",
        )
        .call()?;
    let mut bytes = Vec::new();
    resp.into_reader().read_to_end(&mut bytes)?;
    Ok(bytes)
}
