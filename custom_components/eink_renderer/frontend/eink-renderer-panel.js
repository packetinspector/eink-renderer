// E-ink Renderer sidebar panel (registered by the eink_renderer integration).
// Everything goes through Home Assistant: the image comes from the integration's authenticated endpoint,
// status from the integration's entities. The browser never talks to the renderer directly.
const PANEL_VERSION = "0.3.1";
const CROP = 56;

class EinkRendererPanel extends HTMLElement {
  static get version() { return PANEL_VERSION; }

  set hass(h) {
    this._hass = h;
    if (this._built) this._update();
  }
  set panel(p) {
    this._cfg = (p && p.config) || { entries: [] };
    if (this._built) { this._built = false; this.innerHTML = ""; }
    if (this.isConnected) this._build();
  }
  set narrow(n) {}
  connectedCallback() { if (this._cfg && !this._built) this._build(); }
  disconnectedCallback() { this._built = false; this.innerHTML = ""; this._revoke(); }

  _entry() {
    const list = this._cfg.entries || [];
    return list.find((e) => e.entry_id === this._sel) || list[0];
  }
  _q(s) { return this.querySelector(s); }
  _revoke() { if (this._blobUrl) { URL.revokeObjectURL(this._blobUrl); this._blobUrl = null; } }

  _build() {
    this._built = true;
    this._inverted = false;
    this._imgKey = null;
    const entries = this._cfg.entries || [];
    this.innerHTML = `
<style>
  eink-renderer-panel { display:block; }
  .wrap { padding:12px; font-family: var(--ha-font-family-body, Roboto, sans-serif); color: var(--primary-text-color); background: var(--primary-background-color); min-height:100vh; box-sizing:border-box; }
  .bar { display:flex; gap:12px; align-items:center; flex-wrap:wrap; margin-bottom:8px; }
  .bar h1 { font-size:20px; margin:0 12px 0 0; font-weight:500; }
  .bar a, .bar button, .bar select { font:inherit; color: var(--primary-color); background:none; border:1px solid var(--divider-color); border-radius:6px; padding:4px 10px; cursor:pointer; text-decoration:none; }
  .alert { display:none; margin:0 0 12px; padding:8px 12px; border-radius:6px; border:1px solid var(--error-color, #c00); color: var(--error-color, #c00); background: rgba(219,68,55,.08); font-size:14px; }
  .alert.on { display:block; }
  .grid { display:flex; gap:12px; flex-wrap:wrap; align-items:flex-start; }
  figure { margin:0; }
  figcaption { font-size:13px; color: var(--secondary-text-color); margin:4px 0; }
  .frame { width:var(--w); height:var(--h); overflow:hidden; position:relative; border:1px solid var(--divider-color); background:#fff; }
  .frame iframe { position:absolute; top:-${CROP}px; left:0; width:var(--w); height:calc(var(--h) + ${CROP}px); border:0; }
  .imgbox { position:relative; width:var(--w); height:var(--h); border:1px solid var(--divider-color); background:#ddd; }
  .imgbox img { width:100%; height:100%; image-rendering:pixelated; image-rendering:crisp-edges; display:block; }
  .imgbox img:not([src]) { visibility:hidden; }
  .imgbox .empty { position:absolute; inset:0; display:flex; align-items:center; justify-content:center; text-align:center; padding:24px; color:#555; }
  .imgbox.has-img .empty { display:none; }
  .imgbox.stale img { opacity:.35; }
  .scale .frame, .scale .imgbox { zoom: var(--zoom, 1); }
  .status { font-size:12px; color: var(--secondary-text-color); margin:8px 0 0; }
  details { margin-top:16px; max-width:820px; }
  summary { cursor:pointer; font-weight:500; }
  pre { background: var(--secondary-background-color, #f3f3f3); padding:12px; border-radius:6px; overflow:auto; font-size:13px; }
</style>
<div class="wrap">
  <div class="bar">
    <h1>E-ink display</h1>
    ${entries.length > 1 ? `<select id="sel">${entries.map((e) => `<option value="${e.entry_id}">${e.title}</option>`).join("")}</select>` : ""}
    <a id="edit" target="_top">Edit this view</a>
    <a id="open" target="_top">Open dashboard</a>
    <a id="device" target="_top">Settings</a>
    <button id="pol" title="Show the exact pixels sent to the display">Show as sent (inverted)</button>
    <button id="reload">Reload live view</button>
    <label><input type="checkbox" id="fit"> fit to width</label>
  </div>
  <div class="alert" id="alert" role="alert"></div>
  <div class="grid" id="grid">
    <figure><figcaption id="livecap">Live view</figcaption><div class="frame"><iframe id="live" title="Live dashboard view"></iframe></div></figure>
    <figure><figcaption id="cap">What the display receives</figcaption>
      <div class="imgbox" id="imgbox"><img id="img" alt="e-ink image"><div class="empty" id="empty">Waiting for the first image…</div></div></figure>
  </div>
  <div class="status" id="st"></div>
  <details id="esp"><summary>ESPHome configuration for the display</summary>
    <p>Add this to your display's ESPHome YAML. The display fetches the image straight from the renderer.</p>
    <pre id="snippet"></pre><button id="copy">Copy</button></details>
</div>`;
    if (!entries.length) {
      this._alert("No E-ink Renderer is set up. Add one under Settings → Devices & services.");
      return;
    }
    const sel = this._q("#sel");
    if (sel) sel.onchange = () => { this._sel = sel.value; this._imgKey = null; this._applyEntry(); };
    this._q("#pol").onclick = () => {
      this._inverted = !this._inverted;
      this._q("#pol").textContent = this._inverted ? "Show as viewed (normal)" : "Show as sent (inverted)";
      this._imgKey = null; this._update();
    };
    this._q("#reload").onclick = () => { const f = this._q("#live"); f.src = f.src; };
    this._q("#fit").onchange = (e) => this._fit(e.target.checked);
    this._q("#copy").onclick = async () => {
      try { await navigator.clipboard.writeText(this._q("#snippet").textContent); this._q("#copy").textContent = "Copied"; }
      catch (e) { this._q("#copy").textContent = "Select the text and copy it"; }
    };
    this._applyEntry();
  }

  _applyEntry() {
    const e = this._entry();
    if (!e) return;
    const g = this._q("#grid");
    g.style.setProperty("--w", (e.width || 800) + "px");
    g.style.setProperty("--h", (e.height || 480) + "px");
    const live = this._q("#live");
    if (live.dataset.path !== e.dashboard_path) {
      live.dataset.path = e.dashboard_path;
      live.src = e.dashboard_path;
    }
    this._q("#edit").href = e.dashboard_path + "?edit=1";
    this._q("#open").href = e.dashboard_path;
    this._q("#device").href = `/config/integrations/integration/eink_renderer`;
    this._q("#livecap").textContent = `Live view (${e.dashboard_path}), ${e.width}×${e.height}`;
    this._q("#snippet").textContent = e.esphome || "";
    this._update();
  }

  _fit(on) {
    const g = this._q("#grid");
    if (!on) { g.classList.remove("scale"); return; }
    const e = this._entry() || {};
    g.style.setProperty("--zoom", Math.min(1, (this.clientWidth - 48) / (e.width || 800)).toFixed(3));
    g.classList.add("scale");
  }

  _alert(t) { const a = this._q("#alert"); if (!a) return; a.textContent = t || ""; a.classList.toggle("on", !!t); }

  async _update() {
    const e = this._entry(), h = this._hass;
    if (!e || !h || !this._q("#imgbox")) return;
    const st = (id) => (id && h.states[id]) || null;
    const problem = st(e.problem_entity), image = st(e.image_entity), state = st(e.state_entity);
    const last = st(e.last_render_entity), memory = st(e.memory_entity);
    const box = this._q("#imgbox");
    if (problem && problem.state === "on") {
      this._alert(`Problem: ${problem.attributes.message || "the renderer reports a problem"}`);
      box.classList.add("stale");
    } else {
      this._alert("");
      box.classList.remove("stale");
    }
    const key = `${image ? image.state : ""}|${this._inverted}`;
    if (image && image.state !== "unavailable" && key !== this._imgKey) {
      this._imgKey = key;
      try {
        const r = await h.fetchWithAuth(`/api/eink_renderer/${e.entry_id}/image?variant=${this._inverted ? "eink" : "preview"}&t=${Date.now()}`);
        if (!r.ok) throw new Error("HTTP " + r.status);
        const blob = await r.blob();
        if (!this._q("#img")) return;
        this._revoke();
        this._blobUrl = URL.createObjectURL(blob);
        this._q("#img").src = this._blobUrl;
        box.classList.add("has-img");
      } catch (err) {
        this._imgKey = null;
        if (!box.classList.contains("has-img")) this._q("#empty").textContent = "The image could not be loaded from the renderer.";
      }
    }
    const fmt = (s) => (s && s.state && !["unknown", "unavailable"].includes(s.state) ? s.state : "–");
    const when = last && last.state && !["unknown", "unavailable"].includes(last.state) ? new Date(last.state).toLocaleTimeString() : "–";
    this._q("#cap").textContent = `What the display receives (${this._inverted ? "inverted, exact bytes" : "normal polarity"}), last change ${when}`;
    this._q("#st").textContent = `Renderer: ${state ? (h.formatEntityState ? h.formatEntityState(state) : state.state) : "–"} · memory ${fmt(memory)} MB · panel ${PANEL_VERSION}`;
  }
}

if (!customElements.get("eink-renderer-panel")) {
  customElements.define("eink-renderer-panel", EinkRendererPanel);
}
