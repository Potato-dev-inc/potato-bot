// Potato Bot phone app. Plain ES modules, no build step.
// Server text is only ever put into the page with textContent (never as markup).
import { t, setLang, pickLang, getLang, fmtMoney, fmtTime } from './i18n.js';
import {
  STORE_KEY, ApiError, NetError, helloKey, raceHello, b64ToBytes, normalizeOrigin, dedupeOrigins
} from './net.js';

const MAX_PHOTOS = 3;
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
  const m = /^#\/(bot|room)\/([^/?]+)/.exec(String(hash || ''));
  if (!m) return { name: 'list' };
  try { return { name: m[1], id: decodeURIComponent(m[2]) }; } catch { return { name: 'list' }; }
}
function routeHash(r) { return r.name === 'list' ? '#/' : '#/' + r.name + '/' + enc(r.id); }

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
    hiddenAt: 0, presenceTimer: null, pending: null, destroyed: false, toastTimer: null, listScroll: 0
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

  function setScreen(name, el) {
    S.screen = name;
    host.replaceChildren(el);
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
    return deps.fetch(S.base + path, {
      method, headers, signal: opts.signal,
      body: body !== undefined && body !== null ? JSON.stringify(body) : undefined
    });
  }
  async function api(method, path, body, opts = {}) {
    if (!S.base) {
      if (!(await connect())) throw new NetError();
    }
    let res;
    try {
      res = await sendReq(method, path, body, opts);
    } catch (e) {
      if (opts.signal && opts.signal.aborted) throw e;
      if (!(await connect())) throw new NetError();
      try { res = await sendReq(method, path, body, opts); } catch (e2) {
        if (opts.signal && opts.signal.aborted) throw e2;
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
      h('div', { class: 'status-inner' },
        h('div', { class: 'logo', 'aria-hidden': 'true' }),
        title ? h('h1', { text: title }) : null,
        notice ? h('p', { class: 'notice', role: 'alert', text: notice }) : null,
        text ? h('p', { class: 'lead', text }) : null,
        button ? h('button', { class: 'btn primary', type: 'button', onclick: button.onclick, text: button.label }) : null)));
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
    S.route = route;
    S.seq++;
    if (push) { try { win.history.pushState(null, '', routeHash(route)); } catch { /* ignore */ } }
    if (route.name === 'bot') S.view = buildChat(route.id);
    else if (route.name === 'room') S.view = buildRoom(route.id);
    else S.view = null;
    if (route.name === 'bot') api('POST', '/api/bots/' + enc(route.id) + '/read').catch(() => {});
    if (S.view) setScreen(route.name, S.view.el);
    else setScreen('list', h('section', { class: 'screen' }, h('div', { class: 'loading', text: t('common.loading') })));
    await refresh();
  }

  async function refresh() {
    if (!S.store || S.destroyed || S.screen === 'unreachable' || S.screen === 'pair') return;
    const seq = ++S.seq;
    const route = S.route;
    try {
      let data;
      if (route.name === 'bot') data = await api('GET', `/api/bots/${enc(route.id)}?limit=200`);
      else if (route.name === 'room') data = await api('GET', `/api/rooms/${enc(route.id)}`);
      else data = await api('GET', '/api/bots');
      if (seq !== S.seq || S.route !== route) return;
      if (data && typeof data.rev === 'number') S.rev = Math.max(S.rev, data.rev);
      if (route.name === 'list') renderList(data);
      else if (S.view) S.view.update(data);
    } catch (e) {
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
  function topbar(...kids) { return h('header', { class: 'bar' }, ...kids); }

  // ---------- bot list ----------
  function renderList(data) {
    const old = host.querySelector('.scroll');
    if (old) S.listScroll = old.scrollTop;
    const paused = !!data.paused;
    const bots = data.bots || [];
    const rooms = S.me && S.me.groups ? data.rooms || [] : [];
    const pcName = (S.me && S.me.pcName) || S.pcName || t('pc.unknown');

    const bar = topbar(
      h('div', { class: 'bar-title' },
        h('h1', { text: t('list.title') }),
        h('div', { class: 'sub pc-status', text: t('pc.status', { name: pcName }) })),
      paused ? null : h('button', { class: 'btn small', type: 'button', 'data-act': 'pause', onclick: onPause, text: t('list.pause') }),
      h('button', { class: 'icon-btn', type: 'button', 'aria-label': t('list.settings'), 'data-act': 'settings', onclick: openSettings },
        h('span', { class: 'gear', 'aria-hidden': 'true' })));

    const scroll = h('div', { class: 'scroll' });
    if (paused) {
      scroll.append(h('button', { class: 'banner', type: 'button', 'data-act': 'resume', onclick: onResume, text: t('list.paused_resume') }));
    }
    if (!bots.length) scroll.append(h('p', { class: 'empty', text: t('list.empty') }));
    const ul = h('ul', { class: 'rows' });
    for (const b of bots) ul.append(h('li', {}, botRow(b)));
    scroll.append(ul);
    if (rooms.length) {
      scroll.append(h('h2', { class: 'section', text: t('list.rooms') }));
      const rl = h('ul', { class: 'rows', 'data-list': 'rooms' });
      for (const r of rooms) rl.append(h('li', {}, roomRow(r)));
      scroll.append(rl);
    }
    host.replaceChildren(h('section', { class: 'screen list' }, bar, scroll));
    S.screen = 'list';
    scroll.scrollTop = S.listScroll;
  }
  function botRow(b) {
    const needs = b.state === 'needs_you';
    return h('button', { class: 'row', type: 'button', 'data-bot': b.id, 'data-state': b.state, onclick: () => go({ name: 'bot', id: b.id }) },
      dot(b.state, b.stateText || stateLabel(b.state)),
      h('span', { class: 'row-main' },
        h('span', { class: 'row-top' },
          h('span', { class: 'row-name', text: b.name }),
          h('span', { class: 'row-time', text: fmtTime(b.lastAt, now()) })),
        h('span', { class: 'row-bottom' },
          h('span', { class: 'row-last', text: b.typing ? t('chat.typing') + '...' : (b.lastLine || t('list.no_messages')) }),
          needs ? h('span', { class: 'tag', text: t('list.needs_you') }) : null,
          b.unread > 0 ? h('span', { class: 'count', 'aria-label': t('list.unread', { n: b.unread }), text: String(b.unread) }) : null)));
  }
  function roomRow(r) {
    const typing = r.typing && r.typing.length;
    return h('button', { class: 'row', type: 'button', 'data-room': r.id, onclick: () => go({ name: 'room', id: r.id }) },
      h('span', { class: 'dot', 'data-state': typing ? 'working' : 'idle', 'aria-hidden': 'true' }),
      h('span', { class: 'row-main' },
        h('span', { class: 'row-top' },
          h('span', { class: 'row-name', text: r.name }),
          h('span', { class: 'row-time', text: fmtTime(r.lastMs, now()) })),
        h('span', { class: 'row-bottom' },
          h('span', { class: 'row-last', text: typing ? t('room.typing', { names: r.typing.join(', ') }) : (r.bots || []).join(', ') }),
          r.unread > 0 ? h('span', { class: 'count', text: String(r.unread) }) : null)));
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
    const sendBtn = h('button', { class: 'btn primary send', type: 'submit', disabled: true, text: t('chat.send') });
    let fileInput = null;
    const sync = () => {
      sendBtn.disabled = st.busy || (!ta.value.trim() && !st.files.length);
      ta.style.height = 'auto';
      ta.style.height = Math.min(ta.scrollHeight || 0, 140) + 'px';
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
        if (!isAuth(e)) toast(errMsg(e, 'err.send'));
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
    const scroll = h('div', { class: 'scroll chat-scroll' }, msgs, stepsHost, typingEl);
    const jobBar = h('div', { class: 'jobbar', hidden: true });
    const cardsEl = h('div', { class: 'cards' });
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
    const dock = h('footer', { class: 'dock' }, jobBar, cardsEl, composer.chips, composer.form);
    const back = h('button', { class: 'icon-btn back', type: 'button', 'aria-label': t('common.back'), 'data-act': 'back', onclick: () => go({ name: 'list' }, { push: false }).then(() => { try { win.history.replaceState(null, '', '#/'); } catch { /* ignore */ } }) },
      h('span', { class: 'chev', 'aria-hidden': 'true' }));
    const el = h('section', { class: 'screen chat' }, topbar(back, title), scroll, dock);

    function msgEl(m) {
      const kind = m.from && m.from.kind;
      const mine = kind === 'you';
      const note = kind === 'note' || m.kind === 'note';
      const row = h('div', { class: 'msg ' + (note ? 'note' : mine ? 'mine' : 'theirs'), 'data-kind': m.kind || 'text', 'data-id': m.id });
      const bubble = h('div', { class: 'bubble' });
      if (!mine && !note && m.from && m.from.name) bubble.append(h('div', { class: 'who', text: m.from.name }));
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
      if (!all.length) msgs.replaceChildren(h('p', { class: 'empty', text: t('chat.empty') }));
      else msgs.replaceChildren(...all.map(msgEl));
      if (stick || nearBottom || !v.rendered) scroll.scrollTop = scroll.scrollHeight;
    }

    v.update = (data) => {
      v.data = data;
      const bot = data.bot || {};
      title.replaceChildren(
        h('div', { class: 'title-line' }, dot(bot.state, bot.stateText), h('h1', { text: bot.name || '' })),
        bot.stateText ? h('div', { class: 'sub', text: bot.stateText }) : null);
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
            h('div', { class: 'job-goal', text: job.goal || t('job.goal') }),
            newest ? h('div', { class: 'job-step', text: newest }) : null,
            h('div', { class: 'job-spend', text: t('job.spent', { amount: fmtMoney(job.spendNano) }) })),
          h('button', { class: 'btn small', type: 'button', 'data-act': 'stop', onclick: onStop, text: t('job.stop') }));
        jobBar.hidden = false;
      } else {
        jobBar.replaceChildren();
        jobBar.hidden = true;
      }

      cardsEl.replaceChildren(...(data.cards || []).map((c) => buildCard(botId, c, () => refresh())));
      v.rendered = true;
    };
    async function onStop() {
      if (!(await confirmSheet({ title: t('job.stop_title'), body: t('job.stop_body'), ok: t('job.stop_ok'), danger: true }))) return;
      try { await api('POST', `/api/bots/${enc(botId)}/stop`); await refresh(); } catch (e) { if (!isAuth(e)) toast(errMsg(e, 'err.generic')); }
    }
    v.el = el;
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
    v.el = h('section', { class: 'screen chat room' }, topbar(back, title), scroll, h('footer', { class: 'dock' }, composer.form));
    let first = true;
    v.update = (data) => {
      const room = data.room || {};
      title.replaceChildren(h('h1', { text: room.name || '' }), h('div', { class: 'sub', text: (room.bots || []).join(', ') }));
      const near = scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < 140;
      const ls = data.lines || [];
      if (!ls.length) lines.replaceChildren(h('p', { class: 'empty', text: t('room.empty') }));
      else {
        lines.replaceChildren(...ls.map((l) => {
          const person = l.from && l.from.kind === 'person';
          const bubble = h('div', { class: 'bubble' });
          if (l.from && l.from.name) bubble.append(h('div', { class: 'who', text: l.from.name }));
          if (l.text) bubble.append(h('div', { class: 'txt', text: l.text }));
          if (l.file) bubble.append(h('div', { class: 'attach', text: t('chat.file', { name: l.file.name || '' }) }));
          return h('div', { class: 'msg ' + (person ? 'mine' : 'theirs'), 'data-id': l.id, 'data-from': l.from && l.from.kind },
            bubble, h('div', { class: 'meta' }, h('span', { class: 'when', text: fmtTime(l.at, now()) })));
        }));
      }
      const runs = (data.work || []).filter((w) => w && w.run && (w.run.steps || []).length);
      stepsHost.replaceChildren(...runs.map((w) => stepsBlock(w.run.steps, w.run.endedMs == null, t('room.steps', { bot: w.bot, n: w.run.steps.length }), null)));
      const names = data.typing || [];
      typing.hidden = !names.length;
      typing.textContent = names.length ? t('room.typing', { names: names.join(', ') }) : '';
      if (first || near) scroll.scrollTop = scroll.scrollHeight;
      first = false;
    };
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
  on(win, 'online', onNetworkChange);
  on(win, 'offline', onNetworkChange);
  on(nav.connection, 'change', onNetworkChange);
  on(win, 'popstate', () => {
    if (!S.store || S.screen === 'unreachable') return;
    const r = parseRoute(win.location.hash);
    if (r.name === S.route.name && r.id === S.route.id) return;
    go(r, { push: false });
  });
  on(doc, 'visibilitychange', () => {
    const vis = doc.visibilityState !== 'hidden';
    S.visible = vis;
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
