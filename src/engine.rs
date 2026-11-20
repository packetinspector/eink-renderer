//! Browser supervisor: owns chrome-headless-shell, drives Home Assistant over CDP, turns screencast frames into
//! the served PNG, and diagnoses start-up problems (unreachable HA, rejected token, unknown dashboard/view).
use crate::cdp::{Cdp, Event};
use crate::config::Config;
use crate::image::{self, Dither, Params};
use crate::{log, Frame, Problem, Shared};
use base64::Engine as _;
use serde_json::{json, Value};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::unix::pipe;
use tokio::process::{Child, Command};

/// Evaluated repeatedly after navigation until it reports `ok` or a definite problem.
const DIAG_JS: &str = r#"(async (DASH, VIEW) => {
  const p = location.pathname;
  if (p.startsWith('/auth/') || p.startsWith('/onboarding')) return {state: 'auth', detail: p};
  if (window.__einkApi === undefined) {
    try {
      const t = JSON.parse(localStorage.getItem('hassTokens') || '{}').access_token;
      window.__einkApi = (await fetch('/api/', {headers: {Authorization: 'Bearer ' + t}})).status;
    } catch (e) { window.__einkApi = 'fetch failed: ' + e; }
  }
  if (window.__einkApi === 401 || window.__einkApi === 403) return {state: 'auth', detail: 'HTTP ' + window.__einkApi};
  const ha = document.querySelector('home-assistant');
  if (!ha && document.readyState === 'complete') {
    if (window.__einkPage === undefined) {
      try { window.__einkPage = (await fetch(location.href)).status; } catch (e) { window.__einkPage = 0; }
    }
    if (window.__einkPage === 404) return {state: 'no_dashboard', detail: '(Home Assistant answered 404 for this path)'};
  }
  const hass = ha && ha.hass;
  if (!hass || !hass.panels) return {state: 'booting', detail: String(window.__einkApi)};
  if (!hass.panels[DASH]) return {state: 'no_dashboard',
    detail: Object.keys(hass.panels).filter(k => hass.panels[k].component_name === 'lovelace').map(k => '/' + k).join(', ')};
  const m = ha.shadowRoot && ha.shadowRoot.querySelector('home-assistant-main');
  const r = m && m.shadowRoot && m.shadowRoot.querySelector('partial-panel-resolver');
  if (!r || r._loading) return {state: 'loading'};
  const panel = r.children[0];
  if (!panel || (('_loading' in panel) && panel._loading)) return {state: 'loading'};
  const views = panel.lovelace && panel.lovelace.config && panel.lovelace.config.views;
  if (VIEW && Array.isArray(views)) {
    const ok = /^\d+$/.test(VIEW) ? Number(VIEW) < views.length : views.some(v => v.path === VIEW);
    if (!ok) return {state: 'no_view', detail: views.map((v, i) => v.path || String(i)).join(', ')};
  }
  return {state: 'ok'};
})"#;

/// A frame is published only while HA is connected and fully started (no "Home Assistant is starting" placeholders).
const FRAME_OK_JS: &str = r#"(() => { const h = document.querySelector('home-assistant'); const s = h && h.hass;
  return !!(s && s.connected && (!s.config || !s.config.state || s.config.state === 'RUNNING')); })()"#;

const TOAST_JS: &str = r#"(() => { const ha=document.querySelector('home-assistant'); const n=ha&&ha.shadowRoot&&ha.shadowRoot.querySelector('notification-manager');
 const t=n&&n.shadowRoot&&n.shadowRoot.querySelector('ha-toast'); return !!(t && (t.open===true || t.hasAttribute('open'))); })()"#;

fn chrome_args(cfg: &Config) -> Vec<String> {
    let mut a: Vec<String> = [
        "--headless",
        "--no-sandbox",
        "--remote-debugging-pipe",
        "--single-process",
        "--no-zygote",
        "--disable-gpu",
        "--use-gl=disabled",
        "--enable-low-end-device-mode",
        "--js-flags=--lite-mode --optimize-for-size --max-semi-space-size=1",
        "--hide-scrollbars",
        "--mute-audio",
        "--no-first-run",
        "--disable-background-networking",
        "--disable-component-update",
        "--disable-sync",
        "--disable-default-apps",
        "--disable-extensions",
        "--disable-breakpad",
        "--disable-dev-shm-usage",
        "--disable-background-timer-throttling",
        "--disable-renderer-backgrounding",
        "--disable-backgrounding-occluded-windows",
        "--disable-features=Translate,OptimizationHints,MediaRouter,BackForwardCache,AutofillServerCommunication,CertificateTransparencyComponentUpdater",
        "--force-color-profile=srgb",
        "--disk-cache-size=1048576",
        "--user-data-dir=/tmp/chrome-profile",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.push(format!("--font-render-hinting={}", cfg.font_hinting));
    a.push(format!("--window-size={},{}", cfg.width, cfg.height + cfg.header_crop));
    a.extend(cfg.chrome_extra_flags.iter().cloned());
    a.push("about:blank".into());
    a
}

fn mk_pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

fn set_nonblock(fd: &OwnedFd) {
    use std::os::fd::AsRawFd;
    unsafe {
        let f = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
        libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, f | libc::O_NONBLOCK);
    }
}

struct Browser {
    child: Child,
    pid: i32,
}
impl Drop for Browser {
    fn drop(&mut self) {
        // also covers the session future being dropped when settings change
        unsafe {
            libc::kill(-self.pid, libc::SIGKILL);
        }
    }
}

impl Browser {
    fn kill(&mut self) {
        unsafe {
            libc::kill(-self.pid, libc::SIGKILL);
        }
        let _ = self.child.start_kill();
    }
}

fn spawn(cfg: &Config) -> Result<(Browser, Cdp, tokio::sync::mpsc::UnboundedReceiver<Event>), String> {
    use std::os::fd::AsRawFd;
    let (to_chrome_r, to_chrome_w) = mk_pipe().map_err(|e| e.to_string())?;
    let (from_chrome_r, from_chrome_w) = mk_pipe().map_err(|e| e.to_string())?;
    let (cr, cw) = (to_chrome_r.as_raw_fd(), from_chrome_w.as_raw_fd());
    let mut cmd = std::process::Command::new(&cfg.chrome_path);
    cmd.args(chrome_args(cfg))
        .env("HOME", "/tmp")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    unsafe {
        cmd.pre_exec(move || {
            libc::setsid();
            if libc::dup2(cr, 3) < 0 || libc::dup2(cw, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = Command::from(cmd)
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", cfg.chrome_path))?;
    drop(to_chrome_r);
    drop(from_chrome_w);
    set_nonblock(&to_chrome_w);
    set_nonblock(&from_chrome_r);
    let w = pipe::Sender::from_owned_fd(to_chrome_w).map_err(|e| e.to_string())?;
    let r = pipe::Receiver::from_owned_fd(from_chrome_r).map_err(|e| e.to_string())?;
    let pid = child.id().unwrap_or(0) as i32;
    let (cdp, ev) = Cdp::start(w, r);
    Ok((Browser { child, pid }, cdp, ev))
}

fn localstorage_script(cfg: &Config, token: &str) -> String {
    let ha = cfg.ha_url.trim_end_matches('/');
    let tokens = json!({"access_token": token, "token_type": "Bearer", "expires_in": 1800, "hassUrl": ha,
                        "clientId": format!("{ha}/"), "expires": 9999999999999u64, "refresh_token": ""});
    let theme = if cfg.theme.is_empty() {
        json!({"dark": cfg.dark})
    } else {
        json!({"theme": cfg.theme, "dark": cfg.dark})
    };
    let items = json!({"dockedSidebar": "\"always_hidden\"", "selectedTheme": theme.to_string(), "hassTokens": tokens.to_string()});
    format!("(() => {{ if (location.origin !== new URL({o}).origin) return; const d = {items}; for (const k in d) localStorage.setItem(k, d[k]); }})();",
            o = json!(ha), items = items)
}

/// Engine loop. Uses the current effective config; restarts the browser session whenever the config changes
/// (Home Assistant changed a setting) and idles in "setup" while there is nothing to render.
pub async fn run(shared: Arc<Shared>) {
    let mut rx = shared.cfg_watch();
    let mut backoff = 1u64;
    loop {
        rx.borrow_and_update();
        let cfg = shared.cfg();
        if !cfg.is_configured() {
            shared.set_state("setup", None);
            shared.show_setup_placeholder();
            let _ = rx.changed().await;
            backoff = 1;
            continue;
        }
        let t = Instant::now();
        shared.set_state("starting", None);
        let r = tokio::select! {
            r = session(&cfg, &shared) => Some(r),
            _ = rx.changed() => None,
        };
        let Some(r) = r else {
            log("engine: settings changed; restarting the browser with the new settings");
            shared.clear_problem();
            backoff = 1;
            continue;
        };
        let msg = match &r {
            Ok(()) => "browser session ended".to_string(),
            Err(e) => e.clone(),
        };
        if t.elapsed() > Duration::from_secs(600) {
            backoff = 1;
        }
        log(&format!("engine: {msg}; retrying in {backoff}s"));
        {
            let mut s = shared.status.lock().unwrap();
            s.engine_restarts += 1;
            s.last_error = Some(msg);
            s.engine_pid = None;
            s.engine_state = "restarting".into();
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(backoff)) => { backoff = (backoff * 2).min(60); }
            _ = rx.changed() => { backoff = 1; }
        }
    }
}

async fn session(cfg: &Config, shared: &Arc<Shared>) -> Result<(), String> {
    let token = match cfg.read_token() {
        Ok(t) => t,
        Err(e) => {
            shared.problem(Problem::new("token", &format!("Cannot read the access token: {e}")));
            return Err(e);
        }
    };
    let (mut br, cdp, mut ev) = spawn(cfg).inspect_err(|e| shared.problem(Problem::new("browser", e)))?;
    shared.status.lock().unwrap().engine_pid = Some(br.pid);
    let res = drive(cfg, shared, &cdp, &mut ev, &mut br, &token).await;
    br.kill();
    let _ = tokio::time::timeout(Duration::from_secs(5), br.child.wait()).await;
    res
}

fn path_segments(path: &str) -> (String, String) {
    let mut it = path
        .split('?')
        .next()
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty());
    (it.next().unwrap_or("").to_string(), it.next().unwrap_or("").to_string())
}

/// Problem for a diagnosis state that will not fix itself (token rejected, dashboard or view missing).
fn target_problem(state: &str, detail: &str, dash: &str, view: &str) -> Option<Problem> {
    match state {
        "auth" => Some(Problem::new(
            "auth",
            &format!(
                "Home Assistant rejected the access token ({detail}). With the integration: remove and add the \
                 renderer again. Headless: create a long-lived access token on your HA profile page and set \
                 HA_TOKEN / HA_TOKEN_FILE."
            ),
        )),
        "no_dashboard" => Some(Problem::new(
            "dashboard",
            &format!("Dashboard '/{dash}' does not exist. Available dashboards: {detail}"),
        )),
        "no_view" => Some(Problem::new(
            "view",
            &format!(
                "View '{view}' does not exist on dashboard '/{dash}' (deleted or renamed?). Choose another view \
                 under Configure in Home Assistant. Views: {detail}"
            ),
        )),
        _ => None,
    }
}

async fn diagnose(
    cfg: &Config,
    shared: &Arc<Shared>,
    cdp: &Cdp,
    s: Option<&str>,
    ev: &mut tokio::sync::mpsc::UnboundedReceiver<Event>,
) -> Result<(), String> {
    let (dash, view) = path_segments(&cfg.dashboard_path);
    let expr = format!("{DIAG_JS}({}, {})", json!(dash), json!(view));
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        while let Ok(e) = ev.try_recv() {
            handle_aux(cdp, &e).await;
        }
        let v = cdp
            .call(
                s,
                "Runtime.evaluate",
                json!({"expression": expr, "returnByValue": true, "awaitPromise": true}),
                15,
            )
            .await?;
        let r = &v["result"]["value"];
        let state = r["state"].as_str().unwrap_or("booting").to_string();
        let detail = r["detail"].as_str().unwrap_or("").to_string();
        if state == "ok" {
            return Ok(());
        }
        let problem = target_problem(&state, &detail, &dash, &view);
        if let Some(p) = problem {
            let m = p.message.clone();
            shared.problem(p);
            return Err(m);
        }
        let last = format!("{state} {detail}");
        if Instant::now() > deadline {
            let m = format!(
                "{} loaded but never became a Home Assistant dashboard (last state: {}). Check the Home Assistant address and the dashboard path.",
                cfg.page_url(),
                last.trim()
            );
            shared.problem(Problem::new("timeout", &m));
            return Err(m);
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

async fn drive(
    cfg: &Config,
    shared: &Arc<Shared>,
    cdp: &Cdp,
    ev: &mut tokio::sync::mpsc::UnboundedReceiver<Event>,
    br: &mut Browser,
    token: &str,
) -> Result<(), String> {
    let t0 = Instant::now();
    let pid = br.pid;
    let targets = cdp.call(None, "Target.getTargets", json!({}), 20).await?;
    let tid = targets["targetInfos"]
        .as_array()
        .and_then(|a| a.iter().find(|t| t["type"] == "page"))
        .and_then(|t| t["targetId"].as_str())
        .map(String::from);
    let tid = match tid {
        Some(t) => t,
        None => cdp
            .call(None, "Target.createTarget", json!({"url": "about:blank"}), 20)
            .await?["targetId"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    };
    let sid = cdp
        .call(
            None,
            "Target.attachToTarget",
            json!({"targetId": tid, "flatten": true}),
            20,
        )
        .await?["sessionId"]
        .as_str()
        .ok_or("no sessionId")?
        .to_string();
    let s = Some(sid.as_str());
    cdp.call(s, "Page.enable", json!({}), 20).await?;
    cdp.call(
        s,
        "Emulation.setDeviceMetricsOverride",
        json!({"width": cfg.width, "height": cfg.height + cfg.header_crop, "deviceScaleFactor": 1, "mobile": false}),
        20,
    )
    .await?;
    if !cfg.timezone.is_empty() {
        if let Err(err) = cdp
            .call(
                s,
                "Emulation.setTimezoneOverride",
                json!({"timezoneId": cfg.timezone}),
                20,
            )
            .await
        {
            log(&format!("engine: cannot apply time zone {}: {err}", cfg.timezone));
        }
    }
    cdp.call(
        s,
        "Page.addScriptToEvaluateOnNewDocument",
        json!({"source": localstorage_script(cfg, token)}),
        20,
    )
    .await?;
    let mut pats: Vec<Value> = cfg
        .block_resources
        .iter()
        .map(|p| json!({"urlPattern": format!("*{p}*"), "requestStage": "Request"}))
        .collect();
    if !is_local_url(&cfg.ha_url) {
        // The page may only reach what Home Assistant itself is: never this host's loopback services or
        // link-local/metadata addresses (protects --network host setups from a malicious "dashboard").
        pats.extend(
            LOCAL_PATTERNS
                .iter()
                .map(|p| json!({"urlPattern": p, "requestStage": "Request"})),
        );
    }
    if !pats.is_empty() {
        cdp.call(s, "Fetch.enable", json!({"patterns": pats}), 20).await?;
    }
    shared.set_state("loading", Some(pid));
    let nav = match cdp.call(s, "Page.navigate", json!({"url": cfg.page_url()}), 30).await {
        Ok(v) => v,
        Err(e) => json!({"errorText": e}),
    };
    if let Some(e) = nav.get("errorText").and_then(|e| e.as_str()) {
        let m = format!(
            "Cannot reach Home Assistant at {} ({e}). Check the address and that the renderer's computer can reach it.",
            cfg.ha_url
        );
        shared.problem(Problem::new("unreachable", &m));
        return Err(m);
    }
    diagnose(cfg, shared, cdp, s, ev).await?;
    let load_s = t0.elapsed().as_secs_f64();
    log(&format!(
        "engine: browser pid {pid} loaded {} in {load_s:.2}s",
        cfg.page_url()
    ));
    shared.status.lock().unwrap().last_load_secs = Some(load_s);
    shared.clear_problem();
    tokio::time::sleep(Duration::from_millis(800)).await;
    cdp.call(
        s,
        "Page.startScreencast",
        json!({"format": "png", "everyNthFrame": 1, "maxWidth": cfg.width, "maxHeight": cfg.height + cfg.header_crop}),
        20,
    )
    .await?;
    shared.set_state("running", Some(pid));

    let params = Arc::new(Params {
        width: cfg.width,
        height: cfg.height,
        top: cfg.header_crop,
        threshold: cfg.threshold,
        invert: cfg.invert,
        dither: Dither::parse(&cfg.dither),
    });
    let mut ping = tokio::time::interval(Duration::from_secs(cfg.ping_interval_secs));
    ping.tick().await;
    let mut last_frame = Instant::now();
    let mut stale_reloads = 0u32;
    // the configured dashboard/view can disappear while we run (deleted in Home Assistant): check on every ping
    let (dash, view) = path_segments(&cfg.dashboard_path);
    let diag_expr = format!("{DIAG_JS}({}, {})", json!(dash), json!(view));
    let mut target_missing = false;
    let mut toast_ticks = 0u32;
    loop {
        tokio::select! {
            e = ev.recv() => {
                let e = e.ok_or("devtools pipe closed (browser exited)")?;
                match e.method.as_str() {
                    "Page.screencastFrame" => {
                        let fsid = e.params["sessionId"].clone();
                        let data = e.params["data"].as_str().unwrap_or("").to_string();
                        // the configured view must still exist before a frame may reach the display
                        if let Ok(d) = cdp.call(s, "Runtime.evaluate", json!({"expression": diag_expr, "returnByValue": true, "awaitPromise": true}), 10).await {
                            let r = &d["result"]["value"];
                            if let Some(p) = target_problem(r["state"].as_str().unwrap_or(""), r["detail"].as_str().unwrap_or(""), &dash, &view) {
                                if !target_missing {
                                    log("engine: the configured dashboard/view is gone; keeping the last image");
                                }
                                target_missing = true;
                                shared.problem(p);
                            }
                        }
                        let ok = !target_missing && cdp.call(s, "Runtime.evaluate", json!({"expression": FRAME_OK_JS, "returnByValue": true}), 10).await
                            .map(|v| v["result"]["value"] == json!(true)).unwrap_or(true);
                        let p = params.clone();
                        let t = Instant::now();
                        let out = if ok {
                            Some(tokio::task::spawn_blocking(move || -> Result<(Vec<u8>, Vec<u8>), String> {
                                let png = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string())?;
                                let grey = image::decode_grey(&png, &p)?;
                                let bits = image::to_bits(&grey, &p);
                                Ok((image::encode_1bit(&bits, p.width, p.height), grey))
                            }).await.map_err(|e| e.to_string())?)
                        } else { None };
                        // ack after processing: Chrome sends the next frame only after the ack (natural rate limit)
                        let _ = cdp.call(s, "Page.screencastFrameAck", json!({"sessionId": fsid}), 10).await;
                        last_frame = Instant::now();
                        stale_reloads = 0;
                        match out {
                            Some(Ok((png, grey))) => shared.publish(Frame::new(png, grey, cfg), t.elapsed().as_millis() as u64),
                            Some(Err(err)) => log(&format!("engine: frame conversion failed: {err}")),
                            None => shared.status.lock().unwrap().frames_suppressed += 1,
                        }
                    }
                    "Inspector.targetCrashed" => return Err("renderer crashed".into()),
                    "Target.detachedFromTarget" if e.params["sessionId"].as_str() == s => return Err("page detached".into()),
                    _ => handle_aux(cdp, &e).await,
                }
            }
            _ = ping.tick() => {
                let v = cdp.call(s, "Runtime.evaluate", json!({"expression": TOAST_JS, "returnByValue": true}), cfg.ping_timeout_secs).await
                    .map_err(|e| format!("health ping failed: {e}"))?;
                shared.status.lock().unwrap().last_ping = Some(crate::now_unix());
                if let Ok(d) = cdp.call(s, "Runtime.evaluate", json!({"expression": diag_expr, "returnByValue": true, "awaitPromise": true}), cfg.ping_timeout_secs).await {
                    let r = &d["result"]["value"];
                    let state = r["state"].as_str().unwrap_or("");
                    if let Some(p) = target_problem(state, r["detail"].as_str().unwrap_or(""), &dash, &view) {
                        if !target_missing {
                            log("engine: the configured dashboard/view is gone; keeping the last image");
                        }
                        target_missing = true;
                        shared.problem(p);
                    } else if state == "ok" && target_missing {
                        target_missing = false;
                        shared.clear_problem();
                    }
                }
                toast_ticks = if v["result"]["value"] == json!(true) { toast_ticks + 1 } else { 0 };
                let stale = last_frame.elapsed() > Duration::from_secs(cfg.stale_after_secs);
                if stale || toast_ticks >= 3 {
                    if stale_reloads >= 1 {
                        return Err(format!("no new frame for {}s even after reload", last_frame.elapsed().as_secs()));
                    }
                    log(&format!("engine: {}; reloading page", if stale { "no repaint (stale)" } else { "toast stuck on screen" }));
                    stale_reloads += 1;
                    toast_ticks = 0;
                    last_frame = Instant::now();
                    cdp.call(s, "Page.reload", json!({"ignoreCache": false}), 30).await?;
                }
            }
            st = br.child.wait() => {
                return Err(format!("browser exited: {}", st.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string())));
            }
        }
    }
}

const LOCAL_PATTERNS: &[&str] = &[
    "*://127.*",
    "*://localhost*",
    "*://[::1]*",
    "*://0.0.0.0*",
    "*://169.254.*",
    "*://[fe80:*",
];

/// Host part of an http(s) URL (no port), lower-case.
fn url_host(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let hostport = rest.split(['/', '?', '#']).next().unwrap_or("");
    let hostport = hostport.rsplit_once('@').map(|(_, h)| h).unwrap_or(hostport);
    let host = if let Some(v6) = hostport.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        hostport.split(':').next().unwrap_or("")
    };
    host.to_ascii_lowercase()
}

/// Loopback, unspecified or link-local destinations.
fn is_local_url(url: &str) -> bool {
    let h = url_host(url);
    h == "localhost"
        || h.ends_with(".localhost")
        || h.starts_with("127.")
        || h == "::1"
        || h == "0.0.0.0"
        || h.starts_with("169.254.")
        || h.starts_with("fe80:")
}

async fn handle_aux(cdp: &Cdp, e: &Event) {
    if e.method == "Fetch.requestPaused" {
        if is_local_url(e.params["request"]["url"].as_str().unwrap_or("")) {
            let _ = cdp
                .call(
                    e.session.as_deref(),
                    "Fetch.failRequest",
                    json!({"requestId": e.params["requestId"], "errorReason": "BlockedByClient"}),
                    10,
                )
                .await;
            return;
        }
        let body = base64::engine::general_purpose::STANDARD.encode("/* blocked by eink-renderer (BLOCK_RESOURCES) */");
        let _ = cdp.call(e.session.as_deref(), "Fetch.fulfillRequest", json!({
            "requestId": e.params["requestId"], "responseCode": 200,
            "responseHeaders": [{"name": "Content-Type", "value": "text/javascript"}, {"name": "Cache-Control", "value": "no-store"}],
            "body": body}), 10).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_destinations_are_recognised() {
        for u in [
            "http://127.0.0.1:9119/",
            "http://localhost:8000/x",
            "http://[::1]:80/",
            "http://169.254.169.254/latest/meta-data",
            "http://user:pw@127.0.0.2/",
            "http://LOCALHOST/",
            "http://0.0.0.0:1/",
        ] {
            assert!(is_local_url(u), "{u}");
        }
        for u in [
            "http://192.168.0.10:8123/",
            "https://ha.example.org/lovelace/0",
            "http://homeassistant.local:8123",
            "http://127example.com/",
        ] {
            assert!(!is_local_url(u), "{u}");
        }
        assert_eq!(url_host("http://[fe80::1]:8123/x"), "fe80::1");
    }

    #[test]
    fn dashboard_path_segments() {
        assert_eq!(path_segments("/lovelace/0"), ("lovelace".into(), "0".into()));
        assert_eq!(
            path_segments("/kitchen-display/today?kiosk"),
            ("kitchen-display".into(), "today".into())
        );
        assert_eq!(path_segments("/dashboard-x"), ("dashboard-x".into(), "".into()));
    }

    #[test]
    fn chrome_args_include_hinting_and_viewport() {
        let env: std::collections::HashMap<String, String> = [
            ("HA_URL", "http://h"),
            ("HA_TOKEN", "t"),
            ("FONT_HINTING", "full"),
            ("HEIGHT", "300"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let c = Config::from_sources(None, &env, None).unwrap();
        let a = chrome_args(&c);
        assert!(a.contains(&"--font-render-hinting=full".to_string()));
        assert!(a.contains(&"--window-size=800,356".to_string()));
        assert_eq!(a.last().unwrap(), "about:blank");
    }

    #[test]
    fn localstorage_script_targets_ha_origin_only() {
        let env: std::collections::HashMap<String, String> =
            [("HA_URL", "http://h:8123/"), ("HA_TOKEN", "tok"), ("THEME", "E-ink")]
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
        let c = Config::from_sources(None, &env, None).unwrap();
        let js = localstorage_script(&c, "tok");
        assert!(js.contains("new URL(\"http://h:8123\").origin"));
        assert!(js.contains("E-ink"));
        assert!(js.contains("hassTokens"));
    }
}
