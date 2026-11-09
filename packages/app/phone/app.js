// Potato Bot phone app. Plain ES modules, no build step.
// Server text is only ever put into the page with textContent (never as markup).
import { t, setLang, pickLang, getLang, fmtMoney, fmtTime } from './i18n.js';
import {
  STORE_KEY, ApiError, NetError, helloKey, raceHello, b64ToBytes, normalizeOrigin, dedupeOrigins
} from './net.js';

const MAX_PHOTOS = 3;
const KEEP_MS = 5 * 60 * 1000; // how often a Control session is kept alive
const AWAY_MS = 60 * 1000;     // hidden this long while in Control: give the control back
const MAX_EDGE = 1600;
const JOB_DONE = new Set(['done', 'stopped', 'failed', 'error', 'cancelled', 'canceled', 'ended', 'finished']);
const enc = encodeURIComponent;

/** Parse "#/pair/CODE?a=<origin>&a=..." (kept in the fragment so it never reaches a server log). */
export function parsePairFragment(hash) {
  const m = /^#\/pair\/([^?]+)(?:\?(.*))?$/.exec(String(hash || ''));
  if (!m) return null;
  let code = '';
  try { code = decodeURIComponent(m[1]).trim().toUpperCase(); } catch { return null; }
  if (!code) return null;
  const params = new URLSearchParams(m[2] || '');
  return { code, candidates: dedupeOrigins(params.getAll('a')) };
}

function parseRoute(hash) {
  const sp = /^#\/space\/([^/?]+)(?:\/([^/?]+))?/.exec(String(hash || ''));
  if (sp) {
    try {
      const sid = decodeURIComponent(sp[1]);
      return sp[2] ? { name: 'channel', sid, cid: decodeURIComponent(sp[2]) } : { name: 'space', id: sid };
    } catch { return { name: 'list' }; }
  }
  if (/^#\/spaces\/?$/.test(String(hash || ''))) return { name: 'spaces' };
  const c = /^#\/computer\/([^/?]+)/.exec(String(hash || ''));
  if (c) {
    try { return { name: 'computer', id: decodeURIComponent(c[1]) }; } catch { return { name: 'list' }; }
  }
  const m = /^#\/(bot|room)\/([^/?]+)/.exec(String(hash || ''));
  if (!m) return { name: 'list' };
  try { return { name: m[1], id: decodeURIComponent(m[2]) }; } catch { return { name: 'list' }; }
}
function routeHash(r) {
  if (r.name === 'list') return '#/';
  if (r.name === 'spaces') return '#/spaces';
  if (r.name === 'channel') return '#/space/' + enc(r.sid) + '/' + enc(r.cid);
  if (r.name === 'space') return '#/space/' + enc(r.id);
  return '#/' + r.name + '/' + enc(r.id);
}
const SPACE_ROUTES = new Set(['spaces', 'space', 'channel']);
const sameRoute = (a, b) => a.name === b.name && a.id === b.id && a.sid === b.sid && a.cid === b.cid;

export function mountPhoneApp(root, deps) {
  const doc = root.ownerDocument;
  const win = doc.defaultView;
  const nav = deps.navigator || {};
  const loc = deps.location || win.location;
  const now = deps.now || (() => Date.now());
  const setT = deps.setTimeout || ((f, ms) => globalThis.setTimeout(f, ms));
  const clearT = deps.clearTimeout || ((id) => globalThis.clearTimeout(id));
  const netDeps = { fetch: deps.fetch, crypto: deps.crypto, setTimeout: setT, clearTimeout: clearT };
  const storage = deps.storage;

  setLang(pickLang((nav.languages && nav.languages[0]) || nav.language));
  doc.documentElement.lang = getLang() === 'zh' ? 'zh-Hant' : 'en';

  const S = {
    store: null, base: '', me: null, route: { name: 'list' }, view: null, screen: '',
    rev: 0, seq: 0, racing: null, loop: null, loopGen: 0, visible: doc.visibilityState !== 'hidden',
    hiddenAt: 0, presenceTimer: null, pending: null, destroyed: false, toastTimer: null, listScroll: 0,
    conn: 'online', botsById: {}, botsKnown: false, spacesOn: false, spaces: null
  };

  // ---------- tiny DOM helper ----------
  function h(tag, props, ...kids) {
    const el = doc.createElement(tag);
    for (const [k, v] of Object.entries(props || {})) {
      if (v === false || v == null) continue;
      if (k === 'class') el.className = v;
      else if (k === 'text') el.textContent = v;
      else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
      else el.setAttribute(k, v === true ? '' : String(v));
    }
    for (const kid of kids.flat()) {
      if (kid == null || kid === false) continue;
      el.append(typeof kid === 'string' ? doc.createTextNode(kid) : kid);
    }
    return el;
  }

  const host = h('main', { class: 'screen-host' });
  const overlay = h('div', { class: 'overlay' });
  const toastEl = h('div', { class: 'toast', role: 'status', 'aria-live': 'polite', hidden: true });
  const shell = h('div', { class: 'app' }, host, overlay, toastEl);
  root.replaceChildren(shell);

  // ---------- keyboard / visual viewport ----------
  // Android Chrome resizes the page itself (interactive-widget=resizes-content). iOS Safari does not:
  // it shrinks only the visual viewport. So we size the shell to the visual viewport and publish the keyboard height.
  const vv = deps.visualViewport || win.visualViewport || null;
  const rootStyle = doc.documentElement.style;
  function applyViewport() {
    const ih = Number(win.innerHeight) || 0;
    let kb = 0;
    if (vv && Number(vv.height) > 0) {
      const top = Number(vv.offsetTop) || 0;
      kb = Math.max(0, Math.round(ih - vv.height - top));
      rootStyle.setProperty('--vvh', Math.round(vv.height) + 'px');
      rootStyle.setProperty('--vvtop', Math.round(top) + 'px');
    }
    rootStyle.setProperty('--kb', kb + 'px');
    // Android shrinks the whole page, so --kb stays 0 there: also compare with the tallest height seen at this width.
    const w = Number(win.innerWidth) || 0;
    const cur = vv && Number(vv.height) > 0 ? vv.height + (Number(vv.offsetTop) || 0) : ih;
    if (w !== S.vpW) { S.vpW = w; S.vpMax = cur; }
    S.vpMax = Math.max(S.vpMax || 0, cur);
    shell.classList.toggle('kb-open', kb > 120 || S.vpMax - cur > 120);
    if (S.view && typeof S.view.onViewport === 'function') S.view.onViewport(kb);
  }
  function onViewportEvent() {
    applyViewport();
    // iOS may scroll the layout viewport when the keyboard opens; the shell is pinned, so put it back.
    try { if (vv && (Number(vv.offsetTop) || 0) > 0 && win.scrollY) win.scrollTo(0, 0); } catch { /* ignore */ }
    if (typeof win.requestAnimationFrame === 'function') win.requestAnimationFrame(() => { if (S.view && S.view.onViewport) S.view.onViewport(); });
  }

  function setScreen(name, el) {
    S.screen = name;
    host.replaceChildren(el);
    paintConn();
  }
  // ---------- connection indicator ----------
  function connState() { return nav.onLine === false ? 'offline' : S.conn; }
  function paintConn() {
    const st = connState();
    for (const el of shell.querySelectorAll('.conn')) {
      el.setAttribute('data-state', st);
      el.setAttribute('aria-label', t('conn.' + st));
      el.setAttribute('title', t('conn.' + st));
    }
    for (const nb of shell.querySelectorAll('.netbar')) {
      nb.hidden = st === 'online';
      nb.textContent = st === 'online' ? '' : t('conn.banner_' + st);
    }
  }
  function setConn(st) {
    if (S.conn === st) return;
    S.conn = st;
    paintConn();
  }
  function connEl() {
    return h('span', { class: 'conn', role: 'img', 'data-state': connState(), 'aria-label': t('conn.' + connState()) });
  }
  function toast(msg) {
    toastEl.textContent = msg;
    toastEl.hidden = false;
    if (S.toastTimer) clearT(S.toastTimer);
    S.toastTimer = setT(() => { toastEl.hidden = true; }, 4000);
  }

  // ---------- storage (never throws) ----------
  function loadStore() {
    try {
      const raw = storage.getItem(STORE_KEY);
      if (!raw) return null;
      const o = JSON.parse(raw);
      if (o && typeof o.token === 'string' && o.token) {
        return {
          token: o.token,
          deviceId: String(o.deviceId || ''),
          candidates: Array.isArray(o.candidates) ? dedupeOrigins(o.candidates) : []
        };
      }
    } catch { /* ignore */ }
    return null;
  }
  function saveStore() { try { storage.setItem(STORE_KEY, JSON.stringify(S.store)); } catch { /* ignore */ } }
  function clearStore() { try { storage.removeItem(STORE_KEY); } catch { /* ignore */ } }

  // ---------- connecting (candidate race) ----------
  function pageOrigin() { return normalizeOrigin(loc && loc.origin) || null; }

  function connect() {
    if (!S.racing) S.racing = doConnect().finally(() => { S.racing = null; });
    return S.racing;
  }
  async function doConnect() {
    if (!S.store) return false;
    let key;
    try { key = await helloKey(deps.crypto, 'device', S.store.token); } catch { return false; }
    const list = dedupeOrigins([...S.store.candidates, pageOrigin()].filter(Boolean));
    const win0 = await raceHello(netDeps, list, 'device', key, S.store.deviceId);
    if (!win0) { S.base = ''; return false; }
    S.base = win0.origin;
    S.pcName = win0.pcName;
    return true;
  }

  // ---------- API ----------
  async function sendReq(method, path, body, opts) {
    const headers = { 'X-Potato-Phone': '1' };
    if (body !== undefined && body !== null) headers['Content-Type'] = 'application/json';
    if (!opts.noAuth && S.store) headers.Authorization = 'Bearer ' + S.store.token;
    const res = await deps.fetch(S.base + path, {
      method, headers, signal: opts.signal,
      body: body !== undefined && body !== null ? JSON.stringify(body) : undefined
    });
    setConn('online');
    return res;
  }
  async function api(method, path, body, opts = {}) {
    if (!S.base) {
      if (!(await connect())) { setConn('retry'); throw new NetError(); }
    }
    let res;
    try {
      res = await sendReq(method, path, body, opts);
    } catch (e) {
      if (opts.signal && opts.signal.aborted) throw e;
      if (!(await connect())) { setConn('retry'); throw new NetError(); }
      try { res = await sendReq(method, path, body, opts); } catch (e2) {
        if (opts.signal && opts.signal.aborted) throw e2;
        setConn('retry');
        throw new NetError();
      }
    }
    let data = null;
    try { data = await res.json(); } catch { /* no body */ }
    if (res.ok) return data;
    if (res.status === 401 && !opts.noAuth) authLost(data && data.message);
    throw new ApiError(res.status, data && data.error, (data && data.message) || t(res.status === 429 ? 'err.rate' : 'err.generic'));
  }
  const isAuth = (e) => e instanceof ApiError && e.status === 401;
  const isSpacesOff = (e) => e instanceof ApiError && e.status === 409 && e.code === 'spaces_off';
  const errMsg = (e, key) => (e instanceof ApiError && e.message ? e.message : t(key));

  function forgetLocal() {
    stopLoop(); stopPresence();
    clearStore();
    S.store = null; S.base = ''; S.me = null; S.view = null; S.pending = null;
  }
  function authLost(message) {
    forgetLocal();
    showNoQr(message || t('err.signed_out'));
  }

  // ---------- simple screens ----------
  function statusScreen(name, { title, text, notice, button }) {
    setScreen(name, h('section', { class: 'screen status' },
      topbar(h('div', { class: 'brand', 'aria-hidden': 'true' }), h('div', { class: 'bar-title' }, h('div', { class: 'brand-name', text: t('app.name') })), connEl()),
      h('div', { class: 'status-body' },
        h('div', { class: 'status-inner' },
          name === 'loading' ? h('div', { class: 'spinner', 'aria-hidden': 'true' }) : h('div', { class: 'logo', 'aria-hidden': 'true' }),
          title ? h('h1', { text: title }) : null,
          notice ? h('p', { class: 'notice', role: 'alert', text: notice }) : null,
          text ? h('p', { class: 'lead', text }) : null,
          button ? h('button', { class: 'btn primary', type: 'button', onclick: button.onclick, text: button.label }) : null))));
  }
  function showNoQr(notice) {
    statusScreen('pair', { title: t('pair.title'), text: t('pair.noqr'), notice });
  }
  function showLoading(text) {
    statusScreen('loading', { text: text || t('common.loading') });
  }
  function showUnreachable() {
    stopLoop();
    S.view = null;
    statusScreen('unreachable', {
      title: t('unreach.title'), text: t('unreach.text'),
      button: { label: t('common.try_again'), onclick: () => { tryAgain(); } }
    });
  }
  function showPairFailed() {
    stopLoop();
    statusScreen('pair-failed', {
      title: t('unreach.title'), text: t('pair.failed'),
      button: { label: t('common.try_again'), onclick: () => { tryAgain(); } }
    });
  }
  function showError(msg) {
    statusScreen('error', {
      title: t('app.name'), text: msg,
      button: { label: t('common.try_again'), onclick: () => { tryAgain(); } }
    });
  }
  async function tryAgain() {
    if (S.pending) return runPair();
    if (!S.store) return showNoQr();
    return enter();
  }

  // ---------- boot / pair ----------
  async function boot() {
    const frag = parsePairFragment(loc.hash);
    if (frag) {
      S.pending = frag;
      clearFragment();
      return runPair();
    }
    S.store = loadStore();
    if (!S.store) return showNoQr();
    S.route = parseRoute(loc.hash);
    return enter();
  }
  function clearFragment() {
    try { win.history.replaceState(null, '', win.location.pathname + win.location.search); } catch { /* ignore */ }
  }
  function deviceName() {
    const ua = String(nav.userAgent || '');
    if (/iPhone/i.test(ua)) return 'iPhone';
    if (/iPad/i.test(ua)) return 'iPad';
    const m = /Android[^;)]*;\s*([^;)]+)/i.exec(ua);
    if (m && m[1] && !/^(wv|mobile)$/i.test(m[1].trim())) return m[1].trim().slice(0, 40);
    if (/Android/i.test(ua)) return 'Android phone';
    return 'Phone';
  }
  async function runPair() {
    const { code, candidates } = S.pending;
    showLoading(t('pair.working'));
    let key;
    try { key = await helloKey(deps.crypto, 'pair', code); } catch { return showError(t('err.generic')); }
    const list = dedupeOrigins([...candidates, pageOrigin()].filter(Boolean));
    const winner = await raceHello(netDeps, list, 'pair', key);
    if (!winner) return showPairFailed();
    S.base = winner.origin;
    S.store = null;
    try {
      const res = await api('POST', '/api/pair', { code, deviceName: deviceName() }, { noAuth: true });
      S.store = {
        token: String(res.token),
        deviceId: String(res.deviceId || ''),
        candidates: dedupeOrigins([...(res.candidates || []), ...candidates, winner.origin])
      };
      saveStore();
      S.pending = null;
      S.route = { name: 'list' };
      return afterConnect();
    } catch (e) {
      if (e instanceof NetError) return showUnreachable();
      S.pending = null;
      return showNoQr(errMsg(e, 'err.generic'));
    }
  }

  async function enter() {
    showLoading();
    if (!(await connect())) return showUnreachable();
    return afterConnect();
  }
  async function afterConnect() {
    try {
      const me = await api('GET', '/api/me');
      S.me = me;
      S.spacesOn = me.spaces === true;
      const fresh = Array.isArray(me.candidates) ? dedupeOrigins(me.candidates) : [];
      if (fresh.length && S.store) { S.store.candidates = fresh; saveStore(); }
    } catch (e) {
      if (isAuth(e)) return;
      if (e instanceof NetError) return showUnreachable();
      return showError(errMsg(e, 'err.generic'));
    }
    S.visible = doc.visibilityState !== 'hidden';
    await go(S.route, { push: false });
    startLoop();
    startPresence();
  }

  // ---------- network-change handling ----------
  async function onNetworkChange() {
    if (!S.store || S.destroyed) return;
    const before = S.base;
    const ok = await connect();
    if (!ok) return showUnreachable();
    if (S.screen === 'unreachable' || S.screen === 'loading') return afterConnect();
    if (S.base !== before) { stopLoop(); startLoop(); }
    refresh();
  }

  // ---------- navigation ----------
  async function go(route, { push = true } = {}) {
    if (SPACE_ROUTES.has(route.name) && !S.spacesOn) route = { name: 'list' };
    S.route = route;
    S.seq++;
    if (push) { try { win.history.pushState(null, '', routeHash(route)); } catch { /* ignore */ } }
    if (S.view && typeof S.view.destroy === 'function') { try { S.view.destroy(); } catch { /* ignore */ } }
    if (route.name === 'bot') S.view = buildChat(route.id);
    else if (route.name === 'room') S.view = buildRoom(route.id);
    else if (route.name === 'computer') S.view = buildComputer(route.id);
    else if (route.name === 'channel') S.view = buildChannel(route.sid, route.cid);
    else S.view = null;
    if (route.name === 'bot') api('POST', '/api/bots/' + enc(route.id) + '/read').catch(() => {});
    if ((route.name === 'bot' || route.name === 'computer') && !S.botsKnown) learnBots();
    if (S.view) setScreen(route.name, S.view.el);
    else setScreen('list', h('section', { class: 'screen' }, h('div', { class: 'loading', text: t('common.loading') })));
    await refresh();
  }

  // Spaces was turned off on the PC: drop the tab and go home.
  function spacesGone() {
    S.spacesOn = false; S.spaces = null;
    if (SPACE_ROUTES.has(S.route.name)) go({ name: 'list' }, { push: false }).then(() => { try { win.history.replaceState(null, '', '#/'); } catch { /* ignore */ } });
    else if (S.route.name === 'list') { const sc = host.querySelector('.tabbar'); if (sc) sc.remove(); }
  }

  // The chat reply does not say whether a bot has a computer; the list does. Learn it once when a chat is opened first.
  async function learnBots() {
    try {
      const d = await api('GET', '/api/bots');
      noteBots(d && d.bots);
      if (S.view && typeof S.view.paintActions === 'function') S.view.paintActions();
    } catch { /* the header just stays without the button */ }
  }
  function noteBots(list) {
    if (!Array.isArray(list)) return;
    S.botsKnown = true;
    for (const b of list) if (b && b.id != null) S.botsById[String(b.id)] = b;
  }

  async function refresh() {
    if (!S.store || S.destroyed || S.screen === 'unreachable' || S.screen === 'pair') return;
    if (S.route.name === 'computer') return;
    const seq = ++S.seq;
    const route = S.route;
    try {
      let data;
      if (route.name === 'channel') {
        if (S.view && S.view.reload) await S.view.reload();
        return;
      }
      if (route.name === 'spaces' || route.name === 'space') {
        const sp = await api('GET', '/api/spaces');
        if (seq !== S.seq || S.route !== route) return;
        S.spaces = sp;
        if (route.name === 'spaces') renderSpaces(); else renderSpace(route.id);
        return;
      }
      if (route.name === 'bot') data = await api('GET', `/api/bots/${enc(route.id)}?limit=200`);
      else if (route.name === 'room') data = await api('GET', `/api/rooms/${enc(route.id)}`);
      else {
        const wantSpaces = route.name === 'list' && S.spacesOn;
        const [bots, sp] = await Promise.all([
          api('GET', '/api/bots'),
          wantSpaces ? api('GET', '/api/spaces').catch((e) => { if (isSpacesOff(e)) spacesGone(); return null; }) : null
        ]);
        data = bots;
        if (sp) S.spaces = sp;
      }
      if (seq !== S.seq || S.route !== route) return;
      if (data && typeof data.rev === 'number') S.rev = Math.max(S.rev, data.rev);
      if (route.name === 'list') renderList(data);
      else if (S.view) S.view.update(data);
    } catch (e) {
      if (isSpacesOff(e)) return spacesGone();
      if (isAuth(e) || (seq !== S.seq)) return;
      if (e instanceof NetError) return showUnreachable();
      toast(errMsg(e, 'err.generic'));
    }
  }

  // ---------- live updates: long poll ----------
  function startLoop() {
    if (S.loop || !S.store || !S.visible || S.destroyed) return;
    const gen = ++S.loopGen;
    const ctrl = new AbortController();
    S.loop = { gen, ctrl };
    runLoop(gen, ctrl);
  }
  function stopLoop() {
    if (S.loop) { try { S.loop.ctrl.abort(); } catch { /* ignore */ } S.loop = null; }
  }
  const sleep = (ms) => new Promise((r) => setT(r, ms));
  async function runLoop(gen, ctrl) {
    let backoff = 0;
    const alive = () => S.loop && S.loop.gen === gen;
    while (alive()) {
      const t0 = now();
      try {
        const r = await api('GET', `/api/changes?since=${S.rev}&wait_ms=20000`, null, { signal: ctrl.signal });
        if (!alive()) return;
        backoff = 0;
        if (r && typeof r.rev === 'number' && r.rev !== S.rev) {
          const target = r.rev;
          await refresh();
          if (S.rev < target) S.rev = target;
        } else if (now() - t0 < 300) {
          await sleep(1000);
        }
      } catch (e) {
        if (!alive() || ctrl.signal.aborted) return;
        setConn('retry');
        backoff = backoff ? Math.min(backoff * 2, 15000) : 1000;
        await sleep(backoff);
      }
    }
  }

  // ---------- presence ----------
  function postPresence(visible) {
    if (!S.store || S.screen === 'unreachable' || S.screen === 'pair') return Promise.resolve();
    return api('POST', '/api/push/presence', { visible }).catch(() => {});
  }
  function startPresence() {
    stopPresence();
    if (!S.store || !S.visible) return;
    postPresence(true);
    const tick = () => {
      S.presenceTimer = setT(() => {
        S.presenceTimer = null;
        if (!S.visible || !S.store || S.destroyed) return;
        postPresence(true).then(tick, tick);
      }, 20000);
    };
    tick();
  }
  function stopPresence() {
    if (S.presenceTimer) { clearT(S.presenceTimer); S.presenceTimer = null; }
  }

  // ---------- sheets ----------
  function openSheet(build) {
    const close = () => { backdrop.remove(); doc.removeEventListener('keydown', onKey); };
    const onKey = (ev) => { if (ev.key === 'Escape') close(); };
    const sheet = h('div', { class: 'sheet', role: 'dialog', 'aria-modal': 'true' });
    const backdrop = h('div', { class: 'backdrop', onclick: (ev) => { if (ev.target === backdrop) close(); } }, sheet);
    build(sheet, close);
    overlay.append(backdrop);
    doc.addEventListener('keydown', onKey);
    return close;
  }
  function confirmSheet({ title, body, ok, danger }) {
    return new Promise((resolve) => {
      let closeFn;
      const finish = (v) => { closeFn(); resolve(v); };
      closeFn = openSheet((sheet) => {
        sheet.append(
          h('h2', { text: title }),
          body ? h('p', { text: body }) : null,
          h('div', { class: 'sheet-actions' },
            h('button', { class: 'btn', type: 'button', 'data-act': 'cancel', onclick: () => finish(false), text: t('common.cancel') }),
            h('button', { class: 'btn primary' + (danger ? ' danger' : ''), type: 'button', 'data-act': 'confirm', onclick: () => finish(true), text: ok })));
      });
    });
  }

  // ---------- shared bits ----------
  function dot(state, label) {
    return h('span', { class: 'dot', 'data-state': state || 'idle', role: 'img', 'aria-label': label || stateLabel(state) });
  }
  function stateLabel(state) {
    const k = 'state.' + state;
    const s = t(k);
    return s === k ? String(state || '') : s;
  }
  function topbar(...kids) {
    return h('div', { class: 'head' },
      h('header', { class: 'bar' }, ...kids),
      h('div', { class: 'netbar', role: 'status', hidden: true }));
  }
  function initial(name) {
    const ch = Array.from(String(name || '').trim())[0];
    return ch ? ch.toUpperCase() : '?';
  }
  function avatar(name, state, label) {
    return h('span', { class: 'avatar' },
      h('span', { class: 'avatar-ch', 'aria-hidden': 'true', text: initial(name) }),
      state ? dot(state, label) : null);
  }
  /** Where a message sits in a run from the same sender: single | first | mid | last. */
  function groupPos(keys, ats, i) {
    const same = (a, b) => keys[a] === keys[b] && keys[a] !== 'note' && Math.abs((ats[a] || 0) - (ats[b] || 0)) < 5 * 60000;
    const joinPrev = i > 0 && same(i - 1, i);
    const joinNext = i < keys.length - 1 && same(i, i + 1);
    return joinPrev ? (joinNext ? 'mid' : 'last') : (joinNext ? 'first' : 'single');
  }

  // ---------- bot list ----------
  function renderList(data) {
    const old = host.querySelector('.scroll');
    if (old) S.listScroll = old.scrollTop;
    const paused = !!data.paused;
    const bots = data.bots || [];
    const rooms = S.me && S.me.groups ? data.rooms || [] : [];
    const pcName = (S.me && S.me.pcName) || S.pcName || t('pc.unknown');

    noteBots(bots);
    const bar = topbar(
      h('div', { class: 'bar-title' },
        h('h1', { text: t('list.title') }),
        h('div', { class: 'sub-line' }, connEl(), h('span', { class: 'sub pc-status', text: t('pc.status', { name: pcName }) }))),
      paused ? null : h('button', { class: 'btn small', type: 'button', 'data-act': 'pause', onclick: onPause, text: t('list.pause') }),
      h('button', { class: 'icon-btn', type: 'button', 'aria-label': t('list.settings'), 'data-act': 'settings', onclick: openSettings },
        h('span', { class: 'gear', 'aria-hidden': 'true' })));

    const scroll = h('div', { class: 'scroll' });
    if (paused) {
      scroll.append(h('button', { class: 'banner', type: 'button', 'data-act': 'resume', onclick: onResume, text: t('list.paused_resume') }));
    }
    if (!bots.length) scroll.append(h('div', { class: 'empty' }, h('div', { class: 'empty-ico', 'aria-hidden': 'true' }), h('p', { text: t('list.empty') })));
    const ul = h('ul', { class: 'rows' });
    for (const b of bots) ul.append(h('li', {}, botRow(b)));
    scroll.append(ul);
    if (rooms.length) {
      scroll.append(h('h2', { class: 'section', text: t('list.rooms') }));
      const rl = h('ul', { class: 'rows', 'data-list': 'rooms' });
      for (const r of rooms) rl.append(h('li', {}, roomRow(r)));
      scroll.append(rl);
    }
    host.replaceChildren(h('section', { class: 'screen list' }, bar, scroll, tabBar('bots')));
    S.screen = 'list';
    paintConn();
    scroll.scrollTop = S.listScroll;
  }
  function botRow(b) {
    const needs = b.state === 'needs_you';
    const showState = b.state && b.state !== 'idle' && !needs;
    return h('button', { class: 'row', type: 'button', 'data-bot': b.id, 'data-state': b.state, onclick: () => go({ name: 'bot', id: b.id }) },
      avatar(b.name, b.state, b.stateText || stateLabel(b.state)),
      h('span', { class: 'row-main' },
        h('span', { class: 'row-top' },
          h('span', { class: 'row-name', text: b.name }),
          showState ? h('span', { class: 'row-state', text: b.stateText || stateLabel(b.state) }) : null,
          h('span', { class: 'row-time', text: fmtTime(b.lastAt, now()) })),
        h('span', { class: 'row-bottom' },
          h('span', { class: 'row-last', text: b.typing ? t('chat.typing') + '...' : (b.lastLine || t('list.no_messages')) }),
          needs ? h('span', { class: 'tag', text: t('list.needs_you') }) : null,
          b.unread > 0 ? h('span', { class: 'count', 'aria-label': t('list.unread', { n: b.unread }), text: String(b.unread) }) : null)));
  }
  function roomRow(r) {
    const typing = r.typing && r.typing.length;
    return h('button', { class: 'row', type: 'button', 'data-room': r.id, onclick: () => go({ name: 'room', id: r.id }) },
      h('span', { class: 'avatar' },
        h('span', { class: 'avatar-ch', 'aria-hidden': 'true', text: initial(r.name) }),
        h('span', { class: 'dot', 'data-state': typing ? 'working' : 'idle', 'aria-hidden': 'true' })),
      h('span', { class: 'row-main' },
        h('span', { class: 'row-top' },
          h('span', { class: 'row-name', text: r.name }),
          h('span', { class: 'row-time', text: fmtTime(r.lastMs, now()) })),
        h('span', { class: 'row-bottom' },
          h('span', { class: 'row-last', text: typing ? t('room.typing', { names: r.typing.join(', ') }) : (r.bots || []).join(', ') }),
          r.unread > 0 ? h('span', { class: 'count', text: String(r.unread) }) : null)));
  }
  // ---------- spaces: tabs, list, channels ----------
  const spaceUnread = (sp) => (typeof sp.unread === 'number' ? sp.unread : (sp.channels || []).reduce((n, c) => n + (c.unread || 0), 0));
  function tabBar(active) {
    if (!S.spacesOn) return null;
    const total = ((S.spaces && S.spaces.spaces) || []).reduce((n, sp) => n + spaceUnread(sp), 0);
    const tab = (name, label, route, badge) => h('button', {
      class: 'tab', type: 'button', role: 'tab', 'data-tab': name, 'aria-selected': active === name ? 'true' : 'false',
      onclick: () => { if (active !== name) go(route); }
    }, h('span', { class: 'tab-ico tab-' + name, 'aria-hidden': 'true' }), h('span', { class: 'tab-label', text: label }),
    badge > 0 ? h('span', { class: 'count tab-count', 'aria-label': t('list.unread', { n: badge }), text: String(badge) }) : null);
    return h('nav', { class: 'tabbar', role: 'tablist' },
      tab('bots', t('tabs.bots'), { name: 'list' }, 0),
      tab('spaces', t('tabs.spaces'), { name: 'spaces' }, total));
  }
  function backTo(route) {
    return go(route, { push: false }).then(() => { try { win.history.replaceState(null, '', routeHash(route)); } catch { /* ignore */ } });
  }
  function backBtn(route) {
    return h('button', { class: 'icon-btn back', type: 'button', 'aria-label': t('common.back'), 'data-act': 'back', onclick: () => backTo(route) },
      h('span', { class: 'chev', 'aria-hidden': 'true' }));
  }
  function offlineLine() {
    return S.spaces && S.spaces.online === false ? h('div', { class: 'quiet-line', role: 'status', text: t('spaces.offline') }) : null;
  }
  const chanLabel = (c) => '# ' + c.name;
  function renderSpaces() {
    const old = host.querySelector('.scroll');
    if (old) S.listScroll = old.scrollTop;
    const list = (S.spaces && S.spaces.spaces) || [];
    const pcName = (S.me && S.me.pcName) || S.pcName || t('pc.unknown');
    const bar = topbar(
      h('div', { class: 'bar-title' },
        h('h1', { text: t('spaces.title') }),
        h('div', { class: 'sub-line' }, connEl(), h('span', { class: 'sub pc-status', text: t('pc.status', { name: pcName }) }))),
      h('button', { class: 'icon-btn', type: 'button', 'aria-label': t('list.settings'), 'data-act': 'settings', onclick: openSettings },
        h('span', { class: 'gear', 'aria-hidden': 'true' })));
    const scroll = h('div', { class: 'scroll' }, offlineLine());
    if (!list.length) scroll.append(h('div', { class: 'empty' }, h('div', { class: 'empty-ico', 'aria-hidden': 'true' }), h('p', { text: t('spaces.empty') })));
    const ul = h('ul', { class: 'rows' });
    for (const sp of list) {
      const n = spaceUnread(sp);
      ul.append(h('li', {}, h('button', { class: 'row', type: 'button', 'data-space': sp.id, onclick: () => go({ name: 'space', id: sp.id }) },
        h('span', { class: 'avatar sq' }, h('span', { class: 'avatar-ch', 'aria-hidden': 'true', text: initial(sp.name) })),
        h('span', { class: 'row-main' },
          h('span', { class: 'row-top' }, h('span', { class: 'row-name', text: sp.name })),
          h('span', { class: 'row-bottom' },
            h('span', { class: 'row-last', text: (sp.channels || []).map((c) => c.name).join(', ') || t('spaces.no_channels') }),
            n > 0 ? h('span', { class: 'count', 'aria-label': t('list.unread', { n }), text: String(n) }) : null)))));
    }
    scroll.append(ul);
    host.replaceChildren(h('section', { class: 'screen list spaces' }, bar, scroll, tabBar('spaces')));
    S.screen = 'spaces';
    paintConn();
    scroll.scrollTop = S.listScroll;
  }
  function renderSpace(sid) {
    const sp = ((S.spaces && S.spaces.spaces) || []).find((x) => String(x.id) === String(sid));
    if (!sp) { backTo({ name: 'spaces' }); return; }
    const chans = sp.channels || [];
    const bar = topbar(
      backBtn({ name: 'spaces' }),
      h('div', { class: 'bar-title' }, h('h1', { text: sp.name }), h('div', { class: 'sub', text: t('spaces.channel_count', { n: chans.length }) })),
      h('div', { class: 'bar-actions' }, connEl()));
    const scroll = h('div', { class: 'scroll' }, offlineLine());
    if (!chans.length) scroll.append(h('div', { class: 'empty' }, h('div', { class: 'empty-ico', 'aria-hidden': 'true' }), h('p', { text: t('spaces.no_channels') })));
    const ul = h('ul', { class: 'rows' });
    for (const c of chans) {
      ul.append(h('li', {}, h('button', { class: 'row chan-row', type: 'button', 'data-channel': c.id, onclick: () => go({ name: 'channel', sid: sp.id, cid: c.id }) },
        h('span', { class: 'chan-hash', 'aria-hidden': 'true', text: '#' }),
        h('span', { class: 'row-main' },
          h('span', { class: 'row-top' }, h('span', { class: 'row-name', text: c.name })),
          c.canWrite === false ? h('span', { class: 'row-bottom' }, h('span', { class: 'row-last', text: t('spaces.read_only_tag') })) : null),
        c.unread > 0 ? h('span', { class: 'count', 'aria-label': t('list.unread', { n: c.unread }), text: String(c.unread) }) : null)));
    }
    scroll.append(ul);
    host.replaceChildren(h('section', { class: 'screen list space' }, bar, scroll));
    S.screen = 'space';
    paintConn();
  }
  async function onPause() {
    if (!(await confirmSheet({ title: t('pause.title'), body: t('pause.body'), ok: t('pause.ok') }))) return;
    try { await api('POST', '/api/pause-all'); await refresh(); } catch (e) { if (!isAuth(e)) toast(errMsg(e, 'err.generic')); }
  }
  async function onResume() {
    try { await api('POST', '/api/resume-all'); await refresh(); } catch (e) { if (!isAuth(e)) toast(errMsg(e, 'err.generic')); }
  }

  // ---------- cards ----------
  function buildCard(botId, card, afterAnswer) {
    const box = h('section', { class: 'card', 'data-card': card.id, 'data-kind': card.kind });
    const buttons = [];
    const choices = Array.isArray(card.choices) ? card.choices : [];
    const add = (label, answer, cls) => {
      const b = h('button', { class: 'btn ' + (cls || ''), type: 'button', text: label });
      b.addEventListener('click', async () => {
        buttons.forEach((x) => { x.disabled = true; });
        try {
          await api('POST', `/api/bots/${enc(botId)}/cards/${enc(card.id)}/answer`, { answer });
          box.remove();
          afterAnswer();
        } catch (e) {
          buttons.forEach((x) => { x.disabled = false; });
          if (!isAuth(e)) toast(errMsg(e, 'err.answer'));
        }
      });
      buttons.push(b);
      return b;
    };
    const actions = h('div', { class: 'card-actions' });
    const head = [];
    if (card.action) head.push(h('div', { class: 'card-action', text: card.action }));

    if (card.login != null) {
      box.setAttribute('data-variant', 'login');
      head.push(h('div', { class: 'card-origin', text: String(card.login) }));
      head.push(h('p', { class: 'card-note', text: t('card.login_pc') }));
      head.push(h('p', { class: 'card-sub', text: t('card.login_hint') }));
      const last = choices.length - 1;
      if (last >= 0 && String(choices[last]).trim().toLowerCase() === 'not now') {
        actions.append(add(t('card.not_now'), String(last)));
      }
    } else if (card.connector != null) {
      box.setAttribute('data-variant', 'connector');
      head.push(h('p', { class: 'card-note', text: t('card.connector', { service: card.connector }) }));
      head.push(h('p', { class: 'card-sub', text: t('card.connector_hint') }));
      choices.forEach((c, i) => { if (i > 0) actions.append(add(String(c), String(i))); });
    } else if (card.keepGoing != null) {
      box.setAttribute('data-variant', 'keepgoing');
      head.push(h('p', { class: 'card-note', text: String(card.keepGoing) }));
      choices.forEach((c, i) => actions.append(add(String(c), String(i), i === 0 ? 'primary' : '')));
    } else if (card.mcpAdd) {
      box.setAttribute('data-variant', 'mcp');
      head.push(h('p', { class: 'card-note', text: t('card.mcp', { name: card.mcpAdd.name }) }));
      if (card.mcpAdd.url) head.push(h('div', { class: 'card-origin', text: String(card.mcpAdd.url) }));
      choices.forEach((c, i) => actions.append(add(String(c), String(i), i === 0 ? 'primary' : '')));
    } else if (card.kind === 'approval') {
      box.setAttribute('data-variant', 'approval');
      actions.append(add(t('card.allow_once'), 'allow_once', 'primary'), add(t('card.allow_job'), 'allow_job'), add(t('card.deny'), 'deny'));
    } else {
      box.setAttribute('data-variant', 'question');
      choices.forEach((c, i) => actions.append(add(String(c), String(i), i === 0 ? 'primary' : '')));
    }
    if (card.why) head.push(h('p', { class: 'card-why', text: t('card.why', { why: card.why }) }));
    box.append(...head, actions);
    return box;
  }

  // ---------- steps ----------
  function stepRow(s) {
    const status = s.status || 'running';
    return h('li', { class: 'step', 'data-status': status },
      h('span', { class: 'step-mark', role: 'img', 'aria-label': t('step.' + status) }),
      h('span', { class: 'step-text' },
        h('span', { class: 'step-tool', text: s.tool || '' }),
        s.detail ? h('span', { class: 'step-detail', text: s.detail }) : null,
        s.why ? h('span', { class: 'step-why', text: s.why }) : null));
  }
  function stepsBlock(steps, running, titleOverride, view) {
    const n = steps.length;
    const title = titleOverride || t(running ? 'step.title_running' : 'step.title_done', { n });
    const det = h('details', { class: 'steps', 'data-running': running ? '1' : '0' },
      h('summary', { text: title }),
      h('ul', {}, steps.map(stepRow)));
    const want = view && view.stepsOpen != null ? view.stepsOpen : running;
    if (want) det.setAttribute('open', '');
    det.querySelector('summary').addEventListener('click', () => { if (view) view.stepsOpen = !det.open; });
    return det;
  }

  // ---------- photo helpers ----------
  function blobToB64(blob) {
    return new Promise((resolve, reject) => {
      const fr = new win.FileReader();
      fr.onload = () => resolve(String(fr.result).split(',')[1] || '');
      fr.onerror = () => reject(fr.error);
      fr.readAsDataURL(blob);
    });
  }
  async function downscale(file) {
    if (typeof win.createImageBitmap !== 'function') return null;
    const bmp = await win.createImageBitmap(file);
    const big = Math.max(bmp.width, bmp.height);
    const scale = Math.min(1, MAX_EDGE / big);
    if (scale === 1 && file.size < 1500000 && /jpe?g/i.test(file.type)) return null;
    const c = doc.createElement('canvas');
    c.width = Math.round(bmp.width * scale);
    c.height = Math.round(bmp.height * scale);
    const ctx = c.getContext && c.getContext('2d');
    if (!ctx) return null;
    ctx.drawImage(bmp, 0, 0, c.width, c.height);
    return new Promise((res) => c.toBlob(res, 'image/jpeg', 0.85));
  }
  async function toPayload(file) {
    let blob = file;
    let name = file.name || 'photo.jpg';
    try {
      const d = await downscale(file);
      if (d) { blob = d; name = name.replace(/\.[^.]+$/, '') + '.jpg'; }
    } catch { /* send the original */ }
    return { name, kind: 'image', data: await blobToB64(blob) };
  }

  // ---------- composer (shared by bot chat and room chat) ----------
  function buildComposer({ placeholder, allowPhotos, onSend }) {
    const st = { files: [], busy: false };
    const ta = h('textarea', { rows: 1, placeholder, 'aria-label': placeholder, enterkeyhint: 'enter', autocomplete: 'off' });
    const chips = h('div', { class: 'chips' });
    const sendBtn = h('button', { class: 'send', type: 'submit', disabled: true },
      h('span', { class: 'sr', text: t('chat.send') }), h('span', { class: 'send-ico', 'aria-hidden': 'true' }));
    let fileInput = null;
    const sync = () => {
      sendBtn.disabled = st.busy || (!ta.value.trim() && !st.files.length);
      ta.style.height = 'auto';
      ta.style.height = Math.min(ta.scrollHeight || 0, 122) + 'px';
    };
    const renderChips = () => {
      chips.replaceChildren(...st.files.map((f, i) => h('span', { class: 'chip' },
        h('span', { class: 'chip-name', text: f.name || 'photo' }),
        h('button', {
          type: 'button', class: 'chip-x', 'aria-label': t('chat.photo_remove'),
          onclick: () => { st.files.splice(i, 1); renderChips(); sync(); }, text: '×'
        }))));
    };
    ta.addEventListener('input', sync);
    // Enter is a new line (phone keyboards). A hardware keyboard can send with Ctrl/Cmd+Enter.
    ta.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter' && (ev.ctrlKey || ev.metaKey) && !sendBtn.disabled) {
        ev.preventDefault();
        if (form.requestSubmit) form.requestSubmit(); else sendBtn.click();
      }
    });
    ta.addEventListener('focus', () => {
      if (S.view && S.view.stick) { S.view.stick(); setT(() => { if (S.view && S.view.stick) S.view.stick(); }, 350); }
    });
    let photoBtn = null;
    if (allowPhotos) {
      fileInput = h('input', { type: 'file', accept: 'image/*', multiple: true, hidden: true, 'aria-label': t('chat.photo') });
      fileInput.addEventListener('change', () => {
        const picked = Array.from(fileInput.files || []);
        for (const f of picked) {
          if (st.files.length >= MAX_PHOTOS) { toast(t('chat.photo_max')); break; }
          st.files.push(f);
        }
        try { fileInput.value = ''; } catch { /* ignore */ }
        renderChips(); sync();
      });
      photoBtn = h('button', { class: 'icon-btn photo', type: 'button', 'aria-label': t('chat.photo'), 'data-act': 'photo', onclick: () => fileInput.click() },
        h('span', { class: 'plus', 'aria-hidden': 'true' }));
    }
    const form = h('form', { class: 'composer', onsubmit: async (ev) => {
      ev.preventDefault();
      const text = ta.value.trim();
      const files = st.files.slice();
      if (st.busy || (!text && !files.length)) return;
      st.busy = true; sync();
      ta.value = ''; st.files = []; renderChips();
      try {
        await onSend(text, files);
      } catch (e) {
        if (!ta.value) ta.value = text;
        st.files = files; renderChips();
        if (!isAuth(e) && !(e && e.quiet)) toast(errMsg(e, 'err.send'));
      } finally { st.busy = false; sync(); }
    } }, photoBtn, fileInput, ta, sendBtn);
    return { form, chips, ta };
  }

  // ---------- bot chat ----------
  function buildChat(botId) {
    const v = { id: botId, locals: [], stepsOpen: null, rendered: false, data: null };
    let localSeq = 0;
    const title = h('div', { class: 'bar-title' });
    const msgs = h('div', { class: 'msgs' });
    const stepsHost = h('div', { class: 'steps-host' });
    const typingEl = h('div', { class: 'typing', hidden: true, 'aria-label': t('chat.typing') },
      h('span', {}), h('span', {}), h('span', {}));
    // Pending cards are part of the conversation: they sit at the bottom of the list, so the list keeps the height.
    const cardsEl = h('div', { class: 'cards' });
    const scroll = h('div', { class: 'scroll chat-scroll' }, msgs, stepsHost, cardsEl, typingEl);
    const jobBar = h('div', { class: 'jobbar', hidden: true });
    const needsPill = h('button', { class: 'needs-pill', type: 'button', 'data-act': 'needs', hidden: true, onclick: () => v.stick() });
    const composer = buildComposer({
      placeholder: t('chat.placeholder'), allowPhotos: true,
      onSend: async (text, files) => {
        const local = { id: 'local-' + (++localSeq), from: { kind: 'you' }, kind: 'text', text, at: now(), local: true, nfiles: files.length };
        v.locals.push(local);
        renderMsgs(true);
        try {
          const payload = await Promise.all(files.map(toPayload));
          const r = await api('POST', `/api/bots/${enc(botId)}/send`, { text, files: payload });
          local.sid = r && r.id != null ? String(r.id) : null;
          await refresh();
          v.locals = v.locals.filter((l) => !l.sid);
          renderMsgs(true);
        } catch (e) {
          v.locals = v.locals.filter((l) => l !== local);
          renderMsgs(false);
          throw e;
        }
      }
    });
    const dock = h('footer', { class: 'dock' }, needsPill, composer.chips, composer.form);
    const back = h('button', { class: 'icon-btn back', type: 'button', 'aria-label': t('common.back'), 'data-act': 'back', onclick: () => go({ name: 'list' }, { push: false }).then(() => { try { win.history.replaceState(null, '', '#/'); } catch { /* ignore */ } }) },
      h('span', { class: 'chev', 'aria-hidden': 'true' }));
    const actions = h('div', { class: 'bar-actions' });
    const head = topbar(back, title, actions);
    head.append(jobBar);
    const el = h('section', { class: 'screen chat' }, head, scroll, dock);
    // keep the list pinned to the newest message unless the reader scrolled up
    let pinned = true;
    const paintPill = () => {
      const n = (v.data && v.data.cards ? v.data.cards.length : 0);
      needsPill.hidden = !(n > 0 && !pinned);
      needsPill.textContent = n > 0 ? t('chat.needs_you', { n }) : '';
    };
    scroll.addEventListener('scroll', () => { pinned = scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < 80; paintPill(); });
    v.stick = () => { scroll.scrollTop = scroll.scrollHeight; pinned = true; paintPill(); };
    v.onViewport = () => { if (pinned) scroll.scrollTop = scroll.scrollHeight; };
    const hasComputer = () => !!((v.data && v.data.bot && v.data.bot.computer) || (S.botsById[botId] && S.botsById[botId].computer));
    v.paintActions = () => {
      actions.replaceChildren(
        hasComputer() ? h('button', { class: 'btn small computer-btn', type: 'button', 'data-act': 'computer', onclick: () => go({ name: 'computer', id: botId }), text: t('chat.computer') }) : null,
        connEl());
    };
    v.paintActions();

    function msgEl(m, pos) {
      const kind = m.from && m.from.kind;
      const mine = kind === 'you';
      const note = kind === 'note' || m.kind === 'note';
      const row = h('div', { class: 'msg ' + (note ? 'note' : mine ? 'mine' : 'theirs'), 'data-kind': m.kind || 'text', 'data-id': m.id, 'data-pos': pos || 'single' });
      const bubble = h('div', { class: 'bubble' });
      if (!mine && !note && (pos === 'single' || pos === 'first' || !pos) && m.from && m.from.name) bubble.append(h('div', { class: 'who', text: m.from.name }));
      if (m.text) bubble.append(h('div', { class: 'txt', text: m.text }));
      if (m.file) bubble.append(h('div', { class: 'attach', text: t('chat.file', { name: m.file.name }) }));
      if (m.chart) bubble.append(h('div', { class: 'attach', text: t('chat.chart', { title: m.chart.title || '' }) }));
      if (m.local && m.nfiles) bubble.append(h('div', { class: 'attach', text: t('chat.photo') + ' x' + m.nfiles }));
      row.append(bubble);
      if (!note) {
        const meta = h('div', { class: 'meta' });
        if (m.queued) meta.append(h('span', { class: 'badge queued', text: t('chat.queued') }));
        if (m.at) meta.append(h('span', { class: 'when', text: fmtTime(m.at, now()) }));
        row.append(meta);
      }
      return row;
    }
    function renderMsgs(stick) {
      const data = v.data;
      const server = (data && data.messages) || [];
      const ids = new Set(server.map((m) => String(m.id)));
      const locals = v.locals.filter((l) => !(l.sid && ids.has(l.sid)));
      const all = [...server, ...locals];
      const nearBottom = scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < 140;
      if (!all.length) msgs.replaceChildren(h('div', { class: 'empty' }, h('div', { class: 'empty-ico', 'aria-hidden': 'true' }), h('p', { text: t('chat.empty') })));
      else {
        const keys = all.map((m) => {
          const k = m.from && m.from.kind;
          return (k === 'note' || m.kind === 'note') ? 'note' : k === 'you' ? 'you' : 'bot:' + ((m.from && (m.from.id || m.from.name)) || '');
        });
        const ats = all.map((m) => Number(m.at) || 0);
        msgs.replaceChildren(...all.map((m, i) => msgEl(m, groupPos(keys, ats, i))));
      }
      if (stick || nearBottom || !v.rendered) { scroll.scrollTop = scroll.scrollHeight; pinned = true; }
    }

    v.update = (data) => {
      v.data = data;
      const bot = data.bot || {};
      title.replaceChildren(
        h('div', { class: 'title-line' }, dot(bot.state, bot.stateText), h('h1', { text: bot.name || '' })),
        bot.stateText ? h('div', { class: 'sub', text: bot.stateText }) : null);
      v.paintActions();
      renderMsgs(false);

      const work = data.work;
      const steps = (work && work.steps) || [];
      const running = !!work && work.endedMs == null;
      stepsHost.replaceChildren(...(steps.length ? [stepsBlock(steps, running, null, v)] : []));
      typingEl.hidden = !(data.typing || bot.typing);

      const job = data.job;
      if (job && !JOB_DONE.has(String(job.state))) {
        const last = steps.length ? steps[steps.length - 1] : null;
        const newest = job.step || (last ? [last.tool, last.detail].filter(Boolean).join(' ') : '');
        jobBar.replaceChildren(
          h('div', { class: 'job-info' },
            h('span', { class: 'job-goal', text: job.goal || t('job.goal') }),
            newest ? h('span', { class: 'job-step', text: newest }) : null,
            h('span', { class: 'job-spend', text: t('job.spent', { amount: fmtMoney(job.spendNano) }) })),
          h('button', { class: 'btn small', type: 'button', 'data-act': 'stop', onclick: onStop, text: t('job.stop') }));
        jobBar.hidden = false;
      } else {
        jobBar.replaceChildren();
        jobBar.hidden = true;
      }

      const prevCards = v.cardCount || 0;
      const cards = data.cards || [];
      cardsEl.replaceChildren(...cards.map((c) => buildCard(botId, c, () => refresh())));
      v.cardCount = cards.length;
      v.rendered = true;
      // a card that just arrived is scrolled into view; otherwise only follow if the reader is at the bottom
      if (pinned || cards.length > prevCards) { scroll.scrollTop = scroll.scrollHeight; pinned = true; }
      paintPill();
    };
    async function onStop() {
      if (!(await confirmSheet({ title: t('job.stop_title'), body: t('job.stop_body'), ok: t('job.stop_ok'), danger: true }))) return;
      try { await api('POST', `/api/bots/${enc(botId)}/stop`); await refresh(); } catch (e) { if (!isAuth(e)) toast(errMsg(e, 'err.generic')); }
    }
    v.el = el;
    return v;
  }

  // ---------- computer (live screen of the bot's sandbox) ----------
  const SCREEN_STATES = new Set(['loading', 'connecting', 'connected', 'disconnected', 'failed']);
  function buildComputer(botId) {
    const v = { id: botId };
    let mode = 'view';
    let gen = 0;
    let frame = null;
    let expected = '';
    let gone = false;
    let controlOn = false;   // the PC has granted control to this phone and it must be released
    let lost = false;        // control was given back while the page was away
    let keepTimer = null;
    let hideTimer = null;
    let autoHid = false;
    const botPath = (x) => `/api/bots/${enc(botId)}/screen/${x}`;
    const landscape = () => { try { return !!(win.matchMedia && win.matchMedia('(orientation: landscape) and (max-height: 500px)').matches); } catch { return false; } };
    // fire and forget with keepalive, so it still goes out while the page is closing
    function fire(path) {
      if (!S.store || !S.base) return;
      try {
        Promise.resolve(deps.fetch(S.base + path, {
          method: 'POST', keepalive: true,
          headers: { 'X-Potato-Phone': '1', Authorization: 'Bearer ' + S.store.token }
        })).catch(() => {});
      } catch { /* ignore */ }
    }
    function clearKeep() { if (keepTimer) { clearT(keepTimer); keepTimer = null; } }
    function releaseControl() {
      clearKeep();
      if (!controlOn) return;
      controlOn = false;
      fire(botPath('release'));
    }
    function scheduleKeep() {
      clearKeep();
      keepTimer = setT(async () => {
        keepTimer = null;
        if (!controlOn || gone) return;
        try { await api('POST', botPath('keep')); } catch (e) {
          if (gone || isAuth(e)) return;
          if (e instanceof ApiError) { controlOn = false; mode = 'view'; paintSeg(); load('view', t('screen.control_ended')); return; }
        }
        if (controlOn && !gone) scheduleKeep();
      }, KEEP_MS);
    }

    const dotEl = h('span', { class: 'dot', 'data-state': 'working', 'aria-hidden': 'true' });
    const statusText = h('span', { class: 'screen-status-text' });
    const retryBtn = h('button', { class: 'btn small', type: 'button', 'data-act': 'screen-retry', hidden: true, onclick: () => { load(mode); }, text: t('screen.retry') });
    const statusRow = h('div', { class: 'screen-status', role: 'status', 'aria-live': 'polite' }, dotEl, statusText, retryBtn);
    const noteEl = h('div', { class: 'screen-note', hidden: true });
    const stageMsg = h('div', { class: 'stage-msg' });
    const stage = h('div', { class: 'stage' }, stageMsg);

    const segBtn = (m, label) => h('button', {
      class: 'seg-btn', type: 'button', role: 'radio', 'data-mode': m, 'aria-checked': m === mode ? 'true' : 'false',
      onclick: () => { if (m !== mode) { if (m === 'view') releaseControl(); mode = m; paintSeg(); load(m); } }, text: label
    });
    const seg = h('div', { class: 'seg', role: 'radiogroup', 'aria-label': t('screen.mode') }, segBtn('view', t('screen.view')), segBtn('control', t('screen.control')));
    function paintSeg() {
      for (const b of seg.querySelectorAll('.seg-btn')) b.setAttribute('aria-checked', b.getAttribute('data-mode') === mode ? 'true' : 'false');
    }

    const back = h('button', { class: 'icon-btn back', type: 'button', 'aria-label': t('common.back'), 'data-act': 'back',
      onclick: () => go({ name: 'bot', id: botId }, { push: false }).then(() => { try { win.history.replaceState(null, '', routeHash({ name: 'bot', id: botId })); } catch { /* ignore */ } }) },
      h('span', { class: 'chev', 'aria-hidden': 'true' }));
    const botName = h('div', { class: 'sub' });
    v.paintActions = () => {
      const b = S.botsById[botId];
      botName.textContent = (b && b.name) || '';
      botName.hidden = !botName.textContent;
    };
    const title = h('div', { class: 'bar-title' }, h('h1', { text: t('screen.title') }), botName);
    v.paintActions();
    const chromeBtn = h('button', { class: 'chrome-toggle', type: 'button', 'data-act': 'chrome', 'aria-label': t('screen.chrome'),
      onclick: () => { v.el.classList.toggle('chrome-off'); } }, h('span', { class: 'chev-up', 'aria-hidden': 'true' }));
    stage.append(chromeBtn);
    v.el = h('section', { class: 'screen computer' }, topbar(back, title, seg), statusRow, noteEl, stage);

    function setStatus(st, reason) {
      const live = st === 'connected';
      const bad = st === 'failed' || st === 'disconnected';
      dotEl.setAttribute('data-state', bad ? 'problem' : live ? 'live' : 'working');
      let text;
      if (st === 'starting') text = t('screen.starting');
      else if (st === 'loading') text = t('screen.loading');
      else if (st === 'connecting') text = t('screen.connecting');
      else if (st === 'connected') text = t(mode === 'control' ? 'screen.connected_control' : 'screen.connected_view');
      else if (st === 'disconnected') text = t('screen.disconnected');
      else text = st === 'error' ? (reason || t('screen.err')) : t('screen.failed');
      if ((st === 'failed' || st === 'disconnected') && reason) {
        const k = 'screen.reason.' + reason;
        const r = t(k);
        if (r !== k) text += ' ' + r;
      }
      statusText.textContent = text;
      v.status = st;
      const retry = bad || st === 'error';
      retryBtn.hidden = !retry;
      stageMsg.textContent = (st === 'starting' || st === 'error' || retry) ? text : '';
      stageMsg.hidden = !stageMsg.textContent;
      // landscape: once the screen is live the bars get out of the way; a problem brings them back
      if (retry) v.el.classList.remove('chrome-off');
      else if (live && !autoHid && landscape()) { autoHid = true; v.el.classList.add('chrome-off'); }
    }
    function dropFrame() {
      if (frame) { try { frame.remove(); } catch { /* ignore */ } frame = null; }
      expected = '';
    }
    function resolveSrc(raw) {
      const base = S.base || (loc && loc.origin) || '';
      let abs;
      try { abs = new URL(raw, base); } catch { return null; }
      let baseOrigin = '';
      try { baseOrigin = new URL(base).origin; } catch { return null; }
      if ((abs.protocol !== 'http:' && abs.protocol !== 'https:') || abs.origin !== baseOrigin) return null;
      return { origin: abs.origin, src: S.base && S.base !== pageOrigin() ? abs.href : raw };
    }
    async function load(want, note) {
      const my = ++gen;
      dropFrame();
      noteEl.hidden = !note;
      noteEl.textContent = note || '';
      setStatus('starting');
      let res;
      try {
        res = await api('POST', `/api/bots/${enc(botId)}/screen`, { mode: want });
      } catch (e) {
        if (my !== gen || gone || isAuth(e)) return;
        if (want === 'control' && e instanceof ApiError && e.status === 403) {
          mode = 'view'; paintSeg();
          return load('view', t(e.code === 'phone_needs_desktop_yes' ? 'screen.needs_desktop_yes' : 'screen.no_control'));
        }
        setStatus('error', errMsg(e, 'screen.err'));
        return;
      }
      if (my !== gen || gone) return;
      const ok = res && typeof res.src === 'string' && res.src ? resolveSrc(res.src) : null;
      if (!ok) { setStatus('error', t('screen.err')); return; }
      mode = res.mode === 'control' ? 'control' : 'view';
      paintSeg();
      if (mode === 'control') { controlOn = true; lost = false; scheduleKeep(); }
      expected = ok.origin;
      frame = h('iframe', { class: 'screen-frame', title: t('screen.frame'), allow: 'clipboard-read; clipboard-write; fullscreen', referrerpolicy: 'no-referrer' });
      stage.append(frame);
      frame.setAttribute('src', ok.src);
      setStatus('loading');
    }
    const onMessage = (ev) => {
      if (!frame || !frame.contentWindow || ev.source !== frame.contentWindow || ev.origin !== expected) return;
      const d = ev.data;
      if (!d || d.type !== 'pm-sbx-screen-status' || !SCREEN_STATES.has(d.status)) return;
      setStatus(d.status, typeof d.reason === 'string' ? d.reason : '');
    };
    win.addEventListener('message', onMessage);
    // The page went to the background or is closing. Control is given back after a minute away, or at once on close.
    v.onVisibility = (vis) => {
      if (vis) {
        if (hideTimer) { clearT(hideTimer); hideTimer = null; }
        if (lost && !gone) { lost = false; mode = 'view'; paintSeg(); load('view', t('screen.control_ended')); }
      } else if (controlOn && !hideTimer) {
        hideTimer = setT(() => { hideTimer = null; if (controlOn) { releaseControl(); lost = true; } }, AWAY_MS);
      }
    };
    v.onPageHide = () => { if (controlOn) { releaseControl(); lost = true; } };
    v.destroy = () => {
      gone = true; gen++;
      if (hideTimer) { clearT(hideTimer); hideTimer = null; }
      releaseControl();
      win.removeEventListener('message', onMessage);
      dropFrame();
    };
    v.mode = () => mode;
    load('view');
    return v;
  }

  // ---------- group room ----------
  function buildRoom(roomId) {
    const v = { id: roomId, stepsOpen: null };
    const title = h('div', { class: 'bar-title' });
    const lines = h('div', { class: 'msgs' });
    const stepsHost = h('div', { class: 'steps-host' });
    const typing = h('div', { class: 'typing-names sub', hidden: true });
    const scroll = h('div', { class: 'scroll chat-scroll' }, lines, stepsHost, typing);
    const composer = buildComposer({
      placeholder: t('room.placeholder'), allowPhotos: false,
      onSend: async (text) => {
        await api('POST', `/api/rooms/${enc(roomId)}/send`, { text });
        await refresh();
      }
    });
    const back = h('button', { class: 'icon-btn back', type: 'button', 'aria-label': t('common.back'), 'data-act': 'back', onclick: () => go({ name: 'list' }, { push: false }).then(() => { try { win.history.replaceState(null, '', '#/'); } catch { /* ignore */ } }) },
      h('span', { class: 'chev', 'aria-hidden': 'true' }));
    v.el = h('section', { class: 'screen chat room' }, topbar(back, title, h('div', { class: 'bar-actions' }, connEl())), scroll, h('footer', { class: 'dock' }, composer.form));
    let first = true;
    let pinned = true;
    scroll.addEventListener('scroll', () => { pinned = scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < 80; });
    v.stick = () => { scroll.scrollTop = scroll.scrollHeight; pinned = true; };
    v.onViewport = () => { if (pinned) scroll.scrollTop = scroll.scrollHeight; };
    v.update = (data) => {
      const room = data.room || {};
      title.replaceChildren(h('h1', { text: room.name || '' }), h('div', { class: 'sub', text: (room.bots || []).join(', ') }));
      const near = scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < 140;
      const ls = data.lines || [];
      if (!ls.length) lines.replaceChildren(h('div', { class: 'empty' }, h('div', { class: 'empty-ico', 'aria-hidden': 'true' }), h('p', { text: t('room.empty') })));
      else {
        const keys = ls.map((l) => (l.from && (l.from.kind + ':' + (l.from.id || l.from.name))) || '');
        const ats = ls.map((l) => Number(l.at) || 0);
        lines.replaceChildren(...ls.map((l, i) => {
          const person = l.from && l.from.kind === 'person';
          const pos = groupPos(keys, ats, i);
          const bubble = h('div', { class: 'bubble' });
          if ((pos === 'single' || pos === 'first') && l.from && l.from.name) bubble.append(h('div', { class: 'who', text: l.from.name }));
          if (l.text) bubble.append(h('div', { class: 'txt', text: l.text }));
          if (l.file) bubble.append(h('div', { class: 'attach', text: t('chat.file', { name: l.file.name || '' }) }));
          return h('div', { class: 'msg ' + (person ? 'mine' : 'theirs'), 'data-id': l.id, 'data-from': l.from && l.from.kind, 'data-pos': pos },
            bubble, h('div', { class: 'meta' }, h('span', { class: 'when', text: fmtTime(l.at, now()) })));
        }));
      }
      const runs = (data.work || []).filter((w) => w && w.run && (w.run.steps || []).length);
      stepsHost.replaceChildren(...runs.map((w) => stepsBlock(w.run.steps, w.run.endedMs == null, t('room.steps', { bot: w.bot, n: w.run.steps.length }), null)));
      const names = data.typing || [];
      typing.hidden = !names.length;
      typing.textContent = names.length ? t('room.typing', { names: names.join(', ') }) : '';
      if (first || near || pinned) { scroll.scrollTop = scroll.scrollHeight; pinned = true; }
      first = false;
    };
    return v;
  }

  // ---------- spaces: channel chat ----------
  const PAGE = 50;
  function fmtSize(n) {
    const x = Number(n) || 0;
    if (x < 1024) return x + ' B';
    if (x < 1048576) return Math.round(x / 1024) + ' KB';
    return (x / 1048576).toFixed(1) + ' MB';
  }
  function buildChannel(sid, cid) {
    const v = { sid, cid, msgs: [], locals: [], hasMore: false, canWrite: null, loaded: false, readKey: null };
    let localSeq = 0;
    let loadingOlder = false;
    let pinned = true;
    const base = `/api/spaces/${enc(sid)}/channels/${enc(cid)}`;
    const known = () => {
      const sp = ((S.spaces && S.spaces.spaces) || []).find((x) => String(x.id) === String(sid));
      const ch = sp && (sp.channels || []).find((x) => String(x.id) === String(cid));
      return { sp, ch };
    };
    const title = h('div', { class: 'bar-title' });
    const msgs = h('div', { class: 'msgs' });
    const earlier = h('button', { class: 'earlier', type: 'button', 'data-act': 'earlier', hidden: true, onclick: () => loadOlder(), text: t('spaces.earlier') });
    const scroll = h('div', { class: 'scroll chat-scroll' }, earlier, msgs);
    const retryLine = h('div', { class: 'retry-line', role: 'alert', hidden: true, text: t('spaces.retry_line') });
    const roNote = h('div', { class: 'readonly-note', role: 'status', text: t('spaces.readonly') });
    const composer = buildComposer({
      placeholder: t('spaces.placeholder'), allowPhotos: false,
      onSend: async (text) => {
        const local = { id: 'local-' + (++localSeq), mine: true, text, atMs: now(), state: 'pending', files: [], local: true };
        retryLine.hidden = true;
        v.locals.push(local);
        render('stick');
        try {
          const r = await api('POST', base + '/send', { text });
          local.state = 'sent';
          local.sid = r && r.id != null ? String(r.id) : null;
          render('stick');
          await v.reload();
          v.locals = v.locals.filter((l) => !l.sid);
          render('stick');
        } catch (e) {
          v.locals = v.locals.filter((l) => l !== local);
          render('none');
          if (isSpacesOff(e)) spacesGone();
          else if (e instanceof ApiError && e.status === 403 && e.code === 'read_only') { v.canWrite = false; paintDock(); }
          else if (!isAuth(e)) { retryLine.hidden = false; }
          if (e && typeof e === 'object') e.quiet = true;
          throw e;
        }
      }
    });
    composer.ta.addEventListener('input', () => { retryLine.hidden = true; });
    const dock = h('footer', { class: 'dock' });
    function paintDock() {
      if (v.canWrite === true) dock.replaceChildren(retryLine, composer.chips, composer.form);
      else if (v.canWrite === false) dock.replaceChildren(roNote);
      else dock.replaceChildren();
    }
    const head = topbar(backBtn({ name: 'space', id: sid }), title, h('div', { class: 'bar-actions' }, connEl()));
    const quiet = h('div', { class: 'quiet-line', role: 'status', hidden: true, text: t('spaces.offline') });
    head.append(quiet);
    const el = h('section', { class: 'screen chat channel' }, head, scroll, dock);

    function paintTitle() {
      const { sp, ch } = known();
      title.replaceChildren(
        h('h1', { text: ch ? '# ' + ch.name : '' }),
        sp ? h('div', { class: 'sub', text: sp.name }) : null);
      quiet.hidden = !(S.spaces && S.spaces.online === false);
    }
    {
      const { ch } = known();
      if (ch && typeof ch.canWrite === 'boolean') v.canWrite = ch.canWrite;
      paintTitle();
      paintDock();
    }

    function maybeRead() {
      if (!v.loaded || !S.visible || !pinned) return;
      const key = v.msgs.length ? String(v.msgs[v.msgs.length - 1].id) : '-';
      if (v.readKey === key) return;
      v.readKey = key;
      api('POST', base + '/read').catch(() => {});
    }
    scroll.addEventListener('scroll', () => {
      pinned = scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < 80;
      if (pinned) maybeRead();
      if (scroll.scrollTop < 60 && v.hasMore) loadOlder();
    });
    v.stick = () => { scroll.scrollTop = scroll.scrollHeight; pinned = true; maybeRead(); };
    v.onViewport = () => { if (pinned) scroll.scrollTop = scroll.scrollHeight; };
    v.onVisibility = (vis) => { if (vis) maybeRead(); };

    // A bot line is signed by its owner's key, so an older PC may still say mine:true for it. It is the bot talking: always theirs.
    const isMine = (m) => !!m.mine && !m.bot;
    function msgEl(m, pos) {
      const mine = isMine(m);
      const row = h('div', { class: 'msg ' + (mine ? 'mine' : 'theirs'), 'data-id': m.id, 'data-pos': pos, 'data-state': m.state || 'sent' });
      const bubble = h('div', { class: 'bubble' });
      if (!mine && (pos === 'single' || pos === 'first')) {
        bubble.append(h('div', { class: 'who' }, h('span', { class: 'who-name', text: m.authorName || '' }), m.bot ? h('span', { class: 'botmark', text: t('spaces.bot') }) : null, m.bot && m.ownerName ? h('span', { class: 'who-owner', text: m.ownedByMe ? t('spaces.bot_yours') : m.ownerName }) : null));
      }
      if (m.text) bubble.append(h('div', { class: 'txt', text: m.text }));
      for (const f of m.files || []) bubble.append(h('div', { class: 'attach', text: t('chat.file', { name: f.name + (f.size != null ? ' (' + fmtSize(f.size) + ')' : '') }) }));
      row.append(bubble);
      const meta = h('div', { class: 'meta' });
      if (m.state === 'pending') meta.append(h('span', { class: 'badge queued', text: t('spaces.pending') }));
      if (m.state === 'failed') meta.append(h('span', { class: 'badge failed', text: t('spaces.failed') }));
      if (m.atMs) meta.append(h('span', { class: 'when', text: fmtTime(m.atMs, now()) }));
      row.append(meta);
      return row;
    }
    function render(mode) {
      const ids = new Set(v.msgs.map((m) => String(m.id)));
      const all = [...v.msgs, ...v.locals.filter((l) => !(l.sid && ids.has(l.sid)))];
      const near = scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < 140;
      if (!all.length) {
        msgs.replaceChildren(...(v.loaded ? [h('div', { class: 'empty' }, h('div', { class: 'empty-ico', 'aria-hidden': 'true' }), h('p', { text: t('spaces.channel_empty') }))] : []));
      } else {
        const keys = all.map((m) => (isMine(m) ? 'you' : 'a:' + (m.bot ? 'b:' : '') + (m.authorName || '')));
        const ats = all.map((m) => Number(m.atMs) || 0);
        msgs.replaceChildren(...all.map((m, i) => msgEl(m, groupPos(keys, ats, i))));
      }
      earlier.hidden = !v.hasMore;
      earlier.textContent = loadingOlder ? t('common.loading') : t('spaces.earlier');
      if (mode === 'stick' || (mode !== 'none' && (near || pinned))) { scroll.scrollTop = scroll.scrollHeight; pinned = true; }
      maybeRead();
    }

    async function loadOlder() {
      if (loadingOlder || !v.hasMore || !v.msgs.length) return;
      loadingOlder = true;
      earlier.textContent = t('common.loading');
      const oldH = scroll.scrollHeight;
      const oldTop = scroll.scrollTop;
      try {
        const d = await api('GET', `${base}/messages?before=${enc(v.msgs[0].id)}&limit=${PAGE}`);
        const have = new Set(v.msgs.map((m) => String(m.id)));
        const older = ((d && d.messages) || []).filter((m) => !have.has(String(m.id)));
        v.msgs = [...older, ...v.msgs];
        v.hasMore = !!(d && d.hasMore);
        loadingOlder = false;
        render('none');
        scroll.scrollTop = oldTop + (scroll.scrollHeight - oldH);
      } catch (e) {
        loadingOlder = false;
        render('none');
        if (isSpacesOff(e)) spacesGone();
        else if (!isAuth(e)) toast(errMsg(e, 'err.generic'));
      }
    }

    v.reload = async () => {
      const [d, sp] = await Promise.all([
        api('GET', `${base}/messages?limit=${PAGE}`),
        api('GET', '/api/spaces').catch((e) => { if (isSpacesOff(e)) throw e; return null; })
      ]);
      if (sp) S.spaces = sp;
      const fresh = (d && d.messages) || [];
      const first = fresh.length ? String(fresh[0].id) : null;
      const at = first == null ? -1 : v.msgs.findIndex((m) => String(m.id) === first);
      if (at >= 0) v.msgs = [...v.msgs.slice(0, at), ...fresh];
      else { v.msgs = fresh; v.hasMore = !!(d && d.hasMore); }
      if (at === 0 || !v.loaded) v.hasMore = !!(d && d.hasMore);
      const wasLoaded = v.loaded;
      v.loaded = true;
      const ch = known().ch;
      v.canWrite = d && typeof d.canWrite === 'boolean' ? d.canWrite : (ch && typeof ch.canWrite === 'boolean' ? ch.canWrite : v.canWrite);
      paintTitle();
      paintDock();
      render(wasLoaded ? 'auto' : 'stick');
    };
    v.el = el;
    return v;
  }

  // ---------- settings ----------
  async function pushReg() {
    if (win.isSecureContext === false) return null;
    const sw = nav.serviceWorker;
    const N = deps.Notification || win.Notification;
    if (!sw || !N) return null;
    try {
      const reg = await sw.ready;
      return reg && reg.pushManager ? { reg, N } : null;
    } catch { return null; }
  }
  function openSettings() {
    openSheet((sheet, close) => {
      const me = S.me || {};
      const toggle = h('button', { class: 'switch', type: 'button', role: 'switch', 'aria-checked': 'false', 'aria-label': t('settings.notifications'), 'data-act': 'notif', disabled: true },
        h('span', { class: 'knob' }));
      const note = h('p', { class: 'sub', text: t('settings.notif_hint') });
      sheet.append(
        h('h2', { text: t('settings.title') }),
        h('div', { class: 'group' },
          h('h3', { text: t('settings.phone') }),
          h('div', { class: 'kv' }, h('span', { text: t('settings.name') }), h('strong', { class: 'v-name', text: me.name || '' })),
          h('div', { class: 'kv' }, h('span', { text: t('settings.pc') }), h('strong', { class: 'v-pc', text: me.pcName || S.pcName || '' }))),
        h('div', { class: 'group' },
          h('div', { class: 'kv' }, h('h3', { text: t('settings.notifications') }), toggle),
          note),
        h('p', { class: 'sub', text: t('settings.language') }),
        h('div', { class: 'sheet-actions col' },
          h('button', { class: 'btn danger-text', type: 'button', 'data-act': 'forget', onclick: async () => {
            if (!(await confirmSheet({ title: t('settings.forget_title'), body: t('settings.forget_body'), ok: t('settings.forget_ok'), danger: true }))) return;
            try { await api('DELETE', '/api/me'); } catch { /* forget locally anyway */ }
            close();
            forgetLocal();
            showNoQr();
          }, text: t('settings.forget') }),
          h('button', { class: 'btn', type: 'button', onclick: close, text: t('common.close') })));

      (async () => {
        const p = await pushReg();
        if (!p) { note.textContent = t(win.isSecureContext === false ? 'settings.notif_insecure' : 'settings.notif_unsupported'); return; }
        let sub = null;
        try { sub = await p.reg.pushManager.getSubscription(); } catch { /* ignore */ }
        const paint = (on) => { toggle.setAttribute('aria-checked', on ? 'true' : 'false'); };
        paint(!!sub);
        toggle.disabled = false;
        toggle.addEventListener('click', async () => {
          const on = toggle.getAttribute('aria-checked') === 'true';
          toggle.disabled = true;
          try {
            if (on) {
              const cur = await p.reg.pushManager.getSubscription();
              await api('DELETE', '/api/push/subscribe');
              if (cur) await cur.unsubscribe();
              paint(false);
            } else {
              const perm = await p.N.requestPermission();
              if (perm !== 'granted') { note.textContent = t('settings.notif_denied'); return; }
              const { publicKey } = await api('GET', '/api/push/key');
              const s = await p.reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: b64ToBytes(publicKey) });
              const j = typeof s.toJSON === 'function' ? s.toJSON() : s;
              await api('POST', '/api/push/subscribe', { endpoint: j.endpoint, keys: { p256dh: j.keys.p256dh, auth: j.keys.auth } });
              paint(true);
            }
          } catch (e) {
            if (!isAuth(e)) note.textContent = errMsg(e, 'settings.notif_failed');
          } finally { toggle.disabled = false; }
        });
      })();
    });
  }

  // ---------- global events ----------
  const listeners = [];
  function on(target, type, fn) {
    if (!target || !target.addEventListener) return;
    target.addEventListener(type, fn);
    listeners.push(() => target.removeEventListener(type, fn));
  }
  if (vv) { on(vv, 'resize', onViewportEvent); on(vv, 'scroll', onViewportEvent); }
  on(win, 'resize', onViewportEvent);
  applyViewport();
  on(win, 'online', () => { paintConn(); onNetworkChange(); });
  on(win, 'offline', () => { paintConn(); onNetworkChange(); });
  on(nav.connection, 'change', onNetworkChange);
  on(win, 'popstate', () => {
    if (!S.store || S.screen === 'unreachable') return;
    const r = parseRoute(win.location.hash);
    if (sameRoute(r, S.route)) return;
    go(r, { push: false });
  });
  on(win, 'pagehide', () => { if (S.view && S.view.onPageHide) S.view.onPageHide(); });
  on(win, 'pageshow', (ev) => { if (ev && ev.persisted && S.view && S.view.onVisibility) S.view.onVisibility(true); });
  on(doc, 'visibilitychange', () => {
    const vis = doc.visibilityState !== 'hidden';
    S.visible = vis;
    if (S.view && S.view.onVisibility) S.view.onVisibility(vis);
    if (!S.store) return;
    postPresence(vis);
    if (!vis) {
      S.hiddenAt = now();
      stopLoop(); stopPresence();
      return;
    }
    const away = S.hiddenAt ? now() - S.hiddenAt : 0;
    S.hiddenAt = 0;
    (async () => {
      if (away > 30000) {
        const ok = await connect();
        if (!ok) return showUnreachable();
        if (S.screen === 'unreachable') return afterConnect();
      }
      await refresh();
      startLoop();
      startPresence();
    })();
  });

  if (nav.serviceWorker && typeof nav.serviceWorker.register === 'function') {
    try { Promise.resolve(nav.serviceWorker.register('sw.js')).catch(() => {}); } catch { /* ignore */ }
  }

  boot();

  return {
    destroy() {
      S.destroyed = true;
      stopLoop(); stopPresence();
      if (S.view && typeof S.view.destroy === 'function') { try { S.view.destroy(); } catch { /* ignore */ } }
      listeners.forEach((f) => f());
    }
  };
}

if (!globalThis.__POTATO_PHONE_NO_AUTOMOUNT && typeof document !== 'undefined') {
  const rootEl = document.getElementById('app');
  if (rootEl) {
    mountPhoneApp(rootEl, {
      fetch: (...a) => globalThis.fetch(...a),
      storage: globalThis.localStorage,
      location: globalThis.location,
      navigator: globalThis.navigator,
      now: () => Date.now(),
      setTimeout: (f, ms) => globalThis.setTimeout(f, ms),
      crypto: globalThis.crypto
    });
  }
}
