use scraper::{Html, Selector};
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct FetchRequest {
    pub type_bw:    bool,
    pub size:       String,   // "5", "10", "15", "20", "25"
    pub difficulty: u32,      // 0=any, 1=easy, 2=medium, 3=hard
}

#[derive(Debug, Clone)]
pub struct NonogramPuzzle {
    pub id:        u32,
    pub title:     String,
    pub is_bw:     bool,
    pub grid:      Vec<Vec<u32>>,
    pub col_clues: Vec<Vec<ClueEntry>>,
    pub row_clues: Vec<Vec<ClueEntry>>,
    pub palette:   Vec<String>,
    pub width:     usize,
    pub height:    usize,
}

#[derive(Debug, Clone)]
pub struct ClueEntry {
    pub count:     u32,
    pub color_idx: u32,
}

// ── Search ────────────────────────────────────────────────────────────────────

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

    // Rotate through pages 1-8 based on current time
    let page = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() % 8) + 1;

    let url = format!("{}{}/p/{}", base, size_path, page);
    eprintln!("[nonogram] searching: {}", url);

    let body = fetch_html(&url)?;
    eprintln!("[nonogram] search page: {} bytes", body.len());
    parse_ids(&body, req.difficulty)
}

fn parse_ids(html: &str, _difficulty: u32) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("a[href*='/i/']").unwrap();
    let mut ids: Vec<u32> = Vec::new();

    for el in doc.select(&sel) {
        if let Some(href) = el.value().attr("href") {
            if let Some(id_str) = href.split("/i/").nth(1) {
                let id_clean = id_str.split('?').next().unwrap_or("").trim();
                if let Ok(id) = id_clean.parse::<u32>() {
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

// ── Puzzle download ───────────────────────────────────────────────────────────

pub fn fetch_nonogram(id: u32, is_bw: bool) -> Result<NonogramPuzzle, Box<dyn std::error::Error>> {
    let url = if is_bw {
        format!("https://www.nonograms.org/nonograms/i/{}", id)
    } else {
        format!("https://www.nonograms.org/nonograms2/i/{}", id)
    };

    eprintln!("[nonogram] downloading: {}", url);
    let body = fetch_html(&url)?;
    eprintln!("[nonogram] puzzle page: {} bytes", body.len());
    parse_puzzle(&body, id, is_bw)
}

fn parse_puzzle(
    html: &str,
    id: u32,
    is_bw: bool,
) -> Result<NonogramPuzzle, Box<dyn std::error::Error>> {
    let doc = Html::parse_document(html);

    let title = doc
        .select(&Selector::parse("h1").unwrap())
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Nonogram #{}", id));

    eprintln!("[nonogram] title: {}", title);

    let scripts: Vec<String> = doc
        .select(&Selector::parse("script").unwrap())
        .map(|e| e.text().collect::<String>())
        .collect();

    let grid = extract_grid(&scripts)
        .ok_or("Could not find valid var d=[[...]] in page")?;

    let height  = grid.len();
    let width   = grid[0].len();

    let palette = if is_bw {
        vec!["#000000".to_string()]
    } else {
        extract_palette(&scripts).unwrap_or_else(|| vec!["#000000".to_string()])
    };

    eprintln!("[nonogram] grid {}×{}, palette {} colors", width, height, palette.len());

    let row_clues = compute_row_clues(&grid, is_bw);
    let col_clues = compute_col_clues(&grid, width, is_bw);

    Ok(NonogramPuzzle { id, title, is_bw, grid, col_clues, row_clues, palette, width, height })
}

// ── HTTP ──────────────────────────────────────────────────────────────────────

fn fetch_html(url: &str) -> Result<String, Box<dyn std::error::Error>> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(8))
        .timeout_read(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(20))
        .build();

    let resp = agent
        .get(url)
        .set("User-Agent", "Mozilla/5.0 (compatible; NonogramFetcher/1.0)")
        .set("Accept", "text/html,application/xhtml+xml")
        .call()?;

    Ok(resp.into_string()?)
}

// ── JavaScript extraction ─────────────────────────────────────────────────────

/// Find the first valid `var d=[[...]]` in all page scripts.
///
/// "Valid" means: dimensions are reasonable (2–100 per axis) and all inner
/// arrays have the same length.  If a candidate fails validation, we keep
/// searching for the next `var d=` occurrence.
fn extract_grid(scripts: &[String]) -> Option<Vec<Vec<u32>>> {
    for script in scripts {
        let mut search_from = 0usize;
        while let Some(rel) = script[search_from..].find("var d=") {
            let abs = search_from + rel;
            if let Some(grid) = parse_2d_array(&script[abs + 6..]) {
                match validate_and_orient(grid) {
                    Ok(g) => return Some(g),
                    Err(reason) => {
                        eprintln!("[nonogram] discarding var d= candidate: {reason}");
                    }
                }
            }
            search_from = abs + 6;
        }
    }
    None
}

/// Accept a grid if both dimensions are in [2, 100].
///
/// nonograms.org sometimes stores data in column-major order (d[col][row]).
/// If the raw parse gives height > 100 but width ≤ 100, we try transposing.
fn validate_and_orient(
    grid: Vec<Vec<u32>>,
) -> Result<Vec<Vec<u32>>, String> {
    if grid.is_empty() || grid[0].is_empty() {
        return Err("empty grid".into());
    }

    let h = grid.len();
    let w = grid[0].len();

    // All rows must have the same length
    if !grid.iter().all(|r| r.len() == w) {
        return Err("ragged grid (inconsistent row lengths)".into());
    }

    // Dimensions are already sane — accept as-is
    if w >= 2 && w <= 100 && h >= 2 && h <= 100 {
        eprintln!("[nonogram] grid candidate {}×{} accepted", w, h);
        return Ok(grid);
    }

    // Try transposing (handles column-major storage)
    let (th, tw) = (w, h); // after transpose
    if tw >= 2 && tw <= 100 && th >= 2 && th <= 100 {
        eprintln!("[nonogram] transposing {}×{} → {}×{}", w, h, tw, th);
        let transposed: Vec<Vec<u32>> = (0..w)
            .map(|col| (0..h).map(|row| grid[row][col]).collect())
            .collect();
        return Ok(transposed);
    }

    Err(format!("dimensions {}×{} out of range (max 100 per axis)", w, h))
}

fn extract_palette(scripts: &[String]) -> Option<Vec<String>> {
    for s in scripts {
        if let Some(pos) = s.find("\"colors\"") {
            let after = &s[pos..];
            if let (Some(a), Some(b)) = (after.find('['), after.find(']')) {
                let arr = &after[a..=b];
                let colors: Vec<String> = arr
                    .split('"')
                    .filter(|s| s.starts_with('#') && s.len() == 7)
                    .map(|s| s.to_string())
                    .collect();
                if !colors.is_empty() { return Some(colors); }
            }
        }
    }
    None
}

// ── 2-D array parser ──────────────────────────────────────────────────────────

fn parse_2d_array(input: &str) -> Option<Vec<Vec<u32>>> {
    let input = input.trim();
    if !input.starts_with('[') { return None; }

    // Find matching outer ']'
    let mut depth = 0i32;
    let mut end   = 0usize;
    for (i, ch) in input.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 { end = i; break; }
            }
            _ => {}
        }
    }

    let mut rows: Vec<Vec<u32>> = Vec::new();
    let mut row:  Vec<u32>      = Vec::new();
    let mut num   = String::new();
    let mut in_row = false;

    for ch in input[1..end].chars() {
        match ch {
            '[' => { in_row = true; row = Vec::new(); num.clear(); }
            ']' => {
                if in_row {
                    if !num.is_empty() {
                        if let Ok(n) = num.trim().parse::<u32>() { row.push(n); }
                        num.clear();
                    }
                    rows.push(row.clone());
                    in_row = false;
                }
            }
            ',' => {
                if in_row && !num.is_empty() {
                    if let Ok(n) = num.trim().parse::<u32>() { row.push(n); }
                    num.clear();
                }
            }
            c if c.is_ascii_digit() && in_row => num.push(c),
            _ => {}
        }
    }

    if rows.is_empty() { None } else { Some(rows) }
}

// ── Clue computation ──────────────────────────────────────────────────────────

pub fn compute_row_clues(grid: &[Vec<u32>], is_bw: bool) -> Vec<Vec<ClueEntry>> {
    grid.iter().map(|row| line_clues(row, is_bw)).collect()
}

pub fn compute_col_clues(grid: &[Vec<u32>], width: usize, is_bw: bool) -> Vec<Vec<ClueEntry>> {
    (0..width).map(|c| {
        let col: Vec<u32> = grid.iter().map(|r| r[c]).collect();
        line_clues(&col, is_bw)
    }).collect()
}

fn line_clues(line: &[u32], is_bw: bool) -> Vec<ClueEntry> {
    let mut clues: Vec<ClueEntry> = Vec::new();
    let mut run = 0u32;
    let mut col = 0u32;

    for &cell in line {
        if cell == 0 {
            if run > 0 {
                clues.push(ClueEntry { count: run, color_idx: col });
                run = 0;
            }
        } else if is_bw {
            run += 1;
            col  = 1;
        } else if cell == col {
            run += 1;
        } else {
            if run > 0 {
                clues.push(ClueEntry { count: run, color_idx: col });
            }
            run = 1;
            col = cell;
        }
    }
    if run > 0 {
        clues.push(ClueEntry { count: run, color_idx: col });
    }
    if clues.is_empty() {
        clues.push(ClueEntry { count: 0, color_idx: 0 });
    }
    clues
}
