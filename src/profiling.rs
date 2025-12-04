//! CPU and Heap Profiling Support
//!
//! Provides pprof-compatible profiling endpoints with embedded Speedscope UI.
//! Enable with `--features pprof` at compile time.
//!
//! Usage:
//!   # Build with profiling
//!   cargo build --release --features pprof
//!
//!   # Collect 30s CPU profile
//!   curl http://localhost:6060/debug/pprof/profile?seconds=30 > profile.pb
//!
//!   # View CPU profile in browser with Speedscope UI
//!   open http://localhost:6060/debug/pprof/profile/ui?seconds=30
//!
//!   # Dump heap profile
//!   curl http://localhost:6060/debug/pprof/heap > heap.pb
//!
//!   # View memory stats
//!   curl http://localhost:6060/debug/pprof/heap/stats

use std::collections::HashMap;
use std::ffi::CString;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use parking_lot::RwLock;
use pprof::protos::Message;
use tikv_jemalloc_ctl::{epoch, stats};
use tokio::net::TcpListener;
use tracing::{error, info};
use uuid::Uuid;

// Embedded Speedscope assets
static SPEEDSCOPE_INDEX: &str = include_str!("../assets/speedscope/index.html");
static SPEEDSCOPE_JS: &[u8] = include_bytes!("../assets/speedscope/speedscope.6f107512.js");
static SPEEDSCOPE_IMPORT_JS: &[u8] = include_bytes!("../assets/speedscope/import.bcbb2033.js");
static SPEEDSCOPE_DEMANGLE_JS: &[u8] =
    include_bytes!("../assets/speedscope/demangle-cpp.1768f4cc.js");
static SPEEDSCOPE_SOURCEMAP_JS: &[u8] =
    include_bytes!("../assets/speedscope/source-map.438fa06b.js");
static SPEEDSCOPE_RESET_CSS: &[u8] = include_bytes!("../assets/speedscope/reset.8c46b7a1.css");
static SPEEDSCOPE_FONT_CSS: &[u8] =
    include_bytes!("../assets/speedscope/source-code-pro.52b1676f.css");
static SPEEDSCOPE_FONT_WOFF2: &[u8] =
    include_bytes!("../assets/speedscope/SourceCodePro-Regular.ttf.f546cbe0.woff2");
static SPEEDSCOPE_FAVICON_16: &[u8] =
    include_bytes!("../assets/speedscope/favicon-16x16.f74b3187.png");
static SPEEDSCOPE_FAVICON_32: &[u8] =
    include_bytes!("../assets/speedscope/favicon-32x32.bc503437.png");

/// Temporary storage for collected profiles
type ProfileStore = Arc<RwLock<HashMap<String, Vec<u8>>>>;

/// Default profiling server bind address
pub const DEFAULT_BIND: &str = "127.0.0.1:6060";

/// Start the profiling HTTP server
pub async fn start_server(
    bind: SocketAddr,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind(bind).await?;
    let profile_store: ProfileStore = Arc::new(RwLock::new(HashMap::new()));

    info!(
        "Profiling server listening on http://{}/debug/pprof/profile",
        bind
    );

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let store = profile_store.clone();

        tokio::spawn(async move {
            if let Err(e) = http1::Builder::new()
                .serve_connection(
                    io,
                    service_fn(move |req| handle_request(req, store.clone())),
                )
                .await
            {
                error!("Profiling server error: {}", e);
            }
        });
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    store: ProfileStore,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let path = req.uri().path();

    let response = match (req.method(), path) {
        // Raw profile download (protobuf)
        (&Method::GET, "/debug/pprof/profile") => {
            let seconds = parse_seconds(req.uri().query());
            match collect_profile(seconds).await {
                Ok(data) => Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "application/octet-stream")
                    .header("Content-Disposition", "attachment; filename=\"profile.pb\"")
                    .body(Full::new(Bytes::from(data)))
                    .unwrap(),
                Err(e) => error_response(&format!("Profile error: {}", e)),
            }
        }

        // Profile UI - collect and view in Speedscope
        (&Method::GET, "/debug/pprof/profile/ui") => {
            let seconds = parse_seconds(req.uri().query());
            match collect_profile(seconds).await {
                Ok(data) => {
                    let id = Uuid::new_v4().to_string();
                    store.write().insert(id.clone(), data);
                    redirect_response(&format!(
                        "/debug/pprof/speedscope/#profileURL=/debug/pprof/data/{}.pb",
                        id
                    ))
                }
                Err(e) => error_response(&format!("Profile error: {}", e)),
            }
        }

        // Raw flamegraph download (SVG)
        (&Method::GET, "/debug/pprof/flamegraph") => {
            let seconds = parse_seconds(req.uri().query());
            match collect_flamegraph(seconds).await {
                Ok(svg) => Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "image/svg+xml")
                    .body(Full::new(Bytes::from(svg)))
                    .unwrap(),
                Err(e) => error_response(&format!("Flamegraph error: {}", e)),
            }
        }

        // Flamegraph UI - collect and view in Speedscope
        (&Method::GET, "/debug/pprof/flamegraph/ui") => {
            let seconds = parse_seconds(req.uri().query());
            match collect_profile(seconds).await {
                Ok(data) => {
                    let id = Uuid::new_v4().to_string();
                    store.write().insert(id.clone(), data);
                    redirect_response(&format!(
                        "/debug/pprof/speedscope/#profileURL=/debug/pprof/data/{}.pb",
                        id
                    ))
                }
                Err(e) => error_response(&format!("Flamegraph error: {}", e)),
            }
        }

        // Plaintext collapsed stacks format
        (&Method::GET, "/debug/pprof/stacks") => {
            let seconds = parse_seconds(req.uri().query());
            match collect_stacks(seconds).await {
                Ok(text) => Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "text/plain; charset=utf-8")
                    .body(Full::new(Bytes::from(text)))
                    .unwrap(),
                Err(e) => error_response(&format!("Stacks error: {}", e)),
            }
        }

        // Top functions (Go pprof style)
        (&Method::GET, "/debug/pprof/top") => {
            let seconds = parse_seconds(req.uri().query());
            match collect_top(seconds).await {
                Ok(text) => Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "text/plain; charset=utf-8")
                    .body(Full::new(Bytes::from(text)))
                    .unwrap(),
                Err(e) => error_response(&format!("Top error: {}", e)),
            }
        }

        // Heap profile dump (jemalloc)
        (&Method::GET, "/debug/pprof/heap") => match dump_heap_profile() {
            Ok(data) => Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/octet-stream")
                .header("Content-Disposition", "attachment; filename=\"heap.pb\"")
                .body(Full::new(Bytes::from(data)))
                .unwrap(),
            Err(e) => error_response(&format!("Heap profile error: {}", e)),
        },

        // Heap stats (plaintext)
        (&Method::GET, "/debug/pprof/heap/stats") => match get_heap_stats() {
            Ok(text) => Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/plain; charset=utf-8")
                .body(Full::new(Bytes::from(text)))
                .unwrap(),
            Err(e) => error_response(&format!("Heap stats error: {}", e)),
        },

        // Heap top (plaintext, like go pprof)
        (&Method::GET, "/debug/pprof/heap/top") => match get_heap_top() {
            Ok(text) => Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/plain; charset=utf-8")
                .body(Full::new(Bytes::from(text)))
                .unwrap(),
            Err(e) => error_response(&format!("Heap top error: {}", e)),
        },

        // Serve stored profile data
        (&Method::GET, p) if p.starts_with("/debug/pprof/data/") => {
            let id = p
                .strip_prefix("/debug/pprof/data/")
                .and_then(|s| s.strip_suffix(".pb"))
                .unwrap_or("");

            if let Some(data) = store.write().remove(id) {
                Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "application/octet-stream")
                    .header("Access-Control-Allow-Origin", "*")
                    .body(Full::new(Bytes::from(data)))
                    .unwrap()
            } else {
                not_found_response()
            }
        }

        // Speedscope static assets
        (&Method::GET, "/debug/pprof/speedscope/")
        | (&Method::GET, "/debug/pprof/speedscope/index.html") => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/html")
            .body(Full::new(Bytes::from(SPEEDSCOPE_INDEX)))
            .unwrap(),

        (&Method::GET, "/debug/pprof/speedscope/speedscope.6f107512.js") => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/javascript")
            .body(Full::new(Bytes::from_static(SPEEDSCOPE_JS)))
            .unwrap(),

        (&Method::GET, "/debug/pprof/speedscope/import.bcbb2033.js") => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/javascript")
            .body(Full::new(Bytes::from_static(SPEEDSCOPE_IMPORT_JS)))
            .unwrap(),

        (&Method::GET, "/debug/pprof/speedscope/demangle-cpp.1768f4cc.js") => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/javascript")
            .body(Full::new(Bytes::from_static(SPEEDSCOPE_DEMANGLE_JS)))
            .unwrap(),

        (&Method::GET, "/debug/pprof/speedscope/source-map.438fa06b.js") => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/javascript")
            .body(Full::new(Bytes::from_static(SPEEDSCOPE_SOURCEMAP_JS)))
            .unwrap(),

        (&Method::GET, "/debug/pprof/speedscope/reset.8c46b7a1.css") => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/css")
            .body(Full::new(Bytes::from_static(SPEEDSCOPE_RESET_CSS)))
            .unwrap(),

        (&Method::GET, "/debug/pprof/speedscope/source-code-pro.52b1676f.css") => {
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/css")
                .body(Full::new(Bytes::from_static(SPEEDSCOPE_FONT_CSS)))
                .unwrap()
        }

        (&Method::GET, "/debug/pprof/speedscope/SourceCodePro-Regular.ttf.f546cbe0.woff2") => {
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "font/woff2")
                .body(Full::new(Bytes::from_static(SPEEDSCOPE_FONT_WOFF2)))
                .unwrap()
        }

        (&Method::GET, "/debug/pprof/speedscope/favicon-16x16.f74b3187.png") => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "image/png")
            .body(Full::new(Bytes::from_static(SPEEDSCOPE_FAVICON_16)))
            .unwrap(),

        (&Method::GET, "/debug/pprof/speedscope/favicon-32x32.bc503437.png") => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "image/png")
            .body(Full::new(Bytes::from_static(SPEEDSCOPE_FAVICON_32)))
            .unwrap(),

        // Landing page
        (&Method::GET, "/") | (&Method::GET, "/debug/pprof") | (&Method::GET, "/debug/pprof/") => {
            let html = r#"<!DOCTYPE html>
<html>
<head>
  <title>VibeMQ Profiling</title>
  <style>
    * { box-sizing: border-box; }
    body { font-family: system-ui, -apple-system, sans-serif; max-width: 900px; margin: 0 auto; padding: 2rem 1rem; background: #fafafa; }
    h1 { color: #1a1a1a; margin-bottom: 0.5rem; }
    .subtitle { color: #666; margin-bottom: 2rem; }
    .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 1.5rem; }
    @media (max-width: 700px) { .grid { grid-template-columns: 1fr; } }
    .card { background: white; border-radius: 12px; padding: 1.5rem; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }
    .card h2 { margin: 0 0 0.5rem 0; font-size: 1.1rem; color: #333; }
    .card p { color: #666; font-size: 0.9rem; margin: 0 0 1rem 0; }
    .form-row { display: flex; gap: 0.5rem; margin-bottom: 0.75rem; }
    input[type="number"] { width: 80px; padding: 0.5rem; border: 1px solid #ddd; border-radius: 6px; font-size: 1rem; }
    .btn { padding: 0.5rem 1rem; border: none; border-radius: 6px; font-size: 0.9rem; cursor: pointer; text-decoration: none; display: inline-flex; align-items: center; gap: 0.25rem; }
    .btn-primary { background: #2563eb; color: white; }
    .btn-primary:hover { background: #1d4ed8; }
    .btn-secondary { background: #e5e7eb; color: #374151; }
    .btn-secondary:hover { background: #d1d5db; }
    .btn-small { padding: 0.4rem 0.75rem; font-size: 0.8rem; }
    .links { display: flex; flex-wrap: wrap; gap: 0.5rem; margin-top: 0.75rem; }
    .status { padding: 1rem; background: #f0fdf4; border-radius: 8px; margin-top: 1.5rem; }
    .status.loading { background: #fef3c7; }
    .status.error { background: #fef2f2; }
    .status h3 { margin: 0 0 0.5rem 0; font-size: 0.9rem; }
    .hidden { display: none; }
    pre { background: #1e293b; color: #e2e8f0; padding: 1rem; border-radius: 8px; overflow-x: auto; font-size: 0.8rem; margin: 0; }
  </style>
</head>
<body>
  <h1>VibeMQ Profiling</h1>
  <p class="subtitle">CPU and heap profiling with Speedscope visualization</p>

  <div class="grid">
    <div class="card">
      <h2>CPU Profile</h2>
      <p>Sample CPU usage and view as interactive flamegraph</p>
      <div class="form-row">
        <input type="number" id="cpu-seconds" value="10" min="1" max="300">
        <span style="line-height:2.2">seconds</span>
        <button class="btn btn-primary" onclick="startCpuProfile()">Start Profile</button>
      </div>
      <div class="links">
        <button class="btn btn-secondary btn-small" onclick="openCpu(&quot;top&quot;)">Top Functions</button>
        <button class="btn btn-secondary btn-small" onclick="openCpu(&quot;flamegraph&quot;)">SVG</button>
        <button class="btn btn-secondary btn-small" onclick="openCpu(&quot;stacks&quot;)">Stacks</button>
        <button class="btn btn-secondary btn-small" onclick="openCpu(&quot;profile&quot;)">Protobuf</button>
      </div>
    </div>

    <div class="card">
      <h2>Heap Profile</h2>
      <p>Memory allocation statistics and heap dump</p>
      <div class="form-row">
        <button class="btn btn-primary" onclick="goTo(&quot;/debug/pprof/heap/stats&quot;)">View Stats</button>
        <button class="btn btn-secondary" onclick="goTo(&quot;/debug/pprof/heap/top&quot;)">Top Allocs</button>
        <button class="btn btn-secondary" onclick="goTo(&quot;/debug/pprof/heap&quot;)">Download</button>
      </div>
    </div>
  </div>

  <div id="status" class="status hidden">
    <h3 id="status-title">Collecting...</h3>
    <p id="status-msg"></p>
  </div>

  <div class="card" style="margin-top:1.5rem;">
    <h2>CLI Usage</h2>
    <pre># CPU: Top functions (human readable)
curl http://localhost:6060/debug/pprof/top?seconds=10

# CPU: Collapsed stacks (for flamegraph tools)
curl http://localhost:6060/debug/pprof/stacks?seconds=10

# Heap: Statistics
curl http://localhost:6060/debug/pprof/heap/stats

# Heap: Top allocation sites
curl http://localhost:6060/debug/pprof/heap/top

# Download protobuf for go tool pprof
curl http://localhost:6060/debug/pprof/profile?seconds=30 -o profile.pb
go tool pprof -http=:8080 profile.pb</pre>
  </div>

  <script>
    function showStatus(title, msg, type) {
      const el = document.getElementById("status");
      el.className = "status " + (type || "");
      document.getElementById("status-title").textContent = title;
      document.getElementById("status-msg").textContent = msg;
    }

    function hideStatus() {
      document.getElementById("status").className = "status hidden";
    }

    function openUrl(url) {
      window.open(url, "_blank");
    }

    function goTo(url) {
      window.location = url;
    }

    function openCpu(endpoint) {
      const seconds = document.getElementById("cpu-seconds").value;
      window.open("/debug/pprof/" + endpoint + "?seconds=" + seconds, "_blank");
    }

    function startCpuProfile() {
      const seconds = document.getElementById("cpu-seconds").value;
      showStatus("Collecting CPU profile...", "Sampling for " + seconds + " seconds. Please wait.", "loading");

      // Redirect to the UI endpoint which will collect and then show Speedscope
      setTimeout(function() {
        window.location = "/debug/pprof/profile/ui?seconds=" + seconds;
      }, 100);
    }
  </script>
</body>
</html>"#;
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/html")
                .body(Full::new(Bytes::from(html)))
                .unwrap()
        }

        _ => not_found_response(),
    };

    Ok(response)
}

fn parse_seconds(query: Option<&str>) -> u64 {
    query
        .and_then(|q| {
            q.split('&')
                .find(|p| p.starts_with("seconds="))
                .and_then(|p| p.strip_prefix("seconds="))
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(30)
}

fn error_response(msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .body(Full::new(Bytes::from(msg.to_string())))
        .unwrap()
}

fn not_found_response() -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Full::new(Bytes::from("Not Found")))
        .unwrap()
}

fn redirect_response(location: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::FOUND)
        .header("Location", location)
        .body(Full::new(Bytes::from("")))
        .unwrap()
}

async fn collect_profile(
    seconds: u64,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    info!("Starting {}s CPU profile", seconds);

    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(99) // Use 99Hz to avoid lockstep with timers
        .build()?;

    tokio::time::sleep(Duration::from_secs(seconds)).await;

    let report = guard.report().build()?;

    // Check if we actually got any samples
    if report.data.is_empty() {
        return Err("No samples collected - the process was likely idle during profiling. Try again while the broker is under load.".into());
    }

    let mut buf = Vec::new();
    let profile = report.pprof()?;
    profile.encode(&mut buf)?;

    info!(
        "CPU profile collected ({} bytes, {} unique stacks)",
        buf.len(),
        report.data.len()
    );
    Ok(buf)
}

async fn collect_flamegraph(
    seconds: u64,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    info!("Starting {}s flamegraph collection", seconds);

    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(99)
        .build()?;

    tokio::time::sleep(Duration::from_secs(seconds)).await;

    let report = guard.report().build()?;

    if report.data.is_empty() {
        return Err("No samples collected - the process was likely idle during profiling. Try again while the broker is under load.".into());
    }

    let mut buf = Vec::new();
    report.flamegraph(&mut buf)?;

    info!(
        "Flamegraph collected ({} bytes, {} unique stacks)",
        buf.len(),
        report.data.len()
    );
    Ok(buf)
}

async fn collect_stacks(seconds: u64) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    info!("Starting {}s stack collection", seconds);

    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(99)
        .build()?;

    tokio::time::sleep(Duration::from_secs(seconds)).await;

    let report = guard.report().build()?;

    if report.data.is_empty() {
        return Err("No samples collected - the process was likely idle during profiling. Try again while the broker is under load.".into());
    }

    // Build collapsed stacks format: "func1;func2;func3 count\n"
    let mut output = String::new();
    for (frames, count) in &report.data {
        let stack: Vec<String> = frames
            .frames
            .iter()
            .rev()
            .filter_map(|symbols| symbols.first().map(|s| s.name()))
            .collect();
        if !stack.is_empty() {
            output.push_str(&stack.join(";"));
            output.push(' ');
            output.push_str(&count.to_string());
            output.push('\n');
        }
    }

    info!(
        "Stacks collected ({} bytes, {} unique stacks)",
        output.len(),
        report.data.len()
    );
    Ok(output)
}

async fn collect_top(seconds: u64) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    info!("Starting {}s top collection", seconds);

    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(99)
        .build()?;

    tokio::time::sleep(Duration::from_secs(seconds)).await;

    let report = guard.report().build()?;

    if report.data.is_empty() {
        return Err("No samples collected - the process was likely idle during profiling. Try again while the broker is under load.".into());
    }

    // Aggregate samples by function (skip profiler frames)
    let mut func_samples: HashMap<String, isize> = HashMap::new();
    let total_samples: isize = report.data.values().sum();

    for (frames, count) in &report.data {
        // Find the first non-profiler frame
        for symbols in &frames.frames {
            if let Some(sym) = symbols.first() {
                let name = sym.name();
                // Skip profiler/backtrace frames
                if name.contains("pprof::")
                    || name.contains("backtrace::")
                    || name.contains("__rust_try")
                    || name.contains("catch_unwind")
                {
                    continue;
                }
                *func_samples.entry(name).or_insert(0) += count;
                break; // Only count the first real frame per stack
            }
        }
    }

    // Sort by sample count descending
    let mut sorted: Vec<_> = func_samples.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    // Build output like go pprof
    let mut output = String::new();
    output.push_str(&format!(
        "Total samples: {} ({:.1}s at 99Hz)\n",
        total_samples,
        total_samples as f64 / 99.0
    ));
    output.push_str(&format!(
        "Showing top {} functions:\n\n",
        sorted.len().min(30)
    ));
    output.push_str(&format!("{:>8} {:>7}  {}\n", "samples", "%", "function"));
    output.push_str(&format!("{}\n", "-".repeat(60)));

    for (func, count) in sorted.iter().take(30) {
        let pct = (*count as f64 / total_samples as f64) * 100.0;
        // Shorten long function names
        let display_name = if func.len() > 60 {
            format!("...{}", &func[func.len() - 57..])
        } else {
            func.clone()
        };
        output.push_str(&format!("{:>8} {:>6.1}%  {}\n", count, pct, display_name));
    }

    info!(
        "Top collected ({} functions, {} total samples)",
        sorted.len(),
        total_samples
    );
    Ok(output)
}

fn dump_heap_profile() -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    // Create temp file for jemalloc to write to
    let path = std::env::temp_dir().join(format!("vibemq_heap_{}.prof", std::process::id()));
    let path_str = path.to_string_lossy();
    let c_path = CString::new(path_str.as_bytes())?;

    // Trigger heap profile dump via mallctl
    // SAFETY: We're calling jemalloc's mallctl with a valid path
    unsafe {
        let name = CString::new("prof.dump")?;
        let ret = tikv_jemalloc_sys::mallctl(
            name.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            c_path.as_ptr() as *mut _,
            std::mem::size_of::<*const i8>(),
        );
        if ret != 0 {
            return Err(format!(
                "jemalloc heap profiling not available (code {}).\n\n\
                 To enable heap profiling:\n\
                 1. Install libunwind-dev: sudo apt install libunwind-dev\n\
                 2. Clean rebuild: cargo clean && cargo build --features pprof\n\
                 3. Run with: MALLOC_CONF=prof:true cargo run --features pprof",
                ret
            )
            .into());
        }
    }

    // Read the profile
    let data = std::fs::read(&path)?;
    let _ = std::fs::remove_file(&path);

    info!("Heap profile dumped ({} bytes)", data.len());
    Ok(data)
}

fn get_heap_stats() -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Advance epoch to get fresh stats
    epoch::advance().map_err(|e| format!("epoch advance failed: {:?}", e))?;

    let allocated = stats::allocated::read().map_err(|e| format!("stats read failed: {:?}", e))?;
    let active = stats::active::read().map_err(|e| format!("stats read failed: {:?}", e))?;
    let resident = stats::resident::read().map_err(|e| format!("stats read failed: {:?}", e))?;
    let mapped = stats::mapped::read().map_err(|e| format!("stats read failed: {:?}", e))?;
    let retained = stats::retained::read().map_err(|e| format!("stats read failed: {:?}", e))?;

    let output = format!(
        "Heap Statistics (jemalloc)\n\
         ==========================\n\
         Allocated: {:>12} ({:.2} MB)\n\
         Active:    {:>12} ({:.2} MB)\n\
         Resident:  {:>12} ({:.2} MB)\n\
         Mapped:    {:>12} ({:.2} MB)\n\
         Retained:  {:>12} ({:.2} MB)\n\
         \n\
         Definitions:\n\
         - Allocated: Total bytes allocated by the application\n\
         - Active: Total bytes in active pages (including fragmentation)\n\
         - Resident: Total bytes in physically resident pages\n\
         - Mapped: Total bytes in mapped memory regions\n\
         - Retained: Total bytes retained for future allocations\n",
        allocated,
        allocated as f64 / 1024.0 / 1024.0,
        active,
        active as f64 / 1024.0 / 1024.0,
        resident,
        resident as f64 / 1024.0 / 1024.0,
        mapped,
        mapped as f64 / 1024.0 / 1024.0,
        retained,
        retained as f64 / 1024.0 / 1024.0,
    );

    Ok(output)
}

fn get_heap_top() -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Advance epoch to get fresh stats
    epoch::advance().map_err(|e| format!("epoch advance failed: {:?}", e))?;

    // Dump heap profile to temp file
    let path = std::env::temp_dir().join(format!("vibemq_heap_top_{}.prof", std::process::id()));
    let path_str = path.to_string_lossy();
    let c_path = CString::new(path_str.as_bytes())?;

    unsafe {
        let name = CString::new("prof.dump")?;
        let ret = tikv_jemalloc_sys::mallctl(
            name.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            c_path.as_ptr() as *mut _,
            std::mem::size_of::<*const i8>(),
        );
        if ret != 0 {
            return Err(format!(
                "jemalloc heap profiling not available (code {}).\n\n\
                 To enable heap profiling:\n\
                 1. Install libunwind-dev: sudo apt install libunwind-dev\n\
                 2. Clean rebuild: cargo clean && cargo build --features pprof\n\
                 3. Run with: MALLOC_CONF=prof:true cargo run --features pprof\n\n\
                 Use /debug/pprof/heap/stats for basic memory statistics (always works).",
                ret
            )
            .into());
        }
    }

    // Read and parse the profile
    let content = std::fs::read_to_string(&path)?;
    let _ = std::fs::remove_file(&path);

    // Parse jemalloc heap profile format
    // Format: "heap_v2/..." header, then @ lines with stack traces, then allocation counts
    let mut allocations: Vec<(u64, u64, Vec<String>)> = Vec::new(); // (objects, bytes, stack)
    let mut current_stack: Vec<String> = Vec::new();
    let mut total_bytes: u64 = 0;

    for line in content.lines() {
        if line.starts_with('@') {
            // Stack trace line: @ 0x123 0x456 ...
            // We'd need to symbolicate addresses - for now just note we have a stack
            current_stack = vec![line.to_string()];
        } else if line.trim().starts_with("t*:") || line.trim().starts_with("t0:") {
            // Allocation count line: t*: objects: bytes [cumulative]
            // Format: "  t*: 123: 456789 [0: 0]"
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 3 {
                if let Ok(objects) = parts[1].trim().parse::<u64>() {
                    // bytes part might have " [" suffix
                    let bytes_str = parts[2].split('[').next().unwrap_or("0").trim();
                    if let Ok(bytes) = bytes_str.parse::<u64>() {
                        if !current_stack.is_empty() && bytes > 0 {
                            allocations.push((objects, bytes, current_stack.clone()));
                        }
                    }
                }
            }
        } else if line.starts_with("heap_v2") {
            // Header line with totals: heap_v2/524288
            // Next line has totals
        } else if line.trim().starts_with("t*:") && total_bytes == 0 {
            // First t*: line is the total
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 3 && parts[1].trim().parse::<u64>().is_ok() {
                let bytes_str = parts[2].split('[').next().unwrap_or("0").trim();
                if let Ok(bytes) = bytes_str.parse::<u64>() {
                    total_bytes = bytes;
                }
            }
        }
    }

    // Sort by bytes descending
    allocations.sort_by(|a, b| b.1.cmp(&a.1));

    // Get actual stats for display
    let allocated = stats::allocated::read().map_err(|e| format!("stats read failed: {:?}", e))?;
    let active = stats::active::read().map_err(|e| format!("stats read failed: {:?}", e))?;

    // Build output
    let mut output = String::new();
    output.push_str(&format!(
        "Heap Profile (jemalloc)\n\
         =======================\n\
         Allocated: {:.2} MB ({} bytes)\n\
         Active:    {:.2} MB ({} bytes)\n\n",
        allocated as f64 / 1024.0 / 1024.0,
        allocated,
        active as f64 / 1024.0 / 1024.0,
        active
    ));

    if allocations.is_empty() {
        output.push_str(
            "No allocation site data available.\n\n\
             To see allocation call stacks:\n\
             1. Set MALLOC_CONF=prof:true,lg_prof_sample:17 before running\n\
             2. Use: jeprof --text /path/to/binary /debug/pprof/heap\n\
             3. Or:  jeprof --svg /path/to/binary /debug/pprof/heap > heap.svg\n",
        );
    } else {
        output.push_str(&format!(
            "Showing top {} allocation sites by bytes:\n\n",
            allocations.len().min(30)
        ));
        output.push_str(&format!(
            "{:>10} {:>12} {:>7}  {}\n",
            "objects", "bytes", "%", "stack"
        ));
        output.push_str(&format!("{}\n", "-".repeat(70)));

        for (objects, bytes, stack) in allocations.iter().take(30) {
            let pct = if total_bytes > 0 {
                (*bytes as f64 / total_bytes as f64) * 100.0
            } else {
                0.0
            };
            // Show just the first part of the stack (addresses)
            let stack_str = stack
                .first()
                .map(|s| {
                    if s.len() > 40 {
                        format!("{}...", &s[..40])
                    } else {
                        s.clone()
                    }
                })
                .unwrap_or_default();
            output.push_str(&format!(
                "{:>10} {:>12} {:>6.1}%  {}\n",
                objects,
                format_bytes(*bytes),
                pct,
                stack_str
            ));
        }

        output.push_str(
            "\nNote: Stack traces shown as addresses. For symbolicated output:\n\
             jeprof --text $(which vibemq) http://localhost:6060/debug/pprof/heap\n",
        );
    }

    Ok(output)
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / 1024.0 / 1024.0 / 1024.0)
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}
