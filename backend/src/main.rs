mod appload;
mod nonogram;
mod pdf_gen;

use std::env;
use std::sync::mpsc;
use appload::AppLoadConnection;
use nonogram::FetchRequest;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("[nonogram-fetcher] ERROR: se requiere el socket path como argv[1]");
        std::process::exit(1);
    }

    let socket_path = args[1].clone();
    eprintln!("[nonogram-fetcher] conectando al socket: {}", socket_path);

    let mut conn = match AppLoadConnection::connect(&socket_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[nonogram-fetcher] no se pudo conectar: {}", e);
            std::process::exit(1);
        }
    };

    eprintln!("[nonogram-fetcher] conectado, esperando mensajes…");

    // Canal para recibir respuestas del worker thread de vuelta al loop principal
    let (tx, rx) = mpsc::channel::<(u32, String)>();

    loop {
        // Comprobar si hay respuestas pendientes del worker (no bloqueante)
        while let Ok((msg_type, contents)) = rx.try_recv() {
            let _ = conn.send_message(msg_type, &contents);
        }

        // Leer siguiente mensaje del frontend (con timeout pequeño para no bloquear)
        match conn.try_read_message() {
            Ok(Some(msg)) => {
                eprintln!("[nonogram-fetcher] mensaje recibido tipo={}", msg.msg_type);
                if msg.msg_type == 0 {
                    let req: FetchRequest = match serde_json::from_str(&msg.contents) {
                        Ok(r) => r,
                        Err(e) => {
                            let _ = conn.send_message(2, &format!("JSON inválido: {}", e));
                            continue;
                        }
                    };

                    // Lanzar en thread separado para no bloquear el loop
                    let tx2 = tx.clone();
                    std::thread::spawn(move || {
                        handle_fetch_request(tx2, req);
                    });
                }
            }
            Ok(None) => {
                // No hay mensaje todavía, esperar un poco
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                eprintln!("[nonogram-fetcher] error leyendo socket: {}", e);
                break;
            }
        }
    }
}

fn handle_fetch_request(tx: mpsc::Sender<(u32, String)>, req: FetchRequest) {
    let send = |t: u32, s: &str| { let _ = tx.send((t, s.to_string())); };

    send(3, "Buscando nonogramas disponibles…");

    let ids = match nonogram::search_nonograms(&req) {
        Ok(ids) if ids.is_empty() => {
            send(2, "No se encontraron nonogramas con esos criterios.");
            return;
        }
        Ok(ids) => ids,
        Err(e) => {
            send(2, &format!("Error buscando nonogramas: {}", e));
            return;
        }
    };

    let idx = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize) % ids.len();
    let chosen_id = ids[idx];

    send(3, &format!("Descargando nonograma #{}…", chosen_id));

    let puzzle = match nonogram::fetch_nonogram(chosen_id, req.type_bw) {
        Ok(p) => p,
        Err(e) => {
            send(2, &format!("Error descargando el puzzle: {}", e));
            return;
        }
    };

    send(3, &format!("Generando PDF para «{}»…", puzzle.title));

    let output_dir = "/home/root/.local/share/remarkable/xochitl";
    match pdf_gen::generate_pdf(&puzzle, output_dir) {
        Ok(path) => send(1, &format!("SAVED:{}", path)),
        Err(e)   => send(2, &format!("Error generando PDF: {}", e)),
    }
}
