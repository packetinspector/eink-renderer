//! eink-renderer: serve a Home Assistant Lovelace view as a 1-bit PNG for an e-paper display.
mod cdp;
mod config;
mod engine;
mod image;
mod mdns;
mod mem;
mod placeholder;
mod store;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{
    HeaderValue, AUTHORIZATION, CACHE_CONTROL, CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_NONE_MATCH,
};
use hyper::{Method, Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::sync::{Arc, Mutex, RwLock};

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// UTC timestamp for log lines (no chrono dependency).
fn fmt_utc(t: u64) -> String {
    let (d, s) = (t / 86400, t % 86400);
    let z = d as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{day:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

pub fn log(msg: &str) {
    eprintln!("{} {msg}", fmt_utc(now_unix()));
}

/// A rendered image, held in memory and served from there.
pub struct Frame {
    pub png: Bytes,
    pub preview: Bytes,
    pub grey: Vec<u8>,
    pub etag: String,
    pub at: u64,
    pub w: u32,
    pub h: u32,
    /// true for generated status images (no real frame yet)
    pub placeholder: bool,
}

impl Frame {
    pub fn new(png: Vec<u8>, grey: Vec<u8>, cfg: &config::Config) -> Frame {
        let etag = etag_of(&png);
        let p = image::Params {
            width: cfg.width,
            height: cfg.height,
            top: 0,
            threshold: cfg.threshold,
            invert: false,
            dither: image::Dither::parse(&cfg.dither),
        };
        let preview = image::encode_1bit(&image::to_bits(&grey, &p), cfg.width, cfg.height);
        Frame {
            png: png.into(),
            preview: preview.into(),
            grey,
            etag,
            at: now_unix(),
            w: cfg.width,
            h: cfg.height,
            placeholder: false,
        }
    }

    fn placeholder(lines: &[String], cfg: &config::Config) -> Frame {
        let png = placeholder::render(lines, cfg.width, cfg.height, cfg.invert);
        let preview = placeholder::render(lines, cfg.width, cfg.height, false);
        Frame {
            etag: etag_of(&png),
            png: png.into(),
            preview: preview.into(),
            grey: vec![255; (cfg.width * cfg.height) as usize],
            at: now_unix(),
            w: cfg.width,
            h: cfg.height,
            placeholder: true,
        }
    }
}

fn etag_of(b: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for x in b {
        h ^= *x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("\"{h:016x}\"")
}

/// Something that stops rendering (shown in /status, in the log, and as the placeholder image).
#[derive(Serialize, Clone, Debug)]
pub struct Problem {
    pub kind: String,
    pub message: String,
    pub since: u64,
}

impl Problem {
    pub fn new(kind: &str, message: &str) -> Problem {
        Problem {
            kind: kind.into(),
            message: message.into(),
            since: now_unix(),
        }
    }
}

#[derive(Serialize, Default)]
pub struct Status {
    pub engine_state: String,
    pub engine_pid: Option<i32>,
    pub engine_restarts: u64,
    pub problem: Option<Problem>,
    pub last_error: Option<String>,
    pub last_load_secs: Option<f64>,
    pub frames_received: u64,
    pub frames_suppressed: u64,
    pub images_changed: u64,
    pub last_frame_at: Option<u64>,
    pub last_render_at: Option<u64>,
    pub last_convert_ms: u64,
    pub last_ping: Option<u64>,
    pub http_200: u64,
    pub http_304: u64,
    pub started_at: u64,
    pub version: &'static str,
}

pub struct Shared {
    pub frame: RwLock<Option<Arc<Frame>>>,
    pub status: Mutex<Status>,
    cfg: RwLock<Arc<config::Config>>,
    cfg_tx: tokio::sync::watch::Sender<u64>,
    sources: config::Sources,
    store: Mutex<store::Store>,
    announcer: Mutex<Option<mdns::Announcer>>,
    addrs: Vec<IpAddr>,
}

impl Shared {
    pub fn cfg(&self) -> Arc<config::Config> {
        self.cfg.read().unwrap().clone()
    }

    pub fn cfg_watch(&self) -> tokio::sync::watch::Receiver<u64> {
        self.cfg_tx.subscribe()
    }

    pub fn set_state(&self, st: &str, pid: Option<i32>) {
        let mut s = self.status.lock().unwrap();
        s.engine_state = st.into();
        if pid.is_some() {
            s.engine_pid = pid;
        }
    }

    /// Recompute the effective config from the process sources + stored settings and notify the engine.
    fn reconfigure(&self) -> Result<(), String> {
        let settings = self.store.lock().unwrap().state.settings.clone();
        let c = self.sources.build(Some(&settings))?;
        *self.cfg.write().unwrap() = Arc::new(c);
        self.cfg_tx.send_modify(|g| *g += 1);
        self.announce();
        Ok(())
    }

    fn renderer_address(&self) -> String {
        let port = self.cfg().port();
        match self.addrs.first() {
            Some(ip) => format!("{ip}:{port}"),
            None => format!("this computer's IP address, port {port} (or the port you published with -p)"),
        }
    }

    fn announce(&self) {
        let cfg = self.cfg();
        let mut guard = self.announcer.lock().unwrap();
        let Some(a) = guard.as_mut() else { return };
        let st = self.store.lock().unwrap().state.clone();
        let props = [
            ("id", st.id.clone()),
            ("version", env!("CARGO_PKG_VERSION").to_string()),
            ("paired", (st.paired() as u8).to_string()),
            ("configured", (cfg.is_configured() as u8).to_string()),
            ("api", "1".to_string()),
        ];
        if let Err(e) = a.announce(&st.id, &cfg.name, cfg.port(), &self.addrs, &props) {
            log(&format!("mDNS: {e}"));
        }
    }

    /// Generated image telling the user how to finish setup (shown until the first real frame).
    pub fn show_setup_placeholder(&self) {
        let cfg = self.cfg();
        let mut cur = self.frame.write().unwrap();
        if cur.as_ref().map(|f| !f.placeholder).unwrap_or(false) {
            return; // keep a real (cached) image on the display
        }
        *cur = Some(Arc::new(Frame::placeholder(
            &[
                "Ready to set up".into(),
                "Open Home Assistant > Settings > Devices & services. Add \"E-ink Renderer\" (it may already be listed as discovered).".into(),
                format!("Renderer address: {}", self.renderer_address()),
            ],
            &cfg,
        )));
    }

    /// Record a problem; log it when it changes; show it as the image if no real frame was rendered yet.
    pub fn problem(&self, p: Problem) {
        let mut s = self.status.lock().unwrap();
        let changed = s.problem.as_ref().map(|o| o.message != p.message).unwrap_or(true);
        if changed {
            log(&format!("PROBLEM [{}]: {}", p.kind, p.message));
        }
        let keep_since = s.problem.as_ref().filter(|o| o.message == p.message).map(|o| o.since);
        let mut p = p;
        if let Some(t) = keep_since {
            p.since = t;
        }
        s.problem = Some(p.clone());
        drop(s);
        let cfg = self.cfg();
        let mut cur = self.frame.write().unwrap();
        if cur.as_ref().map(|f| f.placeholder).unwrap_or(true) {
            let title = match p.kind.as_str() {
                "auth" | "token" => "Access token rejected",
                "unreachable" => "Cannot reach Home Assistant",
                "dashboard" => "Dashboard not found",
                "view" => "View not found",
                _ => "Not rendering yet",
            };
            *cur = Some(Arc::new(Frame::placeholder(
                &[
                    title.to_string(),
                    p.message.clone(),
                    format!("Details: http://{}/status", self.renderer_address()),
                ],
                &cfg,
            )));
        }
    }

    pub fn clear_problem(&self) {
        let mut s = self.status.lock().unwrap();
        if let Some(p) = s.problem.take() {
            log(&format!("resolved [{}]", p.kind));
        }
    }

    pub fn publish(&self, f: Frame, convert_ms: u64) {
        let cfg = self.cfg();
        let mut s = self.status.lock().unwrap();
        s.frames_received += 1;
        s.last_frame_at = Some(f.at);
        s.last_convert_ms = convert_ms;
        let mut cur = self.frame.write().unwrap();
        if cur.as_ref().map(|c| c.etag != f.etag || c.placeholder).unwrap_or(true) {
            s.images_changed += 1;
            s.last_render_at = Some(f.at);
            if let Some(p) = &cfg.cache_file {
                let tmp = format!("{p}.tmp");
                if std::fs::write(&tmp, &f.png)
                    .and_then(|_| std::fs::rename(&tmp, p))
                    .is_err()
                {
                    log(&format!("warning: could not write cache file {p}"));
                }
            }
            *cur = Some(Arc::new(f));
        }
    }
}

fn resp(code: StatusCode, ctype: &str, body: Bytes) -> Response<Full<Bytes>> {
    let len = body.len();
    let mut r = Response::new(Full::new(body));
    *r.status_mut() = code;
    let h = r.headers_mut();
    h.insert(CONTENT_TYPE, HeaderValue::from_str(ctype).unwrap());
    h.insert(CONTENT_LENGTH, HeaderValue::from(len));
    h.insert(CONNECTION, HeaderValue::from_static("close"));
    h.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    r
}

fn json_resp(code: StatusCode, v: &serde_json::Value) -> Response<Full<Bytes>> {
    let mut r = resp(
        code,
        "application/json",
        Bytes::from(serde_json::to_vec_pretty(v).unwrap()),
    );
    r.headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

/// Renderer identity and pairing state. `full` (API key holders only) adds who it is paired with and where it runs.
fn setup_json(sh: &Shared, full: bool) -> serde_json::Value {
    let cfg = sh.cfg();
    let st = sh.store.lock().unwrap();
    let mut v = serde_json::json!({
        "id": st.state.id,
        "name": cfg.name,
        "version": env!("CARGO_PKG_VERSION"),
        "api": 1,
        "configured": cfg.is_configured(),
        "paired": st.state.paired(),
        "pairing_available": st.persistent() && !st.state.paired(),
        "persistent": st.persistent(),
        "locked": cfg.locked,
        "port": cfg.port(),
    });
    if full {
        v["paired_with"] = serde_json::json!(st.state.paired_with);
        v["paired_at"] = serde_json::json!(st.state.paired_at);
        v["addresses"] = serde_json::json!(sh.addrs.iter().map(|a| a.to_string()).collect::<Vec<_>>());
    }
    v
}

fn config_json(c: &config::Config) -> serde_json::Value {
    serde_json::json!({"ha_url": c.ha_url, "dashboard_path": c.dashboard_path, "width": c.width, "height": c.height,
        "header_crop": c.header_crop, "threshold": c.threshold, "invert": c.invert, "dither": c.dither, "theme": c.theme,
        "dark": c.dark, "font_hinting": c.font_hinting, "block_resources": c.block_resources, "timezone": c.timezone, "locked": c.locked})
}

/// `/status` (public): what a display, a health check or a monitoring tool needs, nothing about Home Assistant.
/// `/api/status` (API key): the same plus problem messages, the effective configuration and pairing details.
fn status_json(sh: &Shared, full: bool) -> serde_json::Value {
    let cfg = sh.cfg();
    let setup = setup_json(sh, full);
    let st = sh.status.lock().unwrap();
    let f = sh.frame.read().unwrap();
    let browser = st.engine_pid.and_then(|p| mem::proc_mem(&p.to_string()));
    let mut status = serde_json::to_value(&*st).unwrap_or_default();
    if !full {
        // problem messages and errors can contain Home Assistant URLs and dashboard names
        if let Some(p) = st.problem.as_ref() {
            status["problem"] = serde_json::json!({"kind": p.kind, "since": p.since});
        }
        status["last_error"] = serde_json::Value::Null;
        if let Some(o) = status.as_object_mut() {
            o.remove("engine_pid");
        }
    }
    let mut v = serde_json::json!({
        "status": status,
        "setup": setup,
        "image": f.as_ref().map(|f| serde_json::json!({"etag": f.etag, "bytes": f.png.len(), "rendered_at": f.at,
            "width": f.w, "height": f.h, "placeholder": f.placeholder})),
        "memory": {
            "supervisor": mem::proc_mem("self"),
            "browser": browser,
            "container_pss_kb": mem::container_pss_kb(),
            "cgroup_memory_current_kb": mem::cgroup_current_kb(),
        },
        "now": now_unix(),
    });
    if full {
        v["config"] = config_json(&cfg);
    }
    v
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairRequest {
    /// Shown in "already paired with ..." messages (e.g. the Home Assistant instance name).
    #[serde(default)]
    ha_name: Option<String>,
    settings: config::Settings,
}

async fn read_json<T: serde::de::DeserializeOwned>(req: Request<Incoming>) -> Result<T, Box<Response<Full<Bytes>>>> {
    let bad = |m: String| {
        Box::new(json_resp(
            StatusCode::BAD_REQUEST,
            &serde_json::json!({"error": "bad_request", "message": m}),
        ))
    };
    let body = Limited::new(req.into_body(), 64 * 1024)
        .collect()
        .await
        .map_err(|e| bad(e.to_string()))?
        .to_bytes();
    serde_json::from_slice(&body).map_err(|e| bad(e.to_string()))
}

fn authorized(sh: &Shared, req: &Request<Incoming>) -> bool {
    let given = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let st = sh.store.lock().unwrap();
    st.state
        .api_key
        .as_deref()
        .map(|k| store::key_matches(k, given))
        .unwrap_or(false)
}

/// The configuration API used by the Home Assistant integration.
async fn api(req: Request<Incoming>, sh: Arc<Shared>, path: &str) -> Response<Full<Bytes>> {
    let err =
        |code: StatusCode, kind: &str, msg: &str| json_resp(code, &serde_json::json!({"error": kind, "message": msg}));
    match (req.method().clone(), path) {
        (Method::GET, "/api/info") => json_resp(StatusCode::OK, &setup_json(&sh, false)),
        (Method::GET, "/api/status") if !authorized(&sh, &req) => {
            err(StatusCode::UNAUTHORIZED, "unauthorized", "missing or wrong API key")
        }
        (Method::GET, "/api/status") => json_resp(StatusCode::OK, &status_json(&sh, true)),
        (Method::POST, "/api/pair") => {
            {
                let st = sh.store.lock().unwrap();
                if !st.persistent() {
                    return err(StatusCode::SERVICE_UNAVAILABLE, "no_storage",
                        "This renderer has no writable /data volume, so it cannot remember a pairing. Start it with -v eink-data:/data.");
                }
                if st.state.paired() {
                    return json_resp(
                        StatusCode::CONFLICT,
                        &serde_json::json!({
                        "error": "already_paired",
                        "message": "This renderer is already paired with a Home Assistant. To pair it again, run on its host: docker exec <container> eink-renderer --reset-pairing (then restart the container)."}),
                    );
                }
            }
            let peer = req
                .extensions()
                .get::<std::net::SocketAddr>()
                .map(|a| a.ip().to_string())
                .unwrap_or_else(|| "unknown".into());
            let body: PairRequest = match read_json(req).await {
                Ok(b) => b,
                Err(r) => return *r,
            };
            if body.settings.ha_url.as_deref().unwrap_or("").is_empty()
                || body.settings.token.as_deref().unwrap_or("").is_empty()
            {
                return err(
                    StatusCode::BAD_REQUEST,
                    "bad_request",
                    "settings.ha_url and settings.token are required",
                );
            }
            if let Err(e) = sh.sources.build(Some(&body.settings)) {
                return err(StatusCode::BAD_REQUEST, "invalid_settings", &e);
            }
            let key = store::random_hex(32);
            {
                let mut st = sh.store.lock().unwrap();
                // re-check under the same lock that writes the key: two simultaneous pair requests must not both win
                if st.state.paired() {
                    return err(
                        StatusCode::CONFLICT,
                        "already_paired",
                        "This renderer was just paired by another request.",
                    );
                }
                st.state.api_key = Some(key.clone());
                st.state.paired_with = Some(body.ha_name.clone().unwrap_or_else(|| "Home Assistant".into()));
                st.state.paired_at = Some(now_unix());
                st.state.settings = body.settings;
                if let Err(e) = st.save() {
                    st.state.api_key = None;
                    return err(StatusCode::INTERNAL_SERVER_ERROR, "storage", &e);
                }
            }
            log(&format!(
                "paired with {} (request from {peer})",
                body.ha_name.as_deref().unwrap_or("Home Assistant")
            ));
            if let Err(e) = sh.reconfigure() {
                log(&format!("pairing: {e}"));
            }
            let mut v = setup_json(&sh, true);
            v["api_key"] = serde_json::json!(key);
            v["config"] = config_json(&sh.cfg());
            json_resp(StatusCode::OK, &v)
        }
        (Method::POST, "/api/settings") | (Method::POST, "/api/unpair") if !authorized(&sh, &req) => {
            err(StatusCode::UNAUTHORIZED, "unauthorized", "missing or wrong API key")
        }
        (Method::POST, "/api/settings") => {
            let body: config::Settings = match read_json(req).await {
                Ok(b) => b,
                Err(r) => return *r,
            };
            let mut merged = sh.store.lock().unwrap().state.settings.clone();
            merged.merge(&body);
            if let Err(e) = sh.sources.build(Some(&merged)) {
                return err(StatusCode::BAD_REQUEST, "invalid_settings", &e);
            }
            {
                let mut st = sh.store.lock().unwrap();
                let before = st.state.settings.clone();
                st.state.settings = merged;
                if let Err(e) = st.save() {
                    st.state.settings = before;
                    return err(StatusCode::INTERNAL_SERVER_ERROR, "storage", &e);
                }
            }
            if let Err(e) = sh.reconfigure() {
                return err(StatusCode::BAD_REQUEST, "invalid_settings", &e);
            }
            log("settings updated by Home Assistant");
            json_resp(StatusCode::OK, &serde_json::json!({"config": config_json(&sh.cfg())}))
        }
        (Method::POST, "/api/unpair") => {
            {
                let mut st = sh.store.lock().unwrap();
                st.state.api_key = None;
                st.state.paired_with = None;
                st.state.paired_at = None;
                st.state.settings = config::Settings::default();
                if let Err(e) = st.save() {
                    return err(StatusCode::INTERNAL_SERVER_ERROR, "storage", &e);
                }
            }
            log("unpaired by Home Assistant");
            let _ = sh.reconfigure();
            json_resp(StatusCode::OK, &setup_json(&sh, false))
        }
        _ => err(StatusCode::NOT_FOUND, "not_found", "unknown API endpoint"),
    }
}

async fn handle(req: Request<Incoming>, sh: Arc<Shared>) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    let path = req.uri().path().to_string();
    if path.starts_with("/api/") {
        return Ok(api(req, sh, &path).await);
    }
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return Ok(resp(
            StatusCode::METHOD_NOT_ALLOWED,
            "text/plain",
            Bytes::from_static(b"GET only\n"),
        ));
    }
    let head = req.method() == Method::HEAD;
    let mut r = match path.as_str() {
        "/eink.png" | "/preview.png" | "/grey.png" => {
            let f = sh.frame.read().unwrap().clone();
            match f {
                None => {
                    let mut r = resp(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "text/plain",
                        Bytes::from_static(b"starting\n"),
                    );
                    r.headers_mut().insert("retry-after", HeaderValue::from_static("10"));
                    r
                }
                Some(f) => {
                    let inm = req
                        .headers()
                        .get(IF_NONE_MATCH)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    let tag = match path.as_str() {
                        "/eink.png" => f.etag.clone(),
                        "/preview.png" => format!("\"p{}", &f.etag[1..]),
                        _ => format!("\"g{}", &f.etag[1..]),
                    };
                    if inm.split(',').any(|t| t.trim() == tag || t.trim() == "*") {
                        sh.status.lock().unwrap().http_304 += 1;
                        let mut r = resp(StatusCode::NOT_MODIFIED, "image/png", Bytes::new());
                        r.headers_mut().insert(ETAG, HeaderValue::from_str(&tag).unwrap());
                        r
                    } else {
                        if path == "/eink.png" {
                            sh.status.lock().unwrap().http_200 += 1;
                        }
                        let body = match path.as_str() {
                            "/eink.png" => f.png.clone(),
                            "/preview.png" => f.preview.clone(),
                            _ => Bytes::from(image::encode_grey(&f.grey, f.w, f.h)),
                        };
                        let mut r = resp(StatusCode::OK, "image/png", body);
                        r.headers_mut().insert(ETAG, HeaderValue::from_str(&tag).unwrap());
                        r.headers_mut()
                            .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
                        r
                    }
                }
            }
        }
        "/healthz" => {
            // healthy = a real frame exists and nothing is currently wrong
            let real = sh
                .frame
                .read()
                .unwrap()
                .as_ref()
                .map(|f| !f.placeholder)
                .unwrap_or(false);
            let problem = sh.status.lock().unwrap().problem.clone();
            if !sh.cfg().is_configured() {
                return Ok(resp(
                    StatusCode::OK,
                    "text/plain",
                    Bytes::from_static(b"waiting for setup\n"),
                ));
            }
            match (real, problem) {
                (true, None) => resp(StatusCode::OK, "text/plain", Bytes::from_static(b"ok\n")),
                (_, Some(p)) => resp(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "text/plain",
                    Bytes::from(format!(
                        "problem: {} (details in Home Assistant or the container log)\n",
                        p.kind
                    )),
                ),
                (false, None) => resp(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "text/plain",
                    Bytes::from_static(b"starting\n"),
                ),
            }
        }
        "/status" | "/status.json" => {
            let body = serde_json::to_vec_pretty(&status_json(&sh, false)).unwrap_or_default();
            let mut r = resp(StatusCode::OK, "application/json", Bytes::from(body));
            r.headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            r
        }
        "/" => resp(
            StatusCode::OK,
            "text/html; charset=utf-8",
            Bytes::from_static(include_bytes!("index.html")),
        ),
        _ => resp(StatusCode::NOT_FOUND, "text/plain", Bytes::from_static(b"not found\n")),
    };
    if head {
        let h = r.headers().clone();
        let mut n = Response::new(Full::new(Bytes::new()));
        *n.status_mut() = r.status();
        *n.headers_mut() = h;
        r = n;
    }
    Ok(r)
}

/// `eink-renderer --healthcheck`: exit 0 if /healthz answers 200 (the image has no curl).
fn healthcheck() -> i32 {
    use std::io::{Read, Write};
    let port = config::Sources::from_process()
        .ok()
        .and_then(|s| s.build(None).ok())
        .map(|c| c.port())
        .unwrap_or(10001);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut s) = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(3)) else {
        return 1;
    };
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(3)));
    if s.write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .is_err()
    {
        return 1;
    }
    let mut b = [0u8; 16];
    match s.read(&mut b) {
        Ok(n) if n >= 12 && &b[9..12] == b"200" => 0,
        _ => 1,
    }
}

/// `eink-renderer --reset-pairing`: forget the Home Assistant pairing (needs access to the container).
fn reset_pairing() -> i32 {
    let sources = match config::Sources::from_process() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let Some(dir) = sources.data_dir() else {
        eprintln!("no data directory (/data) found: nothing to reset");
        return 1;
    };
    match store::Store::open(Some(&dir)) {
        Ok(mut s) => {
            s.state.api_key = None;
            s.state.paired_with = None;
            s.state.paired_at = None;
            s.state.settings = config::Settings::default();
            match s.save() {
                Ok(()) => {
                    println!("Pairing removed. Restart the container; it will wait to be set up again.");
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--healthcheck") => std::process::exit(healthcheck()),
        Some("--reset-pairing") => std::process::exit(reset_pairing()),
        Some("--version") => {
            println!("eink-renderer {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some("--help") | Some("-h") => {
            println!(
                "eink-renderer {}\nusage: eink-renderer [CONFIG_FILE] | --healthcheck | --reset-pairing | --version\n\
                 Without configuration it waits to be set up from Home Assistant (E-ink Renderer integration).\n\
                 Headless: configure with environment variables (see README) or a TOML file.",
                env!("CARGO_PKG_VERSION")
            );
            return;
        }
        _ => {}
    }
    let fail = |e: String| -> ! {
        log(&format!("configuration error: {e}"));
        std::process::exit(2)
    };
    let sources = config::Sources::from_process().unwrap_or_else(|e| fail(e));
    let data_dir = sources.data_dir();
    let store = store::Store::open(data_dir.as_deref()).unwrap_or_else(|e| fail(e));
    let mut sources = sources;
    if let Some(d) = data_dir.as_ref().filter(|_| store.persistent()) {
        // tell the config layer where /data is (default cache file location)
        sources.env.entry("DATA_DIR".into()).or_insert_with(|| d.clone());
    }
    let cfg = sources.build(Some(&store.state.settings)).unwrap_or_else(|e| fail(e));
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async_main(cfg, sources, store));
}

async fn async_main(cfg: config::Config, sources: config::Sources, store: store::Store) {
    let addrs = mdns::lan_addresses(cfg.advertise_ip.as_deref());
    let (cfg_tx, _) = tokio::sync::watch::channel(0u64);
    let persistent = store.persistent();
    let shared = Arc::new(Shared {
        frame: RwLock::new(None),
        status: Mutex::new(Status {
            started_at: now_unix(),
            engine_state: "init".into(),
            version: env!("CARGO_PKG_VERSION"),
            ..Default::default()
        }),
        cfg: RwLock::new(Arc::new(cfg.clone())),
        cfg_tx,
        sources,
        store: Mutex::new(store),
        announcer: Mutex::new(None),
        addrs,
    });
    let cached = cfg
        .cache_file
        .as_ref()
        .and_then(|p| std::fs::read(p).ok().map(|b| (p, b)));
    if let Some((p, png)) = cached.filter(|_| cfg.is_configured()) {
        let (w, h) = (cfg.width, cfg.height);
        *shared.frame.write().unwrap() = Some(Arc::new(Frame {
            etag: etag_of(&png),
            preview: Bytes::from(png.clone()),
            png: Bytes::from(png),
            grey: vec![255; (w * h) as usize],
            at: now_unix(),
            w,
            h,
            placeholder: false,
        }));
        log(&format!("serving cached image from {p} until the first render"));
    } else if cfg.is_configured() {
        *shared.frame.write().unwrap() = Some(Arc::new(Frame::placeholder(
            &[
                "Starting...".into(),
                format!("Rendering {} for the first time.", cfg.page_url()),
            ],
            &cfg,
        )));
    }
    let listener = tokio::net::TcpListener::bind(&cfg.listen).await.unwrap_or_else(|e| {
        log(&format!("cannot listen on {}: {e}", cfg.listen));
        std::process::exit(2)
    });
    if cfg.advertise && shared.addrs.is_empty() {
        log(
            "mDNS: no LAN address visible (Docker bridge network?). Home Assistant will not discover this renderer; \
             add it by address, or run with --network host for discovery.",
        );
    } else if cfg.advertise {
        match mdns::Announcer::new() {
            Ok(a) => {
                *shared.announcer.lock().unwrap() = Some(a);
                shared.announce();
            }
            Err(e) => log(&format!("mDNS unavailable: {e}")),
        }
    }
    if cfg.is_configured() {
        log(&format!(
            "eink-renderer {} listening on {}: {} ({}x{}, theme '{}', dither {}, threshold {}, invert {})",
            env!("CARGO_PKG_VERSION"),
            cfg.listen,
            cfg.page_url(),
            cfg.width,
            cfg.height,
            cfg.theme,
            cfg.dither,
            cfg.threshold,
            cfg.invert
        ));
    } else {
        log(&format!(
            "eink-renderer {} listening on {}: waiting to be set up from Home Assistant (Settings > Devices & services > E-ink Renderer, address {}){}",
            env!("CARGO_PKG_VERSION"), cfg.listen, shared.renderer_address(),
            if persistent { "" } else { ". WARNING: no /data volume, pairing would not survive a restart (-v eink-data:/data)" }
        ));
    }
    tokio::spawn(engine::run(shared.clone()));
    // at most this many connections at once; each is closed after 30 s regardless
    let slots = Arc::new(tokio::sync::Semaphore::new(64));
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
    loop {
        tokio::select! {
            a = listener.accept() => {
                let Ok((stream, peer)) = a else { continue };
                let Ok(permit) = slots.clone().try_acquire_owned() else { continue }; // busy: drop the connection
                let sh = shared.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let io = hyper_util::rt::TokioIo::new(stream);
                    let svc = hyper::service::service_fn(move |mut req: Request<Incoming>| {
                        req.extensions_mut().insert(peer);
                        handle(req, sh.clone())
                    });
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(30),
                        hyper::server::conn::http1::Builder::new().keep_alive(false).serve_connection(io, svc)).await;
                });
            }
            _ = term.recv() => { log("SIGTERM, exiting"); std::process::exit(0); }
            _ = tokio::signal::ctrl_c() => { std::process::exit(0); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_formatting() {
        assert_eq!(fmt_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(fmt_utc(1_795_152_398), "2026-11-20T05:26:38Z");
    }

    #[test]
    fn etag_is_quoted_and_stable() {
        assert_eq!(etag_of(b"abc"), etag_of(b"abc"));
        assert_ne!(etag_of(b"abc"), etag_of(b"abd"));
        assert!(etag_of(b"").starts_with('"') && etag_of(b"").ends_with('"'));
    }
}
