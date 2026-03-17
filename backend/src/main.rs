mod appload;
mod nonogram;
mod pdf_gen;

use std::env;
use std::sync::Mutex;
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

    // Dump xochitl's D-Bus introspection at startup so we can see every
    // available object path and method — useful for debugging open_in_xochitl.
    dbus_introspect();

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
    let saved_uuid: std::sync::Arc<Mutex<Option<String>>> =
        std::sync::Arc::new(Mutex::new(None));

    loop {
        while let Ok((t, s)) = rx.try_recv() {
            eprintln!("[fetcher] forwarding type={} to frontend", t);

            if t == 1 {
                let path = s.trim_start_matches("SAVED:");
                if let Some(uuid) = extract_uuid(path) {
                    eprintln!("[fetcher] opening document {} in xochitl", uuid);
                    open_in_xochitl(&uuid);
                    *saved_uuid.lock().unwrap() = Some(uuid);
                }
            }

            let _ = conn.send_message(t, &s);
        }

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

    // Wait for any in-progress download to finish writing to disk.
    if let Some(handle) = active_worker {
        eprintln!("[fetcher] socket closed while download in progress — waiting...");
        let _ = handle.join();

        // Handle the case where the worker finished just as the overlay closed.
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

    eprintln!("[fetcher] exiting.");
}

// ── D-Bus helpers ─────────────────────────────────────────────────────────────

/// Introspect the xochitl D-Bus service and dump everything to stderr.
///
/// Runs once at startup.  In the journal look for lines tagged [dbus] — they
/// show every object path, interface and method that xochitl exposes, so you
/// can update CANDIDATE_CALLS in open_in_xochitl() with the correct values.
fn dbus_introspect() {
    eprintln!("[dbus] ── xochitl introspection ──────────────────────────────");

    // 1. List every well-known name on the system bus so we can confirm the
    //    exact service name xochitl registers.
    let names_out = std::process::Command::new("dbus-send")
        .args([
            "--system",
            "--print-reply",
            "--dest=org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus.ListNames",
        ])
        .output();

    match names_out {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            for line in stdout.lines() {
                let l = line.trim().to_lowercase();
                if l.contains("xochitl") || l.contains("remarkable") {
                    eprintln!("[dbus]   bus name found: {}", line.trim());
                }
            }
        }
        Err(e) => eprintln!("[dbus]   ListNames failed: {}", e),
    }

    // 2. Introspect the root object of the most likely service names.
    for dest in &[
        "com.reMarkable.xochitl",
        "com.remarkable.xochitl",
    ] {
        eprintln!("[dbus]   introspecting {} at /", dest);
        let out = std::process::Command::new("dbus-send")
            .args([
                "--system",
                "--print-reply",
                &format!("--dest={}", dest),
                "/",
                "org.freedesktop.DBus.Introspectable.Introspect",
            ])
            .output();

        match out {
            Ok(o) if o.status.success() => {
                let xml = String::from_utf8_lossy(&o.stdout);
                for line in xml.lines() {
                    let t = line.trim();
                    if t.starts_with("<node")
                        || t.starts_with("<interface")
                        || t.starts_with("<method")
                        || t.starts_with("<signal")
                        || t.starts_with("<property")
                    {
                        eprintln!("[dbus]     {}", t);
                    }
                }
            }
            Ok(o) => {
                eprintln!(
                    "[dbus]   {} failed ({}): {}",
                    dest,
                    o.status,
                    String::from_utf8_lossy(&o.stderr).trim()
                );
            }
            Err(e) => eprintln!("[dbus]   {} error: {}", dest, e),
        }
    }

    eprintln!("[dbus] ────────────────────────────────────────────────────────");
}

/// Ask xochitl to open a document by UUID.
///
/// Each entry in `candidates` is tried in order, with --print-reply so we see
/// the actual response.  The first call that returns exit 0 wins.
///
/// After running once, check the journal for "[dbus]" lines and update this
/// list if needed.
fn open_in_xochitl(uuid: &str) {
    // (dest, object_path, full_method_with_interface)
    let candidates: &[(&str, &str, &str)] = &[
        // Root object, most common on recent firmware
        (
            "com.reMarkable.xochitl",
            "/",
            "com.reMarkable.xochitl.openDocumentRequest",
        ),
        (
            "com.reMarkable.xochitl",
            "/",
            "com.reMarkable.xochitl.openDocumentWithId",
        ),
        (
            "com.reMarkable.xochitl",
            "/",
            "com.reMarkable.xochitl.openFile",
        ),
        // Versioned interface variant
        (
            "com.reMarkable.xochitl",
            "/",
            "com.reMarkable.xochitl1.openDocumentRequest",
        ),
        // Non-root path variants
        (
            "com.reMarkable.xochitl",
            "/com/reMarkable/xochitl",
            "com.reMarkable.xochitl.openDocumentRequest",
        ),
        (
            "com.reMarkable.xochitl",
            "/com/reMarkable/xochitl",
            "com.reMarkable.xochitl1.openDocumentRequest",
        ),
        // Lowercase service variant used by some firmwares
        (
            "com.remarkable.xochitl",
            "/",
            "com.remarkable.xochitl.openDocumentRequest",
        ),
    ];

    for (dest, path, method) in candidates {
        eprintln!("[fetcher] trying D-Bus: dest={} path={} method={}", dest, path, method);

        let out = std::process::Command::new("dbus-send")
            .args([
                "--system",
                "--print-reply",           // wait for reply and show it
                &format!("--dest={}", dest),
                path,
                method,
                &format!("string:{}", uuid),
            ])
            .output();

        match out {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let stderr = String::from_utf8_lossy(&o.stderr);
                eprintln!(
                    "[fetcher]   exit={} | stdout: {} | stderr: {}",
                    o.status,
                    stdout.trim(),
                    stderr.trim()
                );

                if o.status.success() {
                    eprintln!(
                        "[fetcher]   ✓ accepted by xochitl — dest={} path={} method={}",
                        dest, path, method
                    );
                    return;
                }
            }
            Err(e) => eprintln!("[fetcher]   exec error: {}", e),
        }
    }

    eprintln!(
        "[fetcher] all D-Bus candidates exhausted — \
         document will appear in library after next xochitl startup"
    );
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
        Ok(path) => send(1, &format!("SAVED:{}", path)),
        Err(e)   => send(2, &format!("PDF error: {}", e)),
    }
}