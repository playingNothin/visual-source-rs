// VisualSource v4 — offline edition.
// All logic runs locally through a Rust->WebAssembly engine (visual-source-rs).
// No network calls anywhere: fonts fall back to system stacks, sounds are
// generated with the Web Audio API, and the engine ships inside this file.

// ---------------------------------------------------------------- engine ---

const WASM_B64 = '__WASM_B64__';
let _engine = null;
const engineReady = (async () => {
  const raw = Uint8Array.from(atob(WASM_B64), (c) => c.charCodeAt(0));
  const { instance } = await WebAssembly.instantiate(raw, {});
  _engine = instance.exports;
  const vptr = _engine.engine_version();
  const mem = new Uint8Array(_engine.memory.buffer);
  const end = mem.indexOf(0, vptr);
  const ver = new TextDecoder().decode(mem.slice(vptr, end));
  const el = document.getElementById('engine-status');
  if (el) el.textContent = 'local engine: v' + ver + ' · ready';
  return _engine;
})();

// wasm C ABI glue: every call packs (ptr << 32) | len; memory can grow at
// any allocation, so always take a fresh view of the buffer.
async function engineCall(fn, args) {
  const e = await engineReady;
  const ret = fn(...args);
  const ptr = Number(ret >> 32n);
  const len = Number(ret & 0xFFFFFFFFn);
  const mem = new Uint8Array(e.memory.buffer);
  const out = JSON.parse(new TextDecoder().decode(mem.slice(ptr, ptr + len)));
  if (ptr) e.engine_free(ptr, len);
  return out;
}

function putString(s) {
  const e = _engine;
  const b = new TextEncoder().encode(s);
  const p = e.engine_malloc(b.length);
  new Uint8Array(e.memory.buffer).set(b, p);
  return [p, b.length];
}

function dropString(p, l) {
  if (p) _engine.engine_free(p, l);
}

async function callWithStr(fn, s, extra = []) {
  const [p, l] = putString(s);
  try {
    return await engineCall(fn, [p, l, ...extra]);
  } finally {
    dropString(p, l);
  }
}

// ------------------------------------------------------------- ui helpers ---

function switchTab(name) {
  document.querySelectorAll('.tab').forEach((t) => {
    const onclick = t.getAttribute('onclick') || '';
    const m = onclick.match(/switchTab\('([^']+)'\)/);
    t.classList.toggle('active', m ? m[1] === name : false);
  });
  document.querySelectorAll('.panel').forEach((p) => {
    p.classList.toggle('active', p.id === 'panel-' + name);
  });
}

function setMessage(id, type, text, silent) {
  document.getElementById(id).innerHTML = '<div class="msg ' + type + '">' + escapeHtml(text) + '</div>';
  if (silent) return;
  if (type === 'success') _sfx.success();
  else if (type === 'error') _sfx.error();
}

function clearMessage(id) {
  document.getElementById(id).innerHTML = '';
}

function clearPanel(name) {
  const map = {
    encode: ['enc-input', 'enc-output', 'enc-message'],
    decode: ['dec-input', 'dec-output', 'dec-message'],
    luau: ['luau-input', 'luau-output', 'luau-message'],
    luautos: ['luautos-input', 'luautos-output', 'luautos-message', 'luautos-notes'],
    roundtrip: ['rt-input', 'rt-output', 'rt-message'],
    vspreview: ['pv-input', 'pv-message'],
  };
  (map[name] || []).forEach((id) => {
    const el = document.getElementById(id);
    if (el) {
      if (el.tagName === 'TEXTAREA' || el.tagName === 'INPUT') el.value = '';
      else el.innerHTML = '';
    }
  });
  if (name === 'vspreview') {
    const svg = document.querySelector('#pv-graph svg');
    if (svg) svg.remove();
    const lg = document.getElementById('pv-legend');
    if (lg) lg.innerHTML = '';
    _pvData = null;
  }
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
  }[c]));
}

async function copyText(areaId, msgId) {
  const v = document.getElementById(areaId).value;
  if (!v) return;
  try {
    await navigator.clipboard.writeText(v);
  } catch (e) {
    const ta = document.getElementById(areaId);
    ta.select();
    document.execCommand('copy');
  }
  setMessage(msgId, 'success', 'copied to clipboard', true);
}

function downloadText(areaId, filename) {
  const v = document.getElementById(areaId).value;
  if (!v) return;
  const blob = new Blob([v], { type: 'text/plain' });
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = filename;
  a.click();
  URL.revokeObjectURL(a.href);
}

// ------------------------------------------------------------- settings ---

const DARK_KEY = 'vs4_dark';
const MUTE_KEY = 'vs4_mute';
const FONT_KEY = 'vs4_font';

function isMuted() { return localStorage.getItem(MUTE_KEY) === '1'; }

function applyDark(on) {
  document.documentElement.classList.toggle('dark', on);
}

function toggleDark() {
  const on = document.getElementById('cfg-dark').checked;
  applyDark(on);
  localStorage.setItem(DARK_KEY, on ? '1' : '0');
}

function saveMute() {
  const muting = document.getElementById('cfg-mute').checked;
  localStorage.setItem(MUTE_KEY, muting ? '1' : '0');
  if (!muting) _sfx.tab();
}

// offline: no Google Fonts — the name is applied to the CSS vars only and
// falls back to the system monospace stack when not installed.
function applyFont() {
  const name = document.getElementById('cfg-font').value.trim();
  if (!name) return;
  document.documentElement.style.setProperty('--mono', "'" + name + "', Consolas, monospace");
  document.documentElement.style.setProperty('--sans', "'" + name + "', Consolas, monospace");
  localStorage.setItem(FONT_KEY, name);
}

function toggleSettings() {
  const panel = document.getElementById('settings-panel');
  panel.classList.toggle('visible');
  document.getElementById('settings-toggle').classList.toggle('open', panel.classList.contains('visible'));
}

function toggleOpts(btnId, panelId) {
  const panel = document.getElementById(panelId);
  const open = panel.classList.toggle('visible');
  document.getElementById(btnId).classList.toggle('open', open);
}

(function restorePrefs() {
  const font = localStorage.getItem(FONT_KEY);
  if (font) {
    document.getElementById('cfg-font').value = font;
    applyFont();
  }
  document.getElementById('cfg-mute').checked = isMuted();
  const dark = localStorage.getItem(DARK_KEY) === '1';
  document.getElementById('cfg-dark').checked = dark;
  applyDark(dark);
})();

// ----------------------------------------------------------------- sfx -----

var _sfx = (() => {
  let ctx = null;
  let muted = () => localStorage.getItem(MUTE_KEY) === '1';

  function getCtx() {
    if (!ctx) ctx = new (window.AudioContext || window.webkitAudioContext)();
    if (ctx.state === 'suspended') ctx.resume();
    return ctx;
  }

  let _reverb = null;
  function getReverb() {
    if (_reverb) return _reverb;
    const c = getCtx();
    const len = c.sampleRate * 0.6;
    const buf = c.createBuffer(2, len, c.sampleRate);
    for (let ch = 0; ch < 2; ch++) {
      const d = buf.getChannelData(ch);
      for (let i = 0; i < len; i++)
        d[i] = (Math.random() * 2 - 1) * Math.pow(1 - i / len, 3.5);
    }
    _reverb = c.createConvolver();
    _reverb.buffer = buf;
    const rv = c.createGain();
    rv.gain.value = 0.18;
    _reverb.connect(rv);
    rv.connect(c.destination);
    return _reverb;
  }

  let _comp = null;
  function getComp() {
    if (_comp) return _comp;
    const c = getCtx();
    _comp = c.createDynamicsCompressor();
    _comp.threshold.value = -18;
    _comp.knee.value = 12;
    _comp.ratio.value = 4;
    _comp.attack.value = 0.003;
    _comp.release.value = 0.15;
    _comp.connect(c.destination);
    return _comp;
  }

  function voice(freq, startOffset, dur, vol, opts = {}) {
    if (muted()) return;
    const c = getCtx();
    const t = c.currentTime + startOffset;
    const atk = opts.atk ?? 0.006;
    const rel = opts.rel ?? dur * 0.55;
    const comp = getComp();
    const rev = getReverb();

    function makeOsc(type, f, gainMul) {
      const o = c.createOscillator();
      o.type = type;
      o.frequency.setValueAtTime(f, t);
      if (opts.slideTo)
        o.frequency.exponentialRampToValueAtTime(Math.max(opts.slideTo, 10), t + dur * 0.85);

      const lp = c.createBiquadFilter();
      lp.type = 'lowpass';
      lp.frequency.setValueAtTime(opts.lp ?? 3200, t);
      if (opts.lpSlide)
        lp.frequency.exponentialRampToValueAtTime(opts.lpSlide, t + dur);
      lp.Q.value = opts.Q ?? 0.5;

      const g = c.createGain();
      g.gain.setValueAtTime(0, t);
      g.gain.linearRampToValueAtTime(vol * gainMul, t + atk);
      g.gain.setValueAtTime(vol * gainMul, t + dur - rel);
      g.gain.exponentialRampToValueAtTime(0.00001, t + dur);

      o.connect(lp);
      lp.connect(g);
      g.connect(comp);
      if (opts.wet !== false) g.connect(rev);
      o.start(t);
      o.stop(t + dur + 0.05);
    }

    makeOsc('sine', freq, 1.0);
    makeOsc('triangle', freq * 2, 0.12);
    if (opts.sub !== false) makeOsc('sine', freq * 0.5, 0.08);
  }

  function noiseBurst(startOffset, dur, vol, lpFreq) {
    if (muted()) return;
    const c = getCtx();
    const t = c.currentTime + startOffset;
    const len = Math.ceil(c.sampleRate * dur);
    const buf = c.createBuffer(1, len, c.sampleRate);
    const d = buf.getChannelData(0);
    for (let i = 0; i < len; i++) d[i] = Math.random() * 2 - 1;
    const src = c.createBufferSource();
    src.buffer = buf;
    const lp = c.createBiquadFilter();
    lp.type = 'bandpass';
    lp.frequency.value = lpFreq;
    lp.Q.value = 1.4;
    const g = c.createGain();
    g.gain.setValueAtTime(vol, t);
    g.gain.exponentialRampToValueAtTime(0.00001, t + dur);
    src.connect(lp);
    lp.connect(g);
    g.connect(getComp());
    src.start(t);
    src.stop(t + dur + 0.01);
  }

  return {
    click() {
      noiseBurst(0, 0.018, 0.22, 1800);
      voice(520, 0, 0.10, 0.09, { atk: 0.003, rel: 0.08, lp: 2200, wet: false, sub: false });
    },
    tab() {
      voice(880, 0, 0.18, 0.07, { atk: 0.004, rel: 0.14, lp: 5000, lpSlide: 1200 });
      voice(1175, 0.10, 0.20, 0.06, { atk: 0.004, rel: 0.16, lp: 5000, lpSlide: 1000 });
    },
    submit() {
      voice(330, 0, 0.15, 0.07, { atk: 0.005, rel: 0.11, lp: 1800, lpSlide: 3600 });
      voice(494, 0.11, 0.15, 0.07, { atk: 0.005, rel: 0.11, lp: 2200, lpSlide: 4200 });
      voice(659, 0.21, 0.22, 0.06, { atk: 0.005, rel: 0.17, lp: 2800, lpSlide: 5000 });
    },
    success() {
      voice(523, 0, 0.28, 0.08, { atk: 0.007, rel: 0.22, lp: 4000 });
      voice(659, 0.15, 0.28, 0.08, { atk: 0.007, rel: 0.22, lp: 4200 });
      voice(784, 0.28, 0.28, 0.07, { atk: 0.007, rel: 0.22, lp: 4400 });
      voice(1047, 0.40, 0.34, 0.07, { atk: 0.007, rel: 0.28, lp: 5000 });
    },
    error() {
      noiseBurst(0, 0.025, 0.18, 300);
      voice(220, 0, 0.30, 0.10, { atk: 0.004, rel: 0.24, slideTo: 100, lp: 800, lpSlide: 200, Q: 1.8 });
      voice(160, 0.22, 0.28, 0.07, { atk: 0.004, rel: 0.22, slideTo: 80, lp: 600, lpSlide: 150, Q: 2.0 });
    },
    copy() {
      noiseBurst(0, 0.010, 0.12, 4000);
      voice(1320, 0, 0.16, 0.07, { atk: 0.002, rel: 0.13, lp: 6000, sub: false });
      voice(1760, 0.09, 0.18, 0.05, { atk: 0.002, rel: 0.15, lp: 6500, sub: false });
    },
    type() {
      const bases = [900, 950, 860, 980, 920, 870, 960];
      const f = bases[Math.floor(Math.random() * bases.length)];
      noiseBurst(0, 0.014, 0.09, f);
      voice(f * 0.5, 0, 0.06, 0.025, { atk: 0.001, rel: 0.05, lp: 2200, wet: false, sub: false });
    },
  };
})();

// ------------------------------------------------------------ operations ---

async function doEncode() {
  const input = document.getElementById('enc-input').value;
  const btn = document.getElementById('enc-btn');
  clearMessage('enc-message');
  document.getElementById('enc-output').value = '';
  if (!input.trim()) { setMessage('enc-message', 'error', 'you need to paste something first'); return; }
  btn.disabled = true;
  setMessage('enc-message', 'loading', 're-encoding locally...', true);
  try {
    const r = await callWithStr(_engine.encode_vs, input);
    if (r.error) { setMessage('enc-message', 'error', r.error); return; }
    document.getElementById('enc-output').value = r.output;
    const s = r.stats;
    setMessage('enc-message', 'success',
      're-encoded losslessly — ' + s.input_chars + ' chars → ' + s.compressed_bytes + ' decompressed bytes → ' + s.output_chars + ' total (' + s.blocks + ' blocks)');
  } catch (e) {
    setMessage('enc-message', 'error', String(e));
  } finally {
    btn.disabled = false;
  }
}

async function doDecode() {
  const input = document.getElementById('dec-input').value;
  const btn = document.getElementById('dec-btn');
  clearMessage('dec-message');
  document.getElementById('dec-output').value = '';
  if (!input.trim()) { setMessage('dec-message', 'error', 'you need to paste something first'); return; }
  btn.disabled = true;
  setMessage('dec-message', 'loading', 'decoding locally...', true);
  try {
    const r = await callWithStr(_engine.decode_vs, input, ['<input>']);
    if (r.error) { setMessage('dec-message', 'error', r.error); return; }
    document.getElementById('dec-output').value = r.output;
    const s = r.stats;
    setMessage('dec-message', 'success', 'decoded — version ' + s.version + ', ' + s.blocks + ' blocks, ' + s.comments + ' comments (' + s.output_chars + ' chars)');
  } catch (e) {
    setMessage('dec-message', 'error', String(e));
  } finally {
    btn.disabled = false;
  }
}

async function doToLuau() {
  const input = document.getElementById('luau-input').value;
  const btn = document.getElementById('luau-btn');
  clearMessage('luau-message');
  document.getElementById('luau-output').value = '';
  if (!input.trim()) { setMessage('luau-message', 'error', 'you need to paste something first'); return; }
  btn.disabled = true;
  _sfx.submit();
  setMessage('luau-message', 'loading', 'converting locally...', true);
  try {
    const r = await callWithStr(_engine.toluau_vs, input);
    if (r.error) { setMessage('luau-message', 'error', r.error); return; }
    document.getElementById('luau-output').value = r.output;
    const s = r.stats;
    const gaps = s.gaps || 0;
    if (gaps > 0) {
      setMessage('luau-message', 'success',
        'converted with ' + gaps + ' unsupported block(s) — marked with -- [detranspile] comments');
    } else {
      setMessage('luau-message', 'success', 'converted — ' + s.blocks + ' blocks reconstructed');
    }
  } catch (e) {
    setMessage('luau-message', 'error', String(e));
  } finally {
    btn.disabled = false;
  }
}

async function doToVs() {
  const input = document.getElementById('luautos-input').value;
  const btn = document.getElementById('luautos-btn');
  clearMessage('luautos-message');
  document.getElementById('luautos-output').value = '';
  document.getElementById('luautos-notes').innerHTML = '';
  if (!input.trim()) { setMessage('luautos-message', 'error', 'you need to paste something first'); return; }
  btn.disabled = true;
  _sfx.submit();
  setMessage('luautos-message', 'loading', 'transpiling locally...', true);
  try {
    const r = await callWithStr(_engine.transpile_luau, input, ['<luau>']);
    if (r.error) { setMessage('luautos-message', 'error', r.error); return; }
    document.getElementById('luautos-output').value = r.output;
    const s = r.stats;
    setMessage('luautos-message', 'success', 'transpiled — ' + s.blocks + ' blocks → ' + s.encoded_chars + ' chars');
    const notes = r.notes || [];
    if (notes.length) {
      const kinds = {};
      notes.forEach((n) => { kinds[n.kind] = (kinds[n.kind] || 0) + 1; });
      const badge = Object.entries(kinds).map(([k, v]) => k + ':' + v).join(' · ');
      // actionable notes (UNSUPPORTED/VERIFY) list in full; INFO/WORKAROUND
      // describe internal lowering decisions - collapse into an expandable
      // section so a clean run stays clean
      const ACTIONABLE = ['UNSUPPORTED', 'VERIFY'];
      const front = notes.filter((n) => ACTIONABLE.includes(n.kind));
      const rest = notes.filter((n) => !ACTIONABLE.includes(n.kind));
      let html = '<div class="msg info">compiler notes (' + badge + '):<br>';
      front.slice(0, 30).forEach((n) => {
        html += escapeHtml('[' + n.kind + '] ' + n.msg) + '<br>';
      });
      if (front.length > 30) html += '… ' + (front.length - 30) + ' more';
      if (rest.length) {
        html += '<details><summary style="cursor:pointer">show ' + rest.length + ' info/workaround note(s)</summary>';
        rest.forEach((n) => {
          html += escapeHtml('[' + n.kind + '] ' + n.msg) + '<br>';
        });
        html += '</details>';
      }
      html += '</div>';
      document.getElementById('luautos-notes').innerHTML = html;
    }
  } catch (e) {
    setMessage('luautos-message', 'error', String(e));
  } finally {
    btn.disabled = false;
  }
}

async function doRoundtrip() {
  const input = document.getElementById('rt-input').value;
  const btn = document.getElementById('rt-btn');
  clearMessage('rt-message');
  document.getElementById('rt-output').value = '';
  if (!input.trim()) { setMessage('rt-message', 'error', 'you need to paste something first'); return; }
  btn.disabled = true;
  setMessage('rt-message', 'loading', 'verifying losslessness locally...', true);
  try {
    const r = await callWithStr(_engine.roundtrip_vs, input, ['<input>']);
    if (r.error) { setMessage('rt-message', 'error', r.error); return; }
    document.getElementById('rt-output').value = r.report;
    setMessage('rt-message', r.lossless ? 'success' : 'error',
      r.lossless ? 'LOSSLESS — this payload re-encodes bit-for-bit' : 'MISMATCH — see the layered report');
  } catch (e) {
    setMessage('rt-message', 'error', String(e));
  } finally {
    btn.disabled = false;
  }
}

// ------------------------------------------------------------ VS preview ---
// Draws the save's OWN VisualPosition grid, so what you see here is the
// same layout the RetroStudio editor shows in-game: chains flow top to
// bottom and every block is colored by its category (Actions purple,
// Input blue, Events red, Math yellow, Logic green, Misc blue).

let _pvData = null;
let _pvScale = 150; // px per grid unit (a block footprint is 1.0 x 0.5 units)

const PV_COLORS = {
  Actions:       { fill: '#9b30d9', text: '#ffffff' },
  Input:         { fill: '#2f7fe0', text: '#ffffff' },
  Events:        { fill: '#e03e3e', text: '#ffffff' },
  Math:          { fill: '#eec643', text: '#42370a' },
  Logic:         { fill: '#3fbf5f', text: '#ffffff' },
  Miscellaneous: { fill: '#35b5e5', text: '#ffffff' },
  Unverified:    { fill: '#8a94a6', text: '#ffffff' },
  Deprecated:    { fill: '#6e6e6e', text: '#ffffff' },
};
const PV_LEGEND = [
  ['Actions', 'Actions'], ['Input', 'Input'], ['Events', 'Events'],
  ['Math', 'Math'], ['Logic', 'Logic'], ['Miscellaneous', 'Misc'],
  ['Unverified', 'Unverified'], ['Deprecated', 'Deprecated'],
];

function pvColor(cat) { return PV_COLORS[cat] || PV_COLORS.Miscellaneous; }

// --- camera pan: drag the mouse to move the view (like the RetroStudio editor) ---
let _pvPanX = 0, _pvPanY = 0;
let _pvDragDist = 0;
let _pvPressBlock = null;

function pvApplyPan() {
  const svg = document.querySelector('#pv-graph svg');
  if (svg) svg.style.transform = 'translate(' + _pvPanX + 'px,' + _pvPanY + 'px)';
}

function pvResetPan() {
  _pvPanX = 0;
  _pvPanY = 0;
  pvApplyPan();
}

function initPreviewPan() {
  const wrap = document.getElementById('pv-graph');
  if (!wrap) return;
  let drag = null;
  wrap.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    // remember what was under the pointer: a still press on a block is a click,
    // a moving press is a camera drag. setPointerCapture below retargets the
    // DOM click to the wrapper, so block selection must come from the gesture.
    const hit = e.target.closest ? e.target.closest('[data-block-id]') : null;
    _pvPressBlock = hit ? hit.getAttribute('data-block-id') : null;
    drag = { x: e.clientX, y: e.clientY, px: _pvPanX, py: _pvPanY };
    _pvDragDist = 0;
    wrap.classList.add('panning');
    try { wrap.setPointerCapture(e.pointerId); } catch (_) {}
    e.preventDefault();
  });
  wrap.addEventListener('pointermove', (e) => {
    if (!drag) return;
    const dx = e.clientX - drag.x;
    const dy = e.clientY - drag.y;
    _pvDragDist = Math.max(_pvDragDist, Math.abs(dx) + Math.abs(dy));
    _pvPanX = drag.px + dx;
    _pvPanY = drag.py + dy;
    pvApplyPan();
  });
  const stop = () => {
    if (!drag) return;
    drag = null;
    wrap.classList.remove('panning');
    // still press on a block = open its details; anything further = a pan
    if (_pvDragDist <= 6 && _pvPressBlock) showNodeDetails(_pvPressBlock);
    _pvPressBlock = null;
  };
  wrap.addEventListener('pointerup', stop);
  wrap.addEventListener('pointercancel', stop);
}
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', initPreviewPan);
} else {
  initPreviewPan();
}

function pvZoom(mult) {
  _pvScale = Math.min(320, Math.max(50, _pvScale * mult));
  pvZoomLabel();
  if (_pvData) renderPreview(_pvData);
}

function pvZoomReset() {
  _pvScale = 150;
  pvZoomLabel();
  if (_pvData) renderPreview(_pvData);
}

function pvZoomLabel() {
  const el = document.getElementById('pv-zoom-label');
  if (el) el.textContent = Math.round((_pvScale / 150) * 100) + '%';
}

async function doPreview() {
  const input = document.getElementById('pv-input').value;
  const btn = document.getElementById('pv-btn');
  clearMessage('pv-message');
  if (!input.trim()) { setMessage('pv-message', 'error', 'you need to paste something first'); return; }
  btn.disabled = true;
  setMessage('pv-message', 'loading', 'building preview locally...', true);
  try {
    const r = await callWithStr(_engine.vs_to_json, input);
    if (r.error) { setMessage('pv-message', 'error', r.error); return; }
    _pvData = r;
    pvResetPan();
    renderPreview(r);
    const nCom = (r.comments || []).length;
    setMessage('pv-message', 'success',
      r.order.length + ' blocks rendered in the RetroStudio layout (' + nCom + ' comment boxes) — click a block for details');
  } catch (e) {
    setMessage('pv-message', 'error', String(e));
  } finally {
    btn.disabled = false;
  }
}

// true when every block carries its VisualPosition from the save — then the
// preview draws the exact editor grid. Saves without positions (or a fully
// stacked one) fall back to an auto top-down layout.
function pvHasGeometry(data) {
  const ids = data.order.filter((id) => data.blocks[id]);
  if (!ids.length) return false;
  if (!ids.every((id) => data.blocks[id].x != null && data.blocks[id].y != null)) return false;
  if (ids.length === 1) return true;
  const xs = ids.map((id) => data.blocks[id].x);
  const ys = ids.map((id) => data.blocks[id].y);
  const spread = (a) => Math.max(...a) - Math.min(...a);
  return spread(xs) > 0.25 || spread(ys) > 0.25;
}

function pvLegend(data) {
  const el = document.getElementById('pv-legend');
  if (!el) return;
  const present = new Set();
  data.order.forEach((id) => {
    const b = data.blocks[id];
    if (b) present.add(b.category || 'Miscellaneous');
  });
  el.innerHTML = '';
  PV_LEGEND.forEach(([cat, name]) => {
    const always = ['Unverified', 'Deprecated'].indexOf(cat) < 0;
    if (!always && !present.has(cat)) return;
    const chip = document.createElement('span');
    chip.className = 'pv-chip';
    const sw = document.createElement('span');
    sw.className = 'sw';
    sw.style.background = pvColor(cat).fill;
    chip.appendChild(sw);
    chip.appendChild(document.createTextNode(name));
    el.appendChild(chip);
  });
}

function pvInputLine(v) {
  if (!v) return '';
  if (v.var !== undefined) return '$' + v.var;
  let s = pvCleanWireList(String(v.value));
  if (s.length > 26) s = s.slice(0, 25) + '…';
  return s;
}

// Values can embed comma-lists of encoded tuple wire names
// ("TUPLEPARAM_Parameters_1,...") — show them as Parameters[1], ... instead.
function pvCleanWireList(s) {
  return String(s).split(',').map((item) => {
    const t = pvSplitTuple(item.trim());
    if (!t) return item;
    return t.elem ? t.tuple + '[' + t.idx + ']' : t.tuple + ' (tuple)';
  }).join(',');
}

function renderPreview(data) {
  pvLegend(data);
  pvZoomLabel();

  const svgNS = 'http://www.w3.org/2000/svg';
  const S = _pvScale;
  const BW = 1.0 * S; // RetroStudio block footprint: 1.0 wide, 0.5 tall
  const BH = 0.5 * S;
  const dark = document.documentElement.classList.contains('dark');
  const wireCol = dark ? '#5a84b0' : '#7e9dbb';
  const comFill = dark ? '#39424f' : '#ccd1d6';
  const comText = dark ? '#c9d4e4' : '#39404a';

  const ids = data.order.filter((id) => data.blocks[id]);
  const useGeom = pvHasGeometry(data);
  const pos = {}; // id -> top-left corner in px

  let minX = 0, minY = 0, maxX = BW, maxY = BH;
  const grow = (x, y) => {
    minX = Math.min(minX, x); minY = Math.min(minY, y);
    maxX = Math.max(maxX, x); maxY = Math.max(maxY, y);
  };

  if (useGeom) {
    ids.forEach((id) => {
      const b = data.blocks[id];
      pos[id] = { x: b.x * S, y: b.y * S };
      grow(pos[id].x, pos[id].y);
    });
    (data.comments || []).forEach((c) => { grow(c.x * S, c.y * S); grow((c.x + c.w) * S, (c.y + c.h) * S); });
    ids.forEach((id) => { grow(pos[id].x + BW, pos[id].y + BH); });
  } else {
    // fallback: auto TOP-DOWN layout — same reading direction as RetroStudio
    const g = new dagre.graphlib.Graph();
    g.setGraph({ rankdir: 'TB', nodesep: 18, ranksep: 34, marginx: 20, marginy: 20 });
    g.setDefaultEdgeLabel(() => ({}));
    ids.forEach((id) => g.setNode(id, { width: BW, height: BH }));
    ids.forEach((id) => {
      const b = data.blocks[id];
      (b.children || []).forEach((c) => { if (data.blocks[c]) g.setEdge(id, c); });
      if (b.else && data.blocks[b.else]) g.setEdge(id, b.else);
    });
    dagre.layout(g);
    ids.forEach((id) => {
      const n = g.node(id);
      pos[id] = { x: n.x - BW / 2, y: n.y - BH / 2 };
      grow(pos[id].x, pos[id].y);
      grow(pos[id].x + BW, pos[id].y + BH);
    });
  }

  const PAD = 24;
  minX -= PAD; minY -= PAD; maxX += PAD; maxY += PAD;
  const W = Math.max(maxX - minX, 200), H = Math.max(maxY - minY, 100);

  const svg = document.createElementNS(svgNS, 'svg');
  svg.setAttribute('width', W);
  svg.setAttribute('height', H);
  svg.setAttribute('viewBox', minX + ' ' + minY + ' ' + W + ' ' + H);

  // --- comment boxes behind everything (the editor's gray notes) ---
  if (useGeom) {
    (data.comments || []).forEach((c) => {
      const x = c.x * S, y = c.y * S;
      const w = Math.max(c.w * S, 20), h = Math.max(c.h * S, 14);
      const grp = document.createElementNS(svgNS, 'g');
      const rect = document.createElementNS(svgNS, 'rect');
      rect.setAttribute('x', x); rect.setAttribute('y', y);
      rect.setAttribute('width', w); rect.setAttribute('height', h);
      rect.setAttribute('rx', Math.min(10, h * 0.2));
      rect.setAttribute('fill', comFill);
      rect.setAttribute('fill-opacity', '0.92');
      rect.setAttribute('stroke', dark ? '#4b5563' : '#a9b0b8');
      grp.appendChild(rect);
      // RetroStudio wraps text inside the box: wrap by width, clip by height
      const charsPerLine = Math.max(8, Math.floor((w - 14) / 6.4));
      const words = String(c.text || '').split(/\s+/).filter(Boolean);
      const lines = [];
      let cur = '';
      words.forEach((wd) => {
        const t = cur ? cur + ' ' + wd : wd;
        if (t.length > charsPerLine && cur) { lines.push(cur); cur = wd; } else cur = t;
      });
      if (cur) lines.push(cur);
      const maxLines = Math.max(1, Math.floor((h - 12) / 13));
      const shown = lines.slice(0, maxLines);
      if (lines.length > maxLines && shown.length) {
        shown[shown.length - 1] = shown[shown.length - 1].slice(0, Math.max(1, charsPerLine - 1)) + '…';
      }
      shown.forEach((ln, i) => {
        const t = document.createElementNS(svgNS, 'text');
        t.setAttribute('x', x + 8); t.setAttribute('y', y + 16 + i * 13);
        t.setAttribute('fill', comText);
        t.setAttribute('font-size', '11');
        t.setAttribute('font-family', 'Consolas, monospace');
        t.textContent = ln;
        grp.appendChild(t);
      });
      svg.appendChild(grp);
    });
  }

  // --- wires: parent bottom -> child top, like the editor's flow ---
  ids.forEach((id) => {
    const b = data.blocks[id];
    const p = pos[id];
    if (!p) return;
    const px = p.x + BW / 2, pbot = p.y + BH;
    const draw = (childId, isElse) => {
      const q = pos[childId];
      if (!q) return;
      const cx = q.x + BW / 2;
      const my = (pbot + q.y) / 2;
      const path = document.createElementNS(svgNS, 'path');
      const d = Math.abs(cx - px) < 1
        ? 'M' + px + ' ' + pbot + ' L' + cx + ' ' + q.y
        : 'M' + px + ' ' + pbot + ' L' + px + ' ' + my + ' L' + cx + ' ' + my + ' L' + cx + ' ' + q.y;
      path.setAttribute('d', d);
      path.setAttribute('fill', 'none');
      path.setAttribute('stroke', isElse ? '#c0392b' : wireCol);
      path.setAttribute('stroke-width', '2');
      if (isElse) path.setAttribute('stroke-dasharray', '6 4');
      svg.appendChild(path);
    };
    (b.children || []).forEach((c) => draw(c, false));
    if (b.else) draw(b.else, true);
  });

  // --- blocks: rounded category-colored card, title + first inputs ---
  ids.forEach((id) => {
    const b = data.blocks[id];
    const p = pos[id];
    if (!p) return;
    const col = pvColor(b.category);
    const grp = document.createElementNS(svgNS, 'g');
    grp.setAttribute('class', 'pv-node');
    grp.style.cursor = 'pointer';

    const rect = document.createElementNS(svgNS, 'rect');
    rect.setAttribute('x', p.x); rect.setAttribute('y', p.y);
    rect.setAttribute('width', BW); rect.setAttribute('height', BH);
    rect.setAttribute('rx', Math.min(12, BH * 0.2));
    rect.setAttribute('fill', col.fill);
    rect.setAttribute('stroke', 'rgba(0,0,0,0.4)');
    rect.setAttribute('stroke-width', '1.2');
    grp.appendChild(rect);

    const title = document.createElementNS(svgNS, 'text');
    title.setAttribute('x', p.x + 10);
    title.setAttribute('y', p.y + 19);
    title.setAttribute('fill', col.text);
    title.setAttribute('font-size', '12.5');
    title.setAttribute('font-weight', 'bold');
    title.setAttribute('font-family', 'Consolas, monospace');
    const maxTitle = Math.max(6, Math.floor((BW - 20) / 7));
    let tStr = String(b.type || id);
    if (tStr.length > maxTitle) tStr = tStr.slice(0, maxTitle - 1) + '…';
    title.textContent = tStr;
    grp.appendChild(title);

    if (BH >= 46) {
      const inputs = Object.entries(b.inputs || {});
      const maxLines = Math.max(0, Math.floor((BH - 34) / 11.5));
      const shown = inputs.slice(0, maxLines);
      shown.forEach(([name, v], i) => {
        const maxLen = Math.max(6, Math.floor((BW - 20) / 6));
        let ln = pvPrettyProp(name) + ' = ' + pvInputLine(v);
        if (ln.length > maxLen) ln = ln.slice(0, maxLen - 1) + '…';
        const t = document.createElementNS(svgNS, 'text');
        t.setAttribute('x', p.x + 10);
        t.setAttribute('y', p.y + 32 + i * 11.5);
        t.setAttribute('fill', col.text);
        t.setAttribute('opacity', '0.85');
        t.setAttribute('font-size', '9.5');
        t.setAttribute('font-family', 'Consolas, monospace');
        t.textContent = ln;
        grp.appendChild(t);
      });
      if (inputs.length > maxLines) {
        const t = document.createElementNS(svgNS, 'text');
        t.setAttribute('x', p.x + 10);
        t.setAttribute('y', p.y + 32 + maxLines * 11.5);
        t.setAttribute('fill', col.text);
        t.setAttribute('opacity', '0.7');
        t.setAttribute('font-size', '9');
        t.setAttribute('font-family', 'Consolas, monospace');
        t.textContent = '+' + (inputs.length - maxLines) + ' more';
        grp.appendChild(t);
      }
    }

    grp.setAttribute('data-block-id', id);
    svg.appendChild(grp);
  });

  const wrap = document.getElementById('pv-graph');
  wrap.innerHTML = '';
  wrap.appendChild(svg);
  svg.style.transformOrigin = '0 0';
  pvApplyPan();
}

// --- tuple-aware property display ------------------------------------------
// Encoded wire names leak format internals: a tuple arrives as a `TUPLE_X`
// container (its value being the parameter count, hex) plus `TUPLEPARAM_X_N`
// wires for each element. These helpers translate them for debugging.

function pvSplitTuple(name) {
  const s = String(name);
  if (s.indexOf('TUPLEPARAM_') === 0) {
    const cut = s.lastIndexOf('_');
    if (cut > 11 && /^\d+$/.test(s.slice(cut + 1))) {
      return { tuple: s.slice(11, cut), idx: parseInt(s.slice(cut + 1), 10), elem: true };
    }
  } else if (s.indexOf('TUPLE_') === 0 && s.length > 6) {
    return { tuple: s.slice(6), elem: false };
  }
  return null;
}

function pvPrettyProp(name) {
  const t = pvSplitTuple(name);
  if (t && t.elem) return t.tuple + '[' + t.idx + ']';
  if (t && !t.elem) return t.tuple + ' (tuple)';
  return name;
}

// Merges tuple containers + elements into one readable row per tuple; plain
// entries pass through untouched. `shown(name, v)` renders the value column.
function pvGroupEntries(entries, shown) {
  const items = [];
  const byTuple = {};
  entries.forEach(([name, v]) => {
    const t = pvSplitTuple(name);
    if (!t) { items.push({ kind: 'plain', name, text: shown(name, v) }); return; }
    let g = byTuple[t.tuple];
    if (!g) {
      g = { kind: 'tuple', tuple: t.tuple, arity: -1, arityRaw: '', elems: [] };
      byTuple[t.tuple] = g;
      items.push(g);
    }
    if (t.elem) g.elems.push({ idx: t.idx, text: shown(name, v) });
    else {
      const n = parseInt(String(v), 16);
      if (!isNaN(n)) g.arity = n; else g.arityRaw = String(v);
    }
  });
  return items.map((it) => {
    if (it.kind === 'plain') return { label: it.name, text: it.text };
    it.elems.sort((a, b) => a.idx - b.idx);
    const n = it.arity >= 0 ? it.arity : (it.arityRaw !== '' ? it.arityRaw : it.elems.length);
    let parts;
    if (typeof n === 'number') {
      parts = [];
      for (let i = 1; i <= n; i++) {
        const e = it.elems.find((x) => x.idx === i);
        parts.push(e ? e.text : '?');
      }
    } else parts = it.elems.map((e) => e.text);
    return {
      label: it.tuple + ' (tuple' + (n !== '' ? ' of ' + n : '') + ')',
      text: '(' + (parts.join(', ') || 'no elements') + ')',
    };
  });
}

// One-line translation of structural blocks, so a debugger can read intent
// without decoding wire names.
function pvBlockSummary(b) {
  if (b.type !== 'DefineFunction') return '';
  let fn = '';
  const params = [];
  Object.entries(b.outputs || {}).forEach(([n, v]) => {
    if (n === 'Function') fn = String(v);
    const t = pvSplitTuple(n);
    if (t && t.elem && t.tuple === 'Parameters') params.push({ idx: t.idx, name: String(v) });
  });
  params.sort((a, b2) => a.idx - b2.idx);
  return 'defines: function ' + (fn || '?') + '(' + params.map((p) => p.name).join(', ') + ')';
}

function showNodeDetails(id) {
  const b = _pvData && _pvData.blocks[id];
  if (!b) return;
  const cat = b.category ? ' <span style="opacity:.7">(' + escapeHtml(b.category) + ')</span>' : '';
  let html = '<div class="msg info"><b>' + escapeHtml(id) + '</b> — ' + escapeHtml(b.type) + cat + '<br>';
  const sum = pvBlockSummary(b);
  if (sum) html += '<span style="opacity:.85">' + escapeHtml(sum) + '</span><br>';
  const inRows = pvGroupEntries(Object.entries(b.inputs || {}), (name, v) => {
    if (v && v.var !== undefined) return '$' + v.var;
    return JSON.stringify(pvCleanWireList(String(v && v.value))) + (v && v.type ? ' <' + v.type + '>' : '');
  });
  if (inRows.length) {
    html += 'inputs:<br>';
    inRows.forEach((r) => {
      html += '&nbsp;&nbsp;' + escapeHtml(r.label) + ' = ' + escapeHtml(r.text) + '<br>';
    });
  }
  const outRows = pvGroupEntries(Object.entries(b.outputs || {}), (name, v) => '$' + String(v));
  if (outRows.length) {
    html += 'outputs:<br>';
    outRows.forEach((r) => {
      html += '&nbsp;&nbsp;' + escapeHtml(r.label) + ' → ' + escapeHtml(r.text) + '<br>';
    });
  }
  html += '</div>';
  document.getElementById('pv-details').innerHTML = html;
}

// --------------------------------------------------------- service worker ---
// Only on http(s) (GitHub Pages): cache the page so repeat visits work with
// no network. On file:// this is skipped — the file already is self-contained.
if (location.protocol.startsWith('http') && 'serviceWorker' in navigator) {
  navigator.serviceWorker.register('sw.js').catch(() => {});
}
