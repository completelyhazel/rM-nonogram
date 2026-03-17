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

    // Introspect /Synchronizer on no.remarkable.sync — the one object path
    // we haven't explored yet, revealed by the previous run's introspection.
    dbus_introspect_synchronizer();

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
                    eprintln!("[fetcher] notifying xochitl about {}", uuid);
                    notify_xochitl(path, &uuid);
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

    if let Some(handle) = active_worker {
        eprintln!("[fetcher] socket closed while download in progress — waiting...");
        let _ = handle.join();
        while let Ok((t, s)) = rx.try_recv() {
            if t == 1 {
                let path = s.trim_start_matches("SAVED:");
                if let Some(uuid) = extract_uuid(path) {
                    eprintln!("[fetcher] (late) notifying xochitl about {}", uuid);
                    notify_xochitl(path, &uuid);
                }
            }
        }
    }

    eprintln!("[fetcher] exiting.");
}

// ── D-Bus ─────────────────────────────────────────────────────────────────────

/// Introspect /Synchronizer on no.remarkable.sync and dump every method.
///
/// The previous run showed `<node name="Synchronizer"/>` as a child of `/` —
/// that child object is what this firmware uses for sync/document operations.
fn dbus_introspect_synchronizer() {
    eprintln!("[dbus] ── introspecting no.remarkable.sync /Synchronizer ──");

    // Also try the full reverse-DNS path just in case.
    for path in &["/Synchronizer", "/no/remarkable/sync/Synchronizer"] {
        let xml = dbus_call(
            "no.remarkable.sync",
            path,
            "org.freedesktop.DBus.Introspectable.Introspect",
            &[],
        );

        if xml.trim().is_empty()
            || xml.contains("UnknownObject")
            || xml.contains("ServiceUnknown")
        {
            eprintln!("[dbus]   {} → not found or empty", path);
            continue;
        }

        eprintln!("[dbus]   {} →", path);
        for line in xml.lines() {
            let t = line.trim();
            if t.starts_with("<node")
                || t.starts_with("<interface")
                || t.starts_with("<method")
                || t.starts_with("<signal")
                || t.starts_with("<property")
                || t.starts_with("<arg")
            {
                eprintln!("[dbus]     {}", t);
            }
        }
    }

    eprintln!("[dbus] ────────────────────────────────────────────────────────");
}

/// Try to trigger a library refresh / document open via D-Bus only.
///
/// NO signals are sent — SIGUSR1 disrupts the digitizer and SIGUSR2 kills
/// xochitl on this firmware (confirmed by the previous run's crash).
fn notify_xochitl(full_path: &str, uuid: &str) {
    // Candidates on /Synchronizer — the only real object no.remarkable.sync exposes.
    // (dest, object_path, interface.method, arg_type, arg_value)
    let candidates: &[(&str, &str, &str, &str, &str)] = &[
        // Try the most plausible method names on /Synchronizer
        ("no.remarkable.sync", "/Synchronizer",
            "no.remarkable.sync.Synchronizer.openDocument", "string", uuid),
        ("no.remarkable.sync", "/Synchronizer",
            "no.remarkable.sync.Synchronizer.openDocumentRequest", "string", uuid),
        ("no.remarkable.sync", "/Synchronizer",
            "no.remarkable.sync.Synchronizer.openFile", "string", full_path),
        ("no.remarkable.sync", "/Synchronizer",
            "no.remarkable.sync.Synchronizer.refresh", "", ""),
        ("no.remarkable.sync", "/Synchronizer",
            "no.remarkable.sync.Synchronizer.rescan", "", ""),
        ("no.remarkable.sync", "/Synchronizer",
            "no.remarkable.sync.Synchronizer.fileAdded", "string", full_path),
        ("no.remarkable.sync", "/Synchronizer",
            "no.remarkable.sync.Synchronizer.documentAdded", "string", uuid),
        // Interface name might just be "no.remarkable.sync" without the subpath
        ("no.remarkable.sync", "/Synchronizer",
            "no.remarkable.sync.openDocument", "string", uuid),
        ("no.remarkable.sync", "/Synchronizer",
            "no.remarkable.sync.refresh", "", ""),
        ("no.remarkable.sync", "/Synchronizer",
            "no.remarkable.sync.rescan", "", ""),
        ("no.remarkable.sync", "/Synchronizer",
            "no.remarkable.sync.fileAdded", "string", full_path),
    ];

    for (dest, path, method, arg_type, arg_val) in candidates {
        let mut cmd = std::process::Command::new("dbus-send");
        cmd.args([
            "--system",
            "--print-reply",
            &format!("--dest={}", dest),
            path,
            method,
        ]);
        if !arg_type.is_empty() {
            cmd.arg(&format!("{}:{}", arg_type, arg_val));
        }

        eprintln!("[dbus] trying {} {} {}", dest, path, method);
        match cmd.output() {
            Ok(o) => {
                let out = String::from_utf8_lossy(&o.stdout);
                let err = String::from_utf8_lossy(&o.stderr);
                eprintln!(
                    "[dbus]   exit={} stdout={:?} stderr={:?}",
                    o.status,
                    out.trim(),
                    err.trim()
                );
                if o.status.success() {
                    eprintln!("[dbus]   ✓ accepted — {} {} {}", dest, path, method);
                    return;
                }
            }
            Err(e) => eprintln!("[dbus]   exec error: {}", e),
        }
    }

    eprintln!(
        "[dbus] all candidates exhausted — \
         document will appear in library after next xochitl startup"
    );
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn dbus_call(dest: &str, path: &str, method: &str, extra: &[&str]) -> String {
    let mut args = vec![
        "--system".to_string(),
        "--print-reply".to_string(),
        format!("--dest={}", dest),
        path.to_string(),
        method.to_string(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));

    match std::process::Command::new("dbus-send").args(&args).output() {
        Ok(o) => {
            let mut out = String::from_utf8_lossy(&o.stdout).to_string();
            out.push_str(&String::from_utf8_lossy(&o.stderr));
            out
        }
        Err(e) => format!("exec error: {}", e),
    }
}

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