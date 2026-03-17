mod appload;
mod nonogram;
mod pdf_gen;

use std::env;
use std::sync::{Arc, Mutex};
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

    // Stores the UUID of the last successfully saved document so we can open
    // it in xochitl via D-Bus without restarting the whole process.
    let saved_uuid: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    loop {
        // Drain any pending responses from the worker (non-blocking).
        while let Ok((t, s)) = rx.try_recv() {
            eprintln!("[fetcher] forwarding type={} to frontend", t);

            if t == 1 {
                // Worker confirmed a successful save.
                // Extract the document UUID from the path and ask xochitl to
                // open it immediately — this records it in the recents list
                // and lets the user jump straight to the puzzle.
                let path = s.trim_start_matches("SAVED:");
                if let Some(uuid) = extract_uuid(path) {
                    eprintln!("[fetcher] opening document {} in xochitl", uuid);
                    open_in_xochitl(&uuid);
                    *saved_uuid.lock().unwrap() = Some(uuid);
                }
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
    // The socket is gone — the user has closed the AppLoad overlay.
    // Wait for any in-progress download to finish writing its PDF to disk
    // before we exit, so the file is always complete when xochitl reads it.
    if let Some(handle) = active_worker {
        eprintln!("[fetcher] socket closed while download in progress — waiting...");
        let _ = handle.join();
        eprintln!("[fetcher] download finished.");

        // If the download completed just as the overlay was closed we still
        // need to open the document (the main loop may have already exited
        // before the worker sent its type=1 response).
        while let Ok((t, s)) = rx.try_recv() {
            if t == 1 {
                let path = s.trim_start_matches("SAVED:");
                if let Some(uuid) = extract_uuid(path) {
                    eprintln!("[fetcher] (late) opening document {} in xochitl", uuid);
                    open_in_xochitl(&uuid);
                }
            }
        }
    }

    // No xochitl restart needed: open_in_xochitl() already asked xochitl to
    // open the document via D-Bus, which also causes it to appear in recents.
    eprintln!("[fetcher] exiting.");
}

// ── D-Bus helpers ─────────────────────────────────────────────────────────────

/// Ask xochitl to open a document by UUID using its D-Bus interface.
///
/// This works on reMarkable Paper Pro firmware without touching xochitl's
/// lifecycle — the document opens in the background and is recorded in the
/// recents list as soon as the AppLoad overlay is dismissed.
fn open_in_xochitl(uuid: &str) {
    // The interface name differs slightly across firmware versions; we try the
    // most common one first.  Failures are non-fatal: the document will still
    // appear in the library on the next normal xochitl startup.
    let result = std::process::Command::new("dbus-send")
        .args([
            "--system",
            "--type=method_call",
            "--dest=com.reMarkable.xochitl",
            "/com/reMarkable/xochitl",
            "com.reMarkable.xochitl1.openDocumentRequest",
            &format!("string:{}", uuid),
        ])
        .status();

    match result {
        Ok(s) if s.success() => {
            eprintln!("[fetcher] D-Bus openDocumentRequest succeeded");
        }
        Ok(s) => {
            eprintln!("[fetcher] D-Bus returned exit code {:?}, trying fallback", s.code());
            // Fallback: some firmware versions use a slightly different method name.
            let _ = std::process::Command::new("dbus-send")
                .args([
                    "--system",
                    "--type=method_call",
                    "--dest=com.reMarkable.xochitl",
                    "/com/reMarkable/xochitl",
                    "com.reMarkable.xochitl1.openFile",
                    &format!("string:{}", uuid),
                ])
                .spawn();
        }
        Err(e) => {
            eprintln!("[fetcher] D-Bus call failed: {} (document will appear on next startup)", e);
        }
    }
}

/// Extract the UUID stem from a full PDF path.
///
/// Example: `/home/root/.local/share/remarkable/xochitl/abc-123.pdf` → `abc-123`
fn extract_uuid(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
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
        Ok(path) => send(1, &format!("SAVED:{}", path)),
        Err(e)   => send(2, &format!("PDF error: {}", e)),
    }
}