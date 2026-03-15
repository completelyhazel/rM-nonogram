mod appload;
mod nonogram;
mod pdf_gen;

use std::env;
use std::sync::{Arc, Mutex};
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

    let conn = match AppLoadConnection::connect(socket_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[nonogram-fetcher] no se pudo conectar: {}", e);
            std::process::exit(1);
        }
    };

    eprintln!("[nonogram-fetcher] conectado, esperando mensajes…");
    let conn = Arc::new(Mutex::new(conn));

    loop {
        let msg = {
            let mut c = conn.lock().unwrap();
            match c.read_message() {
                Ok(m) => m,
                Err(e) => {
                    let s = e.to_string();
                    // Mensajes binarios de handshake de AppLoad → ignorar y seguir
                    if s.contains("expected value") || s.contains("invalid") || s.contains("trailing") {
                        eprintln!("[nonogram-fetcher] mensaje no-JSON ignorado: {}", s);
                        continue;
                    }
                    // Socket cerrado o error real → salir
                    eprintln!("[nonogram-fetcher] error de socket: {}", s);
                    break;
                }
            }
        };

        eprintln!("[nonogram-fetcher] mensaje tipo={} contents={}", msg.msg_type, msg.contents);

        if msg.msg_type == 0 {
            let req: FetchRequest = match serde_json::from_str(&msg.contents) {
                Ok(r) => r,
                Err(e) => {
                    let mut c = conn.lock().unwrap();
                    let _ = c.send_message(2, &format!("JSON inválido: {}", e));
                    continue;
                }
            };

            let conn2 = Arc::clone(&conn);
            std::thread::spawn(move || {
                handle_fetch_request(conn2, req);
            });
        }
    }
}

fn handle_fetch_request(conn: Arc<Mutex<AppLoadConnection>>, req: FetchRequest) {
    let send = |t: u32, s: &str| {
        if let Ok(mut c) = conn.lock() {
            let _ = c.send_message(t, s);
        }
    };

    send(3, "Buscando nonogramas disponibles…");
    eprintln!("[nonogram-fetcher] búsqueda: type_bw={} size={} diff={}", req.type_bw, req.size, req.difficulty);

    let ids = match nonogram::search_nonograms(&req) {
        Ok(ids) if ids.is_empty() => { send(2, "No se encontraron nonogramas."); return; }
        Ok(ids) => ids,
        Err(e) => { eprintln!("[nonogram-fetcher] error búsqueda: {}", e); send(2, &format!("Error buscando: {}", e)); return; }
    };

    eprintln!("[nonogram-fetcher] {} candidatos encontrados", ids.len());

    let idx = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize) % ids.len();
    let id = ids[idx];

    send(3, &format!("Descargando nonograma #{}…", id));
    eprintln!("[nonogram-fetcher] descargando #{}", id);

    let puzzle = match nonogram::fetch_nonogram(id, req.type_bw) {
        Ok(p) => p,
        Err(e) => { eprintln!("[nonogram-fetcher] error descarga: {}", e); send(2, &format!("Error descargando: {}", e)); return; }
    };

    send(3, &format!("Generando PDF «{}»…", puzzle.title));
    eprintln!("[nonogram-fetcher] generando PDF para «{}»", puzzle.title);

    let output_dir = "/home/root/.local/share/remarkable/xochitl";
    match pdf_gen::generate_pdf(&puzzle, output_dir) {
        Ok(path) => { eprintln!("[nonogram-fetcher] guardado: {}", path); send(1, &format!("SAVED:{}", path)); }
        Err(e)   => { eprintln!("[nonogram-fetcher] error PDF: {}", e); send(2, &format!("Error PDF: {}", e)); }
    }
}
