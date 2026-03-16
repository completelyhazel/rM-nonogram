mod appload;
mod nonogram;
mod pdf_gen;

use std::env;
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
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

    // Canal para que los workers manden respuestas al hilo principal
    let (tx, rx) = mpsc::channel::<(u32, String)>();

    loop {
        // Enviar respuestas pendientes de workers (no bloqueante)
        while let Ok((t, s)) = rx.try_recv() {
            eprintln!("[nonogram-fetcher] enviando tipo={} al frontend", t);
            let _ = conn.send_message(t, &s);
        }

        // Leer mensaje del frontend (bloqueante pero con timeout interno)
        match conn.read_message() {
            Ok(msg) => {
                eprintln!("[nonogram-fetcher] mensaje tipo={}", msg.msg_type);
                if msg.msg_type == 0 {
                    match serde_json::from_str::<FetchRequest>(&msg.contents) {
                        Ok(req) => {
                            eprintln!("[nonogram-fetcher] fetch request: bw={} size={} diff={}", req.type_bw, req.size, req.difficulty);
                            let tx2 = tx.clone();
                            std::thread::spawn(move || handle_fetch(tx2, req));
                        }
                        Err(e) => {
                            eprintln!("[nonogram-fetcher] error parseando request: {}", e);
                            let _ = conn.send_message(2, &format!("Error parseando request: {}", e));
                        }
                    }
                }
            }
            Err(e) => {
                let s = e.to_string();
                if s.contains("expected value") || s.contains("invalid type")
                    || s.contains("trailing") || s == "timeout" {
                    // timeout o mensaje no-JSON = continuar el loop
                    continue;
                }
                eprintln!("[nonogram-fetcher] error socket: {}", s);
                break;
            }
        }
    }
}

fn handle_fetch(tx: mpsc::Sender<(u32, String)>, req: FetchRequest) {
    let send = |t: u32, s: &str| {
        eprintln!("[worker] enviando tipo={}: {}", t, s);
        let _ = tx.send((t, s.to_string()));
    };

    send(3, "Buscando nonogramas…");
    eprintln!("[worker] iniciando búsqueda bw={} size={}", req.type_bw, req.size);

    let ids = match nonogram::search_nonograms(&req) {
        Ok(v) if v.is_empty() => { send(2, "No se encontraron nonogramas."); return; }
        Ok(v) => v,
        Err(e) => { send(2, &format!("Error búsqueda: {}", e)); return; }
    };

    eprintln!("[worker] {} IDs encontrados", ids.len());
    let idx = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize) % ids.len();
    let id = ids[idx];

    send(3, &format!("Descargando #{id}…"));
    let puzzle = match nonogram::fetch_nonogram(id, req.type_bw) {
        Ok(p) => p,
        Err(e) => { send(2, &format!("Error descarga: {}", e)); return; }
    };

    send(3, &format!("Generando PDF «{}»…", puzzle.title));
    let dir = "/home/root/.local/share/remarkable/xochitl";
    match pdf_gen::generate_pdf(&puzzle, dir) {
        Ok(path) => send(1, &format!("SAVED:{}", path)),
        Err(e)   => send(2, &format!("Error PDF: {}", e)),
    }
}
