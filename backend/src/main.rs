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

    // Dump everything we can find on D-Bus and identify xochitl's PID so that
    // notify_xochitl() has the best possible information to work with.
    dbus_introspect_all();

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

// ── Notification strategy ─────────────────────────────────────────────────────
//
// We try every known mechanism in order.  Each attempt is logged so the
// journal tells us exactly which one xochitl responded to.

fn notify_xochitl(full_path: &str, uuid: &str) {
    // 1. Try every D-Bus service we actually found on the bus.
    try_dbus_sync(full_path, uuid);

    // 2. Send SIGUSR1 / SIGUSR2 to xochitl — some firmware versions use these
    //    as a "rescan library" or "open file" signal.
    try_signals_to_xochitl(uuid);
}

/// Try methods on the services that are actually present on this device.
fn try_dbus_sync(full_path: &str, uuid: &str) {
    // (dest, object_path, method, arg_type, arg_value)
    // arg_type: "string" | "path" | ""  (empty = no argument)
    let candidates: &[(&str, &str, &str, &str, &str)] = &[
        // no.remarkable.sync — try every plausible method name
        ("no.remarkable.sync", "/",
            "no.remarkable.sync.openDocument", "string", uuid),
        ("no.remarkable.sync", "/",
            "no.remarkable.sync.openDocumentRequest", "string", uuid),
        ("no.remarkable.sync", "/",
            "no.remarkable.sync.openFile", "string", full_path),
        ("no.remarkable.sync", "/",
            "no.remarkable.sync.rescan", "", ""),
        ("no.remarkable.sync", "/",
            "no.remarkable.sync.refreshLibrary", "", ""),
        ("no.remarkable.sync", "/",
            "no.remarkable.sync.fileAdded", "string", full_path),
        ("no.remarkable.sync", "/",
            "no.remarkable.sync.documentAdded", "string", uuid),
        // Same names under /no/remarkable/sync path
        ("no.remarkable.sync", "/no/remarkable/sync",
            "no.remarkable.sync.openDocument", "string", uuid),
        ("no.remarkable.sync", "/no/remarkable/sync",
            "no.remarkable.sync.rescan", "", ""),
        ("no.remarkable.sync", "/no/remarkable/sync",
            "no.remarkable.sync.fileAdded", "string", full_path),
        // no.remarkable.marker (pen service — unlikely, but log what it exposes)
        ("no.remarkable.marker", "/",
            "no.remarkable.marker.openDocument", "string", uuid),
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
                eprintln!("[dbus]   exit={} stdout={:?} stderr={:?}",
                    o.status, out.trim(), err.trim());
                if o.status.success() {
                    eprintln!("[dbus]   ✓ accepted — {} {} {}", dest, path, method);
                    return;
                }
            }
            Err(e) => eprintln!("[dbus]   exec error: {}", e),
        }
    }
}

/// Send SIGUSR1 then SIGUSR2 to every xochitl process we can find.
/// On some firmware SIGUSR1 triggers a library rescan.
/// On some firmware SIGUSR2 opens the last-modified document.
fn try_signals_to_xochitl(uuid: &str) {
    let pids = find_xochitl_pids();
    if pids.is_empty() {
        eprintln!("[signal] no xochitl PIDs found");
        return;
    }

    for pid in &pids {
        eprintln!("[signal] sending SIGUSR1 to xochitl PID {}", pid);
        let _ = std::process::Command::new("kill")
            .args(["-SIGUSR1", pid])
            .status();

        // Small gap so xochitl processes SIGUSR1 before receiving SIGUSR2.
        std::thread::sleep(std::time::Duration::from_millis(200));

        eprintln!("[signal] sending SIGUSR2 to xochitl PID {}", pid);
        let _ = std::process::Command::new("kill")
            .args(["-SIGUSR2", pid])
            .status();
    }

    // Give xochitl a moment to pick up the new file after the signal.
    std::thread::sleep(std::time::Duration::from_millis(500));
    eprintln!("[signal] done — check if document {} appeared", uuid);
}

/// Return a list of PIDs whose command name is "xochitl".
fn find_xochitl_pids() -> Vec<String> {
    // Try pidof first (faster), fall back to scanning /proc manually.
    let pidof = std::process::Command::new("pidof")
        .arg("xochitl")
        .output();

    if let Ok(o) = pidof {
        if o.status.success() {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let pids: Vec<String> = stdout
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();
            eprintln!("[signal] xochitl PIDs (pidof): {:?}", pids);
            return pids;
        }
    }

    // Fall back: read /proc/*/comm
    let mut pids = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.chars().all(|c| c.is_ascii_digit()) {
                let comm_path = format!("/proc/{}/comm", name_str);
                if let Ok(comm) = std::fs::read_to_string(&comm_path) {
                    if comm.trim() == "xochitl" {
                        pids.push(name_str.to_string());
                    }
                }
            }
        }
    }
    eprintln!("[signal] xochitl PIDs (/proc scan): {:?}", pids);
    pids
}

// ── D-Bus introspection at startup ────────────────────────────────────────────

fn dbus_introspect_all() {
    eprintln!("[dbus] ── full system bus introspection ──────────────────────");

    // Print every service name — look for anything remarkable/xochitl related.
    let names = dbus_call(
        "org.freedesktop.DBus", "/org/freedesktop/DBus",
        "org.freedesktop.DBus.ListNames", &[],
    );
    eprintln!("[dbus] all bus names:\n{}", names.trim());

    // Fully introspect the services we know exist.
    for dest in &["no.remarkable.sync", "no.remarkable.marker", "com.remarkable.devicepolicy"] {
        eprintln!("[dbus] ── introspecting {} ──", dest);

        // Try root object first, then guessed sub-paths.
        for path in &["/", &format!("/{}", dest.replace('.', "/"))] {
            let xml = dbus_call(dest, path, "org.freedesktop.DBus.Introspectable.Introspect", &[]);
            if xml.trim().is_empty() || xml.contains("ServiceUnknown") || xml.contains("error") {
                continue;
            }
            eprintln!("[dbus]   path {} :", path);
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
    }

    eprintln!("[dbus] ────────────────────────────────────────────────────────");
}

/// Run dbus-send --print-reply and return stdout + stderr as a single string.
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

// ─────────────────────────────────────────────────────────────────────────────

fn extract_uuid(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

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