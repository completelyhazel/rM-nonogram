mod appload;
mod nonogram;
mod pdf_gen;

use std::env;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
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

    // Channel for worker threads to send responses back to the main loop.
    let (tx, rx) = mpsc::channel::<(u32, String)>();

    // Handle to the currently active download worker (at most one at a time).
    let mut active_worker: Option<JoinHandle<()>> = None;

    // Set to true by the worker when a PDF is successfully saved.
    // Read after the socket closes to decide whether to restart xochitl.
    let pdf_was_saved = Arc::new(AtomicBool::new(false));

    loop {
        // Drain any pending responses from the worker (non-blocking).
        while let Ok((t, s)) = rx.try_recv() {
            eprintln!("[fetcher] forwarding type={} to frontend", t);
            if t == 1 {
                // Worker confirmed a successful save — flag it so we can
                // restart xochitl once the user has closed the app.
                pdf_was_saved.store(true, Ordering::SeqCst);
            }
            let _ = conn.send_message(t, &s);
        }

        // Wait for a message from the frontend (blocks up to SO_RCVTIMEO = 300 ms).
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

                            // Wait for any previous fetch before starting a new one.
                            if let Some(prev) = active_worker.take() {
                                let _ = prev.join();
                            }

                            let tx2  = tx.clone();
                            let flag = Arc::clone(&pdf_was_saved);
                            active_worker = Some(std::thread::spawn(move || {
                                handle_fetch(tx2, req, flag);
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
                // msg_type 99 / 43 / 1 = internal AppLoad handshake — ignore
            }
            Err(e) => {
                let s = e.to_string();
                if s == "timeout" {
                    continue;
                }
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

    // ── Graceful shutdown ─────────────────────────────────────────────────────
    //
    // The socket is gone — the user has left AppLoad.
    // Wait for any active download to finish writing its PDF before we exit
    // or restart xochitl, so the file is guaranteed to be complete on disk.
    if let Some(handle) = active_worker {
        eprintln!("[fetcher] socket closed while download in progress — waiting...");
        let _ = handle.join();
        eprintln!("[fetcher] download finished.");
    }

    // Restart xochitl only if we actually saved a new document.
    // By this point AppLoad has already terminated our session, so restarting
    // xochitl is safe and is the correct way to force it to index new files.
    if pdf_was_saved.load(Ordering::SeqCst) {
        eprintln!("[fetcher] restarting xochitl to pick up new document...");
        let _ = std::process::Command::new("systemctl")
            .args(["restart", "xochitl"])
            .spawn();
    }

    eprintln!("[fetcher] exiting.");
}

// ─────────────────────────────────────────────────────────────────────────────

fn handle_fetch(
    tx:       mpsc::Sender<(u32, String)>,
    req:      FetchRequest,
    pdf_flag: Arc<AtomicBool>,
) {
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

    // Pick a pseudo-random starting puzzle, retry up to 5 on failure.
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
            None => { send(2, "Could not download any puzzle after multiple attempts."); return; }
        }
    };

    send(3, &format!("Generating PDF for '{}'...", info.title));

    let dir = "/home/root/.local/share/remarkable/xochitl";
    match pdf_gen::generate_pdf(&info, dir) {
        Ok(path) => {
            // Set the flag BEFORE sending type=1 so the main loop sees it
            // even if the frontend closes immediately after receiving the message.
            pdf_flag.store(true, Ordering::SeqCst);
            send(1, &format!("SAVED:{}", path));
        }
        Err(e) => send(2, &format!("PDF error: {}", e)),
    }
}
