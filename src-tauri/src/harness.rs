//! Harness engine subprocess manager.
//!
//! The DeepSeek Harness engine is the official `@deepseek-ai/dsh` CLI.
//! `dsh web` boots an embedded Web UI on a TCP port (default 3080) and
//! prints the access URL to stdout. This manager:
//!
//!  1. spawns `node <bin.js> web --no-open` in the chosen workdir
//!  2. reads stdout, capturing the first `http://...:PORT` URL
//!  3. waits for the HTTP endpoint to actually accept connections
//!  4. exposes the port and a liveness check
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicU16, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

/// Strip the Windows verbatim `\\?\` prefix. Rust's `Path` can surface these
/// (e.g. from `resource_dir()`), but child processes such as Node's module
/// loader mis-handle `\\?\` paths that contain spaces, surfacing errors like
/// `EISDIR lstat 'E:'`. A plain `E:\...` path avoids that.
fn plain(p: &Path) -> String {
    let s = p.to_string_lossy().to_string();
    s.strip_prefix("\\\\?\\").unwrap_or(&s).to_string()
}

pub struct HarnessManager {
    process: Mutex<Option<Child>>,
    port: Arc<AtomicU16>,
    workdir: Mutex<Option<PathBuf>>,
    /// Resolved engine: `(node executable, path to dsh bin.js)`. When the app
    /// is bundled, both ship under the Tauri resource dir so no system Node is
    /// required; otherwise `node` falls back to the system `PATH`.
    engine: Mutex<Option<(PathBuf, PathBuf)>>,
    /// Windows job object handle that forces the engine child to be killed
    /// when *this* process dies (even via a hard kill / Task Manager). Kept
    /// open for the manager's lifetime; closed on `stop()`/`Drop`.
    #[cfg(windows)]
    job: Mutex<Option<KillJobHandle>>,
}

impl Default for HarnessManager {
    fn default() -> Self {
        Self {
            process: Mutex::new(None),
            port: Arc::new(AtomicU16::new(0)),
            workdir: Mutex::new(None),
            engine: Mutex::new(None),
            #[cfg(windows)]
            job: Mutex::new(None),
        }
    }
}

impl Drop for HarnessManager {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            if let Ok(mut j) = self.job.lock() {
                if let Some(h) = j.take() {
                    unsafe {
                        windows_sys::Win32::Foundation::CloseHandle(h.0);
                    }
                }
            }
        }
    }
}

impl HarnessManager {
    pub fn new(workdir: Option<PathBuf>, engine: Option<(PathBuf, PathBuf)>) -> Self {
        let s = Self::default();
        *s.workdir.lock().expect("lock poisoned") = workdir;
        *s.engine.lock().expect("lock poisoned") = engine;
        s
    }

    /// Start the `dsh` engine.  `workdir` is captured in the manager.
    /// Blocks until the HTTP server is actually reachable.
    pub fn start(&self) -> Result<(), String> {
        let wd = self
            .workdir
            .lock()
            .expect("lock poisoned")
            .clone()
            .unwrap_or_else(|| std::env::current_dir().expect("cwd"));

        let (node, bin_js) = self
            .engine
            .lock()
            .expect("lock poisoned")
            .clone()
            .unwrap_or_else(|| (PathBuf::from("node"), PathBuf::from("dsh")));

        let node_s = plain(&node);
        let bin_s = plain(&bin_js);
        let mut cmd = Command::new(&node_s);
        cmd.arg(&bin_s);
        slog!("[engine] spawn node={node_s} bin={bin_s}");

        /* Ensure any child `node` invocations the engine makes resolve to the
           same bundled runtime rather than a system Node on PATH. */
        if let Some(node_dir) = node.parent() {
            if let Some(existing) = std::env::var_os("PATH") {
                let mut new_path = std::ffi::OsString::from(plain(node_dir));
                new_path.push(";");
                new_path.push(existing);
                cmd.env("PATH", new_path);
            }
        }

        /* Windows: hide the console window that Node/dsh.cmd would otherwise
           open. CREATE_NO_WINDOW = 0x08000000. Also suppress console windows
           spawned by Node's own child processes (the browser-opener helper
           that dsh-web-app spawns). */
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
            cmd.env("NODE_DISABLE_COLORS", "1");
        }

        cmd.arg("web");
        cmd.arg("--no-open");

        /* Pick a listen port up front so a leftover/stale engine already
           holding 3080 can never make THIS launch fail with EADDRINUSE.
           Prefer 3080; if it is taken, ask the OS for a free one (`--port 0`)
           and we read the actually-assigned port back from stdout below. */
        let listen_port = free_port_preferred(3080);
        cmd.arg("--port").arg(listen_port.to_string());

        cmd.current_dir(plain(&wd));
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());

        let mut child = cmd.spawn().map_err(|e| format!("spawn dsh: {e}"))?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let port = self.port.clone();

        /* Drain both streams in background threads so they never block.
           Also capture the port from stdout. */
        thread::spawn(move || {
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };
                slog!("[engine-stdout] {line}");
                if let Some(p) = extract_port(&line) {
                    if port
                        .compare_exchange(0, p, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                    {
                        slog!("[engine] detected port {p}");
                    }
                }
            }
        });
        thread::spawn(move || {
            let reader = std::io::BufReader::new(stderr);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };
                slog!("[engine-stderr] {line}");
            }
        });

        *self.process.lock().expect("lock poisoned") = Some(child);

        /* Windows: attach the freshly-spawned engine to a kill-on-close job
           so a forced termination of this shell also reaps the engine child
           (otherwise the orphan keeps 3080 busy and breaks the next launch). */
        #[cfg(windows)]
        {
            let pid = self
                .process
                .lock()
                .ok()
                .and_then(|g| g.as_ref().map(|c| c.id()));
            if let Some(pid) = pid {
                if let Some(h) = attach_kill_job(pid) {
                    *self.job.lock().expect("lock poisoned") = Some(h);
                } else {
                    slog!("[engine] warn: could not attach kill-on-close job to engine");
                }
            }
        }

        /* Phase 1: wait for port to be reported. The engine can take a while
           to come up on a first run (one-time fetch/setup of the web UI),
           so allow a generous window. */
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if self.port.load(Ordering::SeqCst) != 0 {
                break;
            }
            if Instant::now() > deadline {
                return Err(
                    "engine started but no port detected within 120s".to_string(),
                );
            }
            thread::sleep(Duration::from_millis(250));
        }
        let p = self.port.load(Ordering::SeqCst);

        /* Phase 2: wait for the HTTP server to actually accept connections.
           Generous window too, since the engine may still be warming up. */
        let http_deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if Instant::now() > http_deadline {
                return Err(
                    "engine port reported but HTTP not reachable within 60s"
                        .to_string(),
                );
            }
            match reqwest::blocking::get(format!("http://127.0.0.1:{p}/")) {
                Ok(_) => {
                    slog!("[engine] HTTP reachable on port {p}");
                    break;
                }
                Err(_) => thread::sleep(Duration::from_millis(300)),
            }
        }
        Ok(())
    }

    pub fn port(&self) -> Result<u16, String> {
        let p = self.port.load(Ordering::SeqCst);
        if p == 0 {
            return Err("port not yet detected".to_string());
        }
        Ok(p)
    }

    pub fn is_running(&self) -> bool {
        let mut g = match self.process.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        if let Some(ref mut c) = *g {
            c.try_wait().ok().flatten().is_none()
        } else {
            false
        }
    }

    pub fn stop(&self) -> Result<(), String> {
        let mut g = self.process.lock().map_err(|_| "lock poisoned")?;
        if let Some(ref mut c) = *g {
            let _ = c.kill();
            let _ = c.wait();
            *g = None;
        }
        self.port.store(0, Ordering::SeqCst);
        #[cfg(windows)]
        {
            if let Ok(mut j) = self.job.lock() {
                if let Some(h) = j.take() {
                    unsafe {
                        windows_sys::Win32::Foundation::CloseHandle(h.0);
                    }
                }
            }
        }
        Ok(())
    }
}

/// Pull a port out of strings like "http://127.0.0.1:3080" or "0.0.0.0:3080".
fn extract_port(s: &str) -> Option<u16> {
    let s = s.trim();
    for needle in &["http://", "https://", "0.0.0.0", "127.0.0.1", "localhost"] {
        if let Some(rest) = s.find(needle) {
            let rest = &s[rest + needle.len()..];
            if let Some(idx) = rest.find(':') {
                let after = &rest[idx + 1..];
                if let Some(end) = after.find(|c: char| !c.is_ascii_digit()) {
                    return after[..end].parse::<u16>().ok();
                }
                return after.parse::<u16>().ok();
            }
        }
    }
    None
}

/// Choose the engine listen port: prefer `preferred`, but if it is already
/// bound fall back to `0` so the OS assigns a free one. A brief bind test is
/// used to detect the conflict; the tiny race window before the engine binds
/// is acceptable for a desktop launcher.
fn free_port_preferred(preferred: u16) -> u16 {
    match std::net::TcpListener::bind(("127.0.0.1", preferred)) {
        Ok(_) => preferred,
        Err(_) => 0,
    }
}

/// Newtype wrapper so a raw Win32 `HANDLE` can live inside a `static`
/// (`HARNESS`). We only touch it from this process and always guard access
/// with a `Mutex`, so claiming `Send + Sync` here is sound.
#[cfg(windows)]
#[derive(Clone, Copy)]
struct KillJobHandle(pub windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
unsafe impl Send for KillJobHandle {}
#[cfg(windows)]
unsafe impl Sync for KillJobHandle {}

/// Windows: create a job object with `KillOnJobClose` and assign `pid` to it.
/// While we hold the returned handle open, any termination of *this* process
/// (including a hard kill) closes the handle and the OS reaps the engine
/// child — preventing orphaned `dsh web` processes from lingering and holding
/// the listen port. Returns `None` if the OS call fails (e.g. the child is
/// already in an incompatible job); the engine is then reaped only on a clean
/// `stop()`.
#[cfg(windows)]
fn attach_kill_job(pid: u32) -> Option<KillJobHandle> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_ALL_ACCESS};

    unsafe {
        let hproc = OpenProcess(PROCESS_ALL_ACCESS, 0, pid);
        if hproc.is_null() {
            return None;
        }
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            CloseHandle(hproc);
            return None;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set_ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) != 0;
        let assign_ok = AssignProcessToJobObject(job, hproc) != 0;
        CloseHandle(hproc);
        if !set_ok || !assign_ok {
            CloseHandle(job);
            return None;
        }
        Some(KillJobHandle(job))
    }
}