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

    // Compartir la conexión entre el thread lector y los workers HTTP
    let conn = Arc::new(Mutex::new(conn));

    loop {
        // Leer mensaje bloqueante (en el thread principal)
        let msg = {
            let mut c = conn.lock().unwrap();
            match c.read_message() {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("[nonogram-fetcher] error leyendo socket: {}", e);
                    break;
                }
            }
        };

        eprintln!("[nonogram-fetcher] mensaje recibido tipo={} contents={}", msg.msg_type, msg.contents);

        if msg.msg_type == 0 {
            let req: FetchRequest = match serde_json::from_str(&msg.contents) {
                Ok(r) => r,
                Err(e) => {
                    let mut c = conn.lock().unwrap();
                    let _ = c.send_message(2, &format!("JSON inválido: {}", e));
                    continue;
                }
            };

            // Lanzar HTTP en thread separado, conn compartido via Arc<Mutex>
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
    eprintln!("[nonogram-fetcher] iniciando búsqueda, type_bw={} size={}", req.type_bw, req.size);

    let ids = match nonogram::search_nonograms(&req) {
        Ok(ids) if ids.is_empty() => {
            send(2, "No se encontraron nonogramas con esos criterios.");
            return;
        }
        Ok(ids) => ids,
        Err(e) => {
            eprintln!("[nonogram-fetcher] error búsqueda: {}", e);
            send(2, &format!("Error buscando nonogramas: {}", e));
            return;
        }
    };

    eprintln!("[nonogram-fetcher] encontrados {} candidatos", ids.len());

    let idx = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize) % ids.len();
    let chosen_id = ids[idx];

    send(3, &format!("Descargando nonograma #{}…", chosen_id));
    eprintln!("[nonogram-fetcher] descargando puzzle #{}", chosen_id);

    let puzzle = match nonogram::fetch_nonogram(chosen_id, req.type_bw) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[nonogram-fetcher] error descargando: {}", e);
            send(2, &format!("Error descargando el puzzle: {}", e));
            return;
        }
    };

    send(3, &format!("Generando PDF «{}»…", puzzle.title));
    eprintln!("[nonogram-fetcher] generando PDF");

    let output_dir = "/home/root/.local/share/remarkable/xochitl";
    match pdf_gen::generate_pdf(&puzzle, output_dir) {
        Ok(path) => {
            eprintln!("[nonogram-fetcher] PDF guardado: {}", path);
            send(1, &format!("SAVED:{}", path));
        }
        Err(e) => {
            eprintln!("[nonogram-fetcher] error PDF: {}", e);
            send(2, &format!("Error generando PDF: {}", e));
        }
    }
}
