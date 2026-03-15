// ============================================================================
//  nonogram.rs — Scraper de nonograms.org
//
//  Endpoints utilizados:
//    Listado B&W:  https://www.nonograms.org/nonograms/p/<page>
//    Listado Color: https://www.nonograms.org/nonograms2/p/<page>
//    Puzzle B&W:   https://www.nonograms.org/nonograms/i/<id>
//    Puzzle Color: https://www.nonograms.org/nonograms2/i/<id>
//
//  Los datos del puzzle están embebidos en el HTML como un bloque <script>
//  con variables JavaScript tipo:
//    var d=[[0,1,1,0,...],[...],...];   <- solución (filas × columnas)
//    var s={...};                       <- metadatos (title, etc.)
// ============================================================================

use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

/// Parámetros de búsqueda elegidos por el usuario en el frontend
#[derive(Deserialize, Debug, Clone)]
pub struct FetchRequest {
    pub type_bw:    bool,
    pub size:       String,
    pub difficulty: u32,
}

// ─── Estructuras públicas ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct NonogramPuzzle {
    pub id:           u32,
    pub title:        String,
    pub is_bw:        bool,
    /// Solución: grid[fila][col] = 0 (vacío) o color (1..N, donde 1=negro en B&W)
    pub grid:         Vec<Vec<u32>>,
    /// Pistas de columnas: cada elemento es la lista de números de esa columna
    pub col_clues:    Vec<Vec<ClueEntry>>,
    /// Pistas de filas
    pub row_clues:    Vec<Vec<ClueEntry>>,
    /// Paleta de colores en hex (solo para puzzles color; B&W usa ["#000000"])
    pub palette:      Vec<String>,
    pub width:        usize,
    pub height:       usize,
}

#[derive(Debug, Clone)]
pub struct ClueEntry {
    pub count: u32,
    /// Solo relevante en puzzles color
    pub color_idx: u32,
}

// ─── Búsqueda ────────────────────────────────────────────────────────────────

/// Devuelve una lista de IDs candidatos que cumplen los filtros.
/// Hace scraping de la primera página de resultados filtrada.
pub fn search_nonograms(req: &FetchRequest) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    // Construir URL de búsqueda con filtros de tamaño
    // nonograms.org acepta filtros por tamaño en la URL de listado:
    // /nonograms?d=<size>x<size>   (no siempre documentado, pero funciona)
    let base = if req.type_bw {
        "https://www.nonograms.org/nonograms"
    } else {
        "https://www.nonograms.org/nonograms2"
    };

    // Mapear size a parámetro de dimensión
    let dim_filter = match req.size.as_str() {
        "5"  => "?d=5x5",
        "10" => "?d=10x10",
        "15" => "?d=15x15",
        "20" => "?d=20x20",
        "25" => "?d=25x25",
        _    => "",
    };

    // Elegir una página aleatoria entre 1 y 5 para variedad
    let page_seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let page = (page_seed % 5) + 1;

    let url = if dim_filter.is_empty() {
        format!("{}/p/{}", base, page)
    } else {
        format!("{}{}&p={}", base, dim_filter, page)
    };

    eprintln!("[nonogram] buscando en: {}", url);

    let body = fetch_html(&url)?;
    let ids  = parse_puzzle_ids_from_list(&body, req.type_bw, req.difficulty)?;

    Ok(ids)
}

/// Descarga y parsea un puzzle concreto por ID.
pub fn fetch_nonogram(id: u32, is_bw: bool) -> Result<NonogramPuzzle, Box<dyn std::error::Error>> {
    let url = if is_bw {
        format!("https://www.nonograms.org/nonograms/i/{}", id)
    } else {
        format!("https://www.nonograms.org/nonograms2/i/{}", id)
    };

    eprintln!("[nonogram] descargando puzzle: {}", url);
    let body = fetch_html(&url)?;
    parse_puzzle_page(&body, id, is_bw)
}

// ─── HTTP ────────────────────────────────────────────────────────────────────

fn fetch_html(url: &str) -> Result<String, Box<dyn std::error::Error>> {
    let response = ureq::get(url)
        .set("User-Agent", "Mozilla/5.0 (compatible; NonogramFetcher/1.0)")
        .set("Accept", "text/html,application/xhtml+xml")
        .call()?;
    Ok(response.into_string()?)
}

// ─── Parseo de listado ───────────────────────────────────────────────────────

fn parse_puzzle_ids_from_list(
    html:       &str,
    _is_bw:     bool,
    difficulty: u32,
) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let doc      = Html::parse_document(html);
    // Los links a puzzles tienen la forma /nonograms/i/<id> o /nonograms2/i/<id>
    let sel_link = Selector::parse("a[href*='/i/']").unwrap();
    let sel_stars = Selector::parse("td.rating").unwrap(); // rating cell si existe

    let mut ids = Vec::new();

    // Recopilar todos los links que apuntan a puzzles individuales
    for el in doc.select(&sel_link) {
        if let Some(href) = el.value().attr("href") {
            if let Some(id_str) = href.split("/i/").nth(1) {
                let id_clean = id_str.split('?').next().unwrap_or("").trim();
                if let Ok(id) = id_clean.parse::<u32>() {
                    ids.push(id);
                }
            }
        }
    }

    // Eliminar duplicados
    ids.dedup();

    // Filtro por dificultad (aproximado, basado en la posición/rating si disponible)
    // Cuando difficulty==0 no filtramos nada
    if difficulty == 0 || ids.is_empty() {
        return Ok(ids);
    }

    // Sin datos de rating en el listado podemos simplemente devolver todos
    // y dejar que el usuario pruebe suerte. El filtro real requeriría
    // parsear cada página individual, lo cual sería demasiado lento.
    // Una heurística: IDs más bajos tienden a ser puzzles más simples/antiguos.
    let filtered: Vec<u32> = match difficulty {
        1 => ids.into_iter().filter(|&id| id < 5000).collect(),
        2 => ids.into_iter().filter(|&id| id >= 5000 && id < 30000).collect(),
        3 => ids.into_iter().filter(|&id| id >= 30000).collect(),
        _ => ids,
    };

    // Si el filtro dejó vacío, devolvemos la lista completa sin filtrar
    // para no dejar al usuario sin resultados
    if filtered.is_empty() {
        return search_nonograms_unfiltered(html);
    }

    Ok(filtered)
}

fn search_nonograms_unfiltered(html: &str) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let doc      = Html::parse_document(html);
    let sel_link = Selector::parse("a[href*='/i/']").unwrap();
    let mut ids  = Vec::new();

    for el in doc.select(&sel_link) {
        if let Some(href) = el.value().attr("href") {
            if let Some(id_str) = href.split("/i/").nth(1) {
                let id_clean = id_str.split('?').next().unwrap_or("").trim();
                if let Ok(id) = id_clean.parse::<u32>() {
                    ids.push(id);
                }
            }
        }
    }
    ids.dedup();
    Ok(ids)
}

// ─── Parseo de página de puzzle ──────────────────────────────────────────────

fn parse_puzzle_page(
    html:  &str,
    id:    u32,
    is_bw: bool,
) -> Result<NonogramPuzzle, Box<dyn std::error::Error>> {
    // Extraer el título
    let doc       = Html::parse_document(html);
    let sel_title = Selector::parse("h1").unwrap();
    let title = doc.select(&sel_title)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .unwrap_or_else(|| format!("Nonogram #{}", id));

    // Los datos de la solución están en un bloque <script> con la variable `var d=`
    // Ejemplo: var d=[[0,1,1,0,1],[1,0,0,1,1],...];
    // En puzzles de color, cada valor es un índice de color (0=vacío, 1..N=colores)
    let scripts = extract_scripts(html);

    let grid = extract_grid_from_scripts(&scripts)
        .ok_or("No se encontró la variable 'd' con la solución del puzzle")?;

    if grid.is_empty() || grid[0].is_empty() {
        return Err("Grid vacío extraído del puzzle".into());
    }

    let height = grid.len();
    let width  = grid[0].len();

    // Extraer paleta de colores (para puzzles color)
    // Busca: var s={..., "colors":["#rrggbb",...], ...}
    let palette = if is_bw {
        vec!["#000000".to_string()]
    } else {
        extract_palette_from_scripts(&scripts)
            .unwrap_or_else(|| vec!["#000000".to_string()])
    };

    // Calcular pistas a partir del grid
    let row_clues = compute_row_clues(&grid, is_bw);
    let col_clues = compute_col_clues(&grid, width, is_bw);

    Ok(NonogramPuzzle {
        id,
        title,
        is_bw,
        grid,
        col_clues,
        row_clues,
        palette,
        width,
        height,
    })
}

// ─── Extracción de datos JS ──────────────────────────────────────────────────

fn extract_scripts(html: &str) -> Vec<String> {
    let doc     = Html::parse_document(html);
    let sel_scr = Selector::parse("script").unwrap();
    doc.select(&sel_scr)
        .map(|e| e.text().collect::<String>())
        .collect()
}

/// Extrae la variable `var d=[[...],[...],...];` de los scripts
fn extract_grid_from_scripts(scripts: &[String]) -> Option<Vec<Vec<u32>>> {
    for script in scripts {
        if let Some(start) = script.find("var d=") {
            let after = &script[start + 6..]; // salta "var d="
            if let Some(grid) = parse_js_2d_array(after) {
                return Some(grid);
            }
        }
    }
    None
}

/// Extrae la paleta de colores de `var s={..., colors:[...], ...}`
fn extract_palette_from_scripts(scripts: &[String]) -> Option<Vec<String>> {
    for script in scripts {
        // Busca algo como: "colors":["#ff0000","#00ff00",...]
        if let Some(pos) = script.find("\"colors\"") {
            let after = &script[pos..];
            if let Some(arr_start) = after.find('[') {
                if let Some(arr_end) = after.find(']') {
                    let arr_str = &after[arr_start..=arr_end];
                    let colors: Vec<String> = arr_str
                        .split('"')
                        .filter(|s| s.starts_with('#') && s.len() == 7)
                        .map(|s| s.to_string())
                        .collect();
                    if !colors.is_empty() {
                        return Some(colors);
                    }
                }
            }
        }
    }
    None
}

/// Parser minimal de array 2D JS: [[0,1,...],[0,...],...]
fn parse_js_2d_array(input: &str) -> Option<Vec<Vec<u32>>> {
    let input = input.trim();
    if !input.starts_with('[') {
        return None;
    }

    // Encontrar el cierre del array externo
    let mut depth  = 0i32;
    let mut end    = 0;
    for (i, ch) in input.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }

    let array_str = &input[1..end]; // contenido entre los corchetes externos

    let mut rows = Vec::new();
    let mut current_row: Vec<u32> = Vec::new();
    let mut in_inner   = false;
    let mut num_buf    = String::new();

    for ch in array_str.chars() {
        match ch {
            '[' => {
                in_inner    = true;
                current_row = Vec::new();
                num_buf.clear();
            }
            ']' => {
                if in_inner {
                    if !num_buf.is_empty() {
                        if let Ok(n) = num_buf.trim().parse::<u32>() {
                            current_row.push(n);
                        }
                        num_buf.clear();
                    }
                    rows.push(current_row.clone());
                    in_inner = false;
                }
            }
            ',' => {
                if in_inner && !num_buf.is_empty() {
                    if let Ok(n) = num_buf.trim().parse::<u32>() {
                        current_row.push(n);
                    }
                    num_buf.clear();
                }
            }
            ' ' | '\n' | '\r' | '\t' => {}
            c if c.is_ascii_digit() && in_inner => {
                num_buf.push(c);
            }
            _ => {}
        }
    }

    if rows.is_empty() { None } else { Some(rows) }
}

// ─── Cálculo de pistas ───────────────────────────────────────────────────────

pub fn compute_row_clues(grid: &[Vec<u32>], is_bw: bool) -> Vec<Vec<ClueEntry>> {
    grid.iter().map(|row| line_to_clues(row, is_bw)).collect()
}

pub fn compute_col_clues(grid: &[Vec<u32>], width: usize, is_bw: bool) -> Vec<Vec<ClueEntry>> {
    (0..width).map(|col| {
        let column: Vec<u32> = grid.iter().map(|row| row[col]).collect();
        line_to_clues(&column, is_bw)
    }).collect()
}

fn line_to_clues(line: &[u32], is_bw: bool) -> Vec<ClueEntry> {
    let mut clues: Vec<ClueEntry> = Vec::new();
    let mut run_len:   u32 = 0;
    let mut run_color: u32 = 0;

    for &cell in line {
        if cell == 0 {
            if run_len > 0 {
                clues.push(ClueEntry { count: run_len, color_idx: run_color });
                run_len = 0;
            }
        } else {
            if is_bw {
                run_len += 1;
                run_color = 1;
            } else {
                // En color: runs del mismo color se agrupan; cambio de color = nueva entrada
                if cell == run_color {
                    run_len += 1;
                } else {
                    if run_len > 0 {
                        clues.push(ClueEntry { count: run_len, color_idx: run_color });
                    }
                    run_len   = 1;
                    run_color = cell;
                }
            }
        }
    }

    if run_len > 0 {
        clues.push(ClueEntry { count: run_len, color_idx: run_color });
    }

    if clues.is_empty() {
        clues.push(ClueEntry { count: 0, color_idx: 0 });
    }

    clues
}
