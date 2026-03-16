mod appload;
mod nonogram;
mod pdf_gen;

use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use appload::AppLoadConnection;
use nonogram::FetchRequest;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("[nonogram-fetcher] ERROR: socket path required as argv[1]");
        std::process::exit(1);
    }

    let socket_path = &args[1];
    eprintln!("[nonogram-fetcher] connecting to socket: {}", socket_path);

    let mut conn = match AppLoadConnection::connect(socket_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[nonogram-fetcher] could not connect: {}", e);
            std::process::exit(1);
        }
    };

    eprintln!("[nonogram-fetcher] connected, waiting for messages…");

    // Channel for worker threads to send responses back to the main thread.
    let (tx, rx) = mpsc::channel::<(u32, String)>();

    // Counter of worker threads currently running.
    // When the socket disconnects we wait until this reaches zero before exiting,
    // so that in-flight downloads and PDF writes are never abandoned mid-way.
    let active_workers = Arc::new(AtomicUsize::new(0));

    loop {
        // --- Flush any pending responses from workers (non-blocking) ---
        while let Ok((msg_type, body)) = rx.try_recv() {
            eprintln!("[nonogram-fetcher] sending type={} to frontend", msg_type);
            if conn.send_message(msg_type, &body).is_err() {
                // Socket already gone; ignore send errors here — the PDF was
                // already written by the worker before this message was queued.
                eprintln!("[nonogram-fetcher] send failed (socket closed), continuing");
            }
        }

        // --- Read next message from the frontend (blocks with an internal timeout) ---
        match conn.read_message() {
            Ok(msg) => {
                eprintln!("[nonogram-fetcher] received type={}", msg.msg_type);

                if msg.msg_type == 0 {
                    match serde_json::from_str::<FetchRequest>(&msg.contents) {
                        Ok(req) => {
                            eprintln!(
                                "[nonogram-fetcher] fetch request: bw={} size={} diff={}",
                                req.type_bw, req.size, req.difficulty
                            );
                            let tx2 = tx.clone();
                            let counter = Arc::clone(&active_workers);
                            counter.fetch_add(1, Ordering::SeqCst);
                            std::thread::spawn(move || {
                                handle_fetch(tx2, req);
                                counter.fetch_sub(1, Ordering::SeqCst);
                            });
                        }
                        Err(e) => {
                            eprintln!("[nonogram-fetcher] failed to parse request: {}", e);
                            let _ = conn.send_message(2, &format!("Error parsing request: {}", e));
                        }
                    }
                }
            }

            Err(e) => {
                let msg = e.to_string();

                // Timeout / non-JSON noise → just continue the loop
                if msg.contains("expected value")
                    || msg.contains("invalid type")
                    || msg.contains("trailing")
                    || msg == "timeout"
                {
                    continue;
                }

                // Real socket error — the frontend has closed the connection.
                // Do NOT exit immediately: wait for every in-flight worker to
                // finish so that the PDF is fully written before we die.
                eprintln!(
                    "[nonogram-fetcher] socket error: {} — waiting for {} active worker(s)…",
                    msg,
                    active_workers.load(Ordering::SeqCst)
                );

                while active_workers.load(Ordering::SeqCst) > 0 {
                    // Keep draining the response channel so workers don't block
                    // on a full channel (even though we can no longer send them
                    // to the frontend, the PDF is already on disk at this point).
                    while let Ok((t, s)) = rx.try_recv() {
                        eprintln!(
                            "[nonogram-fetcher] worker result (not forwarded, socket closed): type={} body={}",
                            t, s
                        );
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }

                eprintln!("[nonogram-fetcher] all workers finished, exiting");
                break;
            }
        }
    }
}

fn handle_fetch(tx: mpsc::Sender<(u32, String)>, req: FetchRequest) {
    let send = |t: u32, s: &str| {
        eprintln!("[worker] sending type={}: {}", t, s);
        let _ = tx.send((t, s.to_string()));
    };

    send(3, "Searching for nonograms…");
    eprintln!("[worker] starting search bw={} size={}", req.type_bw, req.size);

    let ids = match nonogram::search_nonograms(&req) {
        Ok(v) if v.is_empty() => {
            send(2, "No nonograms found.");
            return;
        }
        Ok(v) => v,
        Err(e) => {
            send(2, &format!("Search error: {}", e));
            return;
        }
    };

    eprintln!("[worker] {} IDs found", ids.len());

    // Pick a pseudo-random puzzle from the list
    let idx = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize)
        % ids.len();
    let id = ids[idx];

    send(3, &format!("Downloading #{id}…"));

    let puzzle = match nonogram::fetch_nonogram(id, req.type_bw) {
        Ok(p) => p,
        Err(e) => {
            send(2, &format!("Download error: {}", e));
            return;
        }
    };

    send(3, &format!("Generating PDF «{}»…", puzzle.title));

    let dir = "/home/root/.local/share/remarkable/xochitl";
    match pdf_gen::generate_pdf(&puzzle, dir) {
        Ok(path) => send(1, &format!("SAVED:{}", path)),
        Err(e)   => send(2, &format!("PDF error: {}", e)),
    }
}
