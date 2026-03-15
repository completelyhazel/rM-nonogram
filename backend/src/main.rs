// ============================================================================
//  Nonogram Fetcher — AppLoad backend
//  Protocolo AppLoad (unix socket):
//    Cada mensaje = 4 bytes LE (len) + len bytes UTF-8 JSON
//    JSON entrante del frontend: { "type": u32, "contents": String }
//    JSON saliente al frontend:  { "type": u32, "contents": String }
//
//  Tipos de mensaje (frontend → backend):
//    0 = fetch request  (contents = JSON con parámetros)
//
//  Tipos de mensaje (backend → frontend):
//    1 = success   (contents = "SAVED:<ruta>")
//    2 = error     (contents = mensaje de error)
//    3 = progress  (contents = texto de estado)
// ============================================================================

mod appload;
mod nonogram;
mod pdf_gen;

use std::env;
use appload::AppLoadConnection;
use nonogram::FetchRequest;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("[nonogram-fetcher] ERROR: se requiere el socket path como argv[1]");
        std::process::exit(1);
    }

    let socket_path = &args[1];
    eprintln!("[nonogram-fetcher] conectando al socket: {}", socket_path);

    let mut conn = match AppLoadConnection::connect(socket_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[nonogram-fetcher] no se pudo conectar: {}", e);
            std::process::exit(1);
        }
    };

    eprintln!("[nonogram-fetcher] conectado, esperando mensajes…");

    loop {
        let msg = match conn.read_message() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[nonogram-fetcher] error leyendo mensaje: {}", e);
                break;
            }
        };

        eprintln!("[nonogram-fetcher] mensaje recibido tipo={} contents={}", msg.msg_type, msg.contents);

        if msg.msg_type == 0 {
            // Fetch request
            let req: FetchRequest = match serde_json::from_str(&msg.contents) {
                Ok(r) => r,
                Err(e) => {
                    let _ = conn.send_message(2, &format!("JSON inválido: {}", e));
                    continue;
                }
            };

            handle_fetch_request(&mut conn, req);
        }
    }

    eprintln!("[nonogram-fetcher] cerrando");
}

fn handle_fetch_request(conn: &mut AppLoadConnection, req: FetchRequest) {
    let _ = conn.send_message(3, "Buscando nonogramas disponibles…");

    // 1. Buscar una lista de IDs que cumplan los criterios
    let ids = match nonogram::search_nonograms(&req) {
        Ok(ids) if ids.is_empty() => {
            let _ = conn.send_message(2, "No se encontraron nonogramas con esos criterios. Prueba con otros filtros.");
            return;
        }
        Ok(ids) => ids,
        Err(e) => {
            let _ = conn.send_message(2, &format!("Error buscando nonogramas: {}", e));
            return;
        }
    };

    eprintln!("[nonogram-fetcher] encontrados {} nonogramas candidatos", ids.len());

    // 2. Elegir uno al azar
    let idx = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize) % ids.len();
    let chosen_id = ids[idx];

    let _ = conn.send_message(3, &format!("Descargando nonograma #{chosen_id}…"));

    // 3. Descargar los datos del nonograma
    let puzzle = match nonogram::fetch_nonogram(chosen_id, req.type_bw) {
        Ok(p) => p,
        Err(e) => {
            let _ = conn.send_message(2, &format!("Error descargando el puzzle: {}", e));
            return;
        }
    };

    let _ = conn.send_message(3, &format!("Generando PDF para «{}»…", puzzle.title));

    // 4. Generar el PDF
    let output_dir  = "/home/root/.local/share/remarkable/xochitl";
    let output_path = match pdf_gen::generate_pdf(&puzzle, output_dir) {
        Ok(p) => p,
        Err(e) => {
            let _ = conn.send_message(2, &format!("Error generando PDF: {}", e));
            return;
        }
    };

    eprintln!("[nonogram-fetcher] PDF guardado en {}", output_path);
    let _ = conn.send_message(1, &format!("SAVED:{}", output_path));
}
