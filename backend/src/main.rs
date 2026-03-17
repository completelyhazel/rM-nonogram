mod appload;
mod nonogram;
mod pdf_gen;

use std::env;
use std::sync::mpsc;
use std::thread::JoinHandle;
use appload::AppLoadConnection;
use nonogram::FetchRequest;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("[fetcher] ERROR: socket path required as argv[1]");
        std::process::exit(1);
    }

    let socket_path = &args[1];
    eprintln!("[fetcher] connecting to socket: {}", socket_path);

    let mut conn = match AppLoadConnection::connect(socket_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[fetcher] connection failed: {}", e);
            std::process::exit(1);
        }
    };

    eprintln!("[fetcher] connected, waiting for messages...");

    let (tx, rx) = mpsc::channel::<(u32, String)>();
    let mut active_worker: Option<JoinHandle<()>> = None;

    loop {
        // Drain any pending responses from the worker (non-blocking).
        while let Ok((t, s)) = rx.try_recv() {
            eprintln!("[fetcher] forwarding type={} to frontend", t);
            let _ = conn.send_message(t, &s);
        }

        // Block up to SO_RCVTIMEO (300 ms) waiting for a frontend message.
        match conn.read_message() {
            Ok(msg) => {
                eprintln!("[fetcher] received type={}", msg.msg_type);

                if msg.msg_type == 0 {
                    match serde_json::from_str::<FetchRequest>(&msg.contents) {
                        Ok(req) => {
                            eprintln!(
                                "[fetcher] fetch request: bw={} size={} difficulty={}",
                                req.type_bw, req.size, req.difficulty
                            );
                            if let Some(prev) = active_worker.take() {
                                let _ = prev.join();
                            }
                            let tx2 = tx.clone();
                            active_worker = Some(std::thread::spawn(move || {
                                handle_fetch(tx2, req);
                            }));
                        }
                        Err(e) => {
                            eprintln!("[fetcher] failed to parse request: {}", e);
                            let _ = conn.send_message(
                                2,
                                &format!("Failed to parse request: {}", e),
                            );
                        }
                    }
                }
                // type 43 = AppLoad handshake ping — ignore silently
            }
            Err(e) => {
                let s = e.to_string();
                if s == "timeout" { continue; }
                if s.contains("expected value")
                    || s.contains("invalid type")
                    || s.contains("trailing")
                {
                    continue;
                }
                eprintln!("[fetcher] socket closed or error: {}", s);
                break;
            }
        }
    }

    // Wait for any in-progress download (the worker restarts xochitl itself,
    // but if the user closed the overlay before the download finished we still
    // want to let it complete and trigger the restart).
    if let Some(handle) = active_worker {
        eprintln!("[fetcher] download in progress — waiting...");
        let _ = handle.join();
    }

    eprintln!("[fetcher] exiting.");
}

// ─────────────────────────────────────────────────────────────────────────────

fn handle_fetch(tx: mpsc::Sender<(u32, String)>, req: FetchRequest) {
    let send = |t: u32, s: &str| {
        eprintln!("[worker] type={}: {}", t, s);
        let _ = tx.send((t, s.to_string()));
    };

    send(3, "Searching for nonograms...");

    let ids = match nonogram::search_nonograms(&req) {
        Ok(v) if v.is_empty() => { send(2, "No puzzles found."); return; }
        Ok(v)  => v,
        Err(e) => { send(2, &format!("Search error: {}", e)); return; }
    };

    eprintln!("[worker] {} puzzle IDs found", ids.len());

    let base = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize)
        % ids.len();

    const MAX_ATTEMPTS: usize = 5;
    let attempts = MAX_ATTEMPTS.min(ids.len());

    let info = {
        let mut result = None;
        for attempt in 0..attempts {
            let id = ids[(base + attempt) % ids.len()];
            send(3, &format!("Downloading puzzle #{}...", id));
            match nonogram::fetch_nonogram(id, req.type_bw) {
                Ok(p) => { result = Some(p); break; }
                Err(e) => {
                    eprintln!(
                        "[worker] puzzle #{} failed (attempt {}/{}): {}",
                        id, attempt + 1, attempts, e
                    );
                    if attempt + 1 < attempts {
                        send(3, &format!("Puzzle #{} unavailable, trying another...", id));
                    }
                }
            }
        }
        match result {
            Some(p) => p,
            None => {
                send(2, "Could not download any puzzle after multiple attempts.");
                return;
            }
        }
    };

    send(3, &format!("Generating PDF for '{}'...", info.title));

    let dir = "/home/root/.local/share/remarkable/xochitl";
    match pdf_gen::generate_pdf(&info, dir) {
        Ok(path) => {
            send(1, &format!("SAVED:{}", path));

            // Give the main loop time to forward the type=1 message to the
            // frontend so the success screen is visible before xochitl dies.
            std::thread::sleep(std::time::Duration::from_secs(2));

            eprintln!("[worker] restarting xochitl to index new document...");
            let status = std::process::Command::new("systemctl")
                .args(["restart", "xochitl"])
                .status();
            eprintln!("[worker] systemctl restart: {:?}", status);
        }
        Err(e) => send(2, &format!("PDF error: {}", e)),
    }
}