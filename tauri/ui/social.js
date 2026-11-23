'use strict';

const SOCIAL_PREFIX = 'trident.social.v4:';

const social = {
  data: { chats: [], lastId: 0 },
  net: { friends: [], incoming: [], outgoing: [], error: '', ready: false },
  account: '',
  open: false,
  bigModal: null,
  tab: 'chats',
  query: '',
  bigQuery: '',
  chat: null,
  drafts: {},
  sig: '',
  busy: false,
  timer: null,
  el: {}
};

const directId = (name) => `u:${name.toLowerCase()}`;
const sameName = (a, b) => a.toLowerCase() === b.toLowerCase();
const findChat = (id) => social.data.chats.find((c) => c.id === id);
const lastMsg = (c) => c.messages[c.messages.length - 1] || null;
const chatTs = (c) => (lastMsg(c)?.ts || 0);
const validUser = (name) => /^[A-Za-z0-9_]{3,16}$/.test(name);
const friendOf = (name) => social.net.friends.find((f) => sameName(f.name, name)) || null;
const isIncoming = (name) => social.net.incoming.some((n) => sameName(n, name));
const isOutgoing = (name) => social.net.outgoing.some((n) => sameName(n, name));

function socialLoad(account) {
  try {
    const raw = JSON.parse(localStorage.getItem(SOCIAL_PREFIX + account.toLowerCase()));
    if (raw && Array.isArray(raw.chats)) {
      return {
        chats: raw.chats.filter((c) => c && !c.group && typeof c.name === 'string' && Array.isArray(c.messages)),
        lastId: Number(raw.lastId) || 0
      };
    }
  } catch {  }
  return { chats: [], lastId: 0 };
}

function socialSave() {
  if (!social.account) return;
  try { localStorage.setItem(SOCIAL_PREFIX + social.account.toLowerCase(), JSON.stringify(social.data)); } catch {  }
}

function shortAgo(ts) {
  if (!ts) return '';
  const s = Math.max(0, (Date.now() - ts) / 1000);
  if (s < 60) return 'Ahora';
  const m = Math.floor(s / 60);
  if (m < 60) return `Hace ${m}min`;
  const hr = Math.floor(m / 60);
  if (hr < 24) return `Hace ${hr}h`;
  return `Hace ${Math.floor(hr / 24)}d`;
}

const clockTime = (ts) => new Date(ts).toLocaleTimeString('es-AR', { hour: 'numeric', minute: '2-digit' });
const dayLabel = (ts) => new Date(ts).toLocaleDateString('es-AR', { day: 'numeric', month: 'long', year: 'numeric' });
const dayKey = (ts) => new Date(ts).toDateString();

function statusText(name) {
  const f = friendOf(name);
  if (!f) return isOutgoing(name) ? 'Solicitud enviada' : 'No es tu amigo';
  if (f.status === 'ingame') return f.server ? `Jugando en ${f.server}` : 'Jugando';
  if (f.status === 'online') return 'En línea';
  return f.lastSeen ? `Visto ${shortAgo(f.lastSeen).toLowerCase()}` : 'Desconectado';
}

function statusClass(name) {
  const f = friendOf(name);
  if (!f) return '';
  return f.status === 'ingame' ? ' game' : f.status === 'online' ? ' on' : '';
}

const headCache = new Map();

function skinToHead(dataUrl) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => {
      const c = document.createElement('canvas');
      c.width = c.height = 64;
      const g = c.getContext('2d');
      g.imageSmoothingEnabled = false;
      g.drawImage(img, 8, 8, 8, 8, 0, 0, 64, 64);
      if (img.height >= 16) g.drawImage(img, 40, 8, 8, 8, 0, 0, 64, 64);
      resolve(c.toDataURL('image/png'));
    };
    img.onerror = () => reject(new Error('skin ilegible'));
    img.src = dataUrl;
  });
}

async function fetchHead(name) {
  try {
    const r = await window.launcher.getSkin(name);
    if (r?.ok && r.data?.skin) return await skinToHead(r.data.skin);
  } catch {  }
  throw new Error('sin skin');
}

function loadHead(name) {
  const key = name.toLowerCase();
  if (!headCache.has(key)) {
    const p = fetchHead(name).catch(() => (key === 'mhf_steve' ? Promise.reject(new Error('sin skin')) : loadHead('MHF_Steve')));
    headCache.set(key, p);
    p.catch(() => headCache.delete(key));
  }
  return headCache.get(key);
}

function socialAvatar(name, size, noDot) {
  const wrap = h('span', { class: 'sa', style: { width: `${size}px`, height: `${size}px` } });
  const img = h('img', { alt: '', draggable: 'false' });
  loadHead(name).then((url) => { img.src = url; wrap.prepend(img); }).catch(() => {
    wrap.classList.add('fallback');
    wrap.prepend(name.slice(0, 1).toUpperCase());
  });
  if (!noDot) wrap.append(h('i', { class: `sa-dot${statusClass(name)}`, title: statusText(name) }));
  return wrap;
}

function keepFocus(render) {
  const a = document.activeElement;
  const key = a && a.dataset ? a.dataset.keep : null;
  const caret = key && typeof a.selectionStart === 'number' ? a.selectionStart : null;
  render();
  if (!key) return;
  const next = document.querySelector(`[data-keep="${key}"]`);
  if (!next) return;
  next.focus();
  if (caret !== null && typeof next.setSelectionRange === 'function') next.setSelectionRange(caret, caret);
}

function socialRefresh() {
  keepFocus(() => {
    socialRender();
    if (social.bigModal) socialRenderBig();
  });
}

function socialOpenChat(id) {
  social.chat = id;
  const c = findChat(id);
  if (c && c.unread) { c.unread = 0; socialSave(); }
  if (!social.bigModal) social.open = true;
  socialRefresh();
  socialSchedule(400);
}

function chatFor(name) {
  const id = directId(name);
  let c = findChat(id);
  if (!c) {
    const known = friendOf(name)?.name || name;
    c = { id, name: known, unread: 0, messages: [] };
    social.data.chats.push(c);
  }
  return c;
}

function socialStartDirect(name) {
  const c = chatFor(name);
  socialSave();
  socialOpenChat(c.id);
}

async function socialAct(action, name, okText) {
  const r = await window.launcher.socialAction(action, name);
  if (!r.ok) { toast(r.error, 'error'); return false; }
  if (okText) toast(typeof okText === 'function' ? okText(r.data) : okText);
  await socialPoll(true);
  return true;
}

function socialRemoveFriend(name) {
  confirmDialog({ title: 'Quitar amigo', text: `¿Quitar a ${name} de tu lista de amigos?`, okLabel: 'Quitar', danger: true })
    .then((ok) => { if (ok) socialAct('remove', name, `${name} ya no está en tu lista.`); });
}

async function socialSend(text) {
  const c = findChat(social.chat);
  const t = text.trim();
  if (!c || !t) return;
  const who = social.account || state?.account?.name || 'Yo';
  const msg = { from: who, mine: true, text: t, ts: Date.now(), pending: true };
  c.messages.push(msg);
  socialSave();
  socialRefresh();
  const r = await window.launcher.socialSend(c.name, t);
  delete msg.pending;
  if (!r.ok) {
    msg.failed = true;
    toast(r.error, 'error');
  }
  socialSave();
  socialRefresh();
}

function socialOpenAddFriend() {
  const input = h('input', { type: 'text', placeholder: 'Nombre de Minecraft', spellcheck: 'false', maxlength: '16' });
  const box = h('label', { class: 'sx-search' }, icon('search', 20), input);
  const out = h('div', { class: 'sm-center' }, h('span', { text: 'Escribí el nombre de un jugador que use Trident Client.' }));

  const draw = () => {
    const q = input.value.trim();
    out.textContent = '';
    if (!q) { out.className = 'sm-center'; out.append(h('span', { text: 'Escribí el nombre de un jugador que use Trident Client.' })); return; }
    if (!validUser(q)) { out.className = 'sm-center'; out.append(h('span', { text: 'Escribí un nombre de Minecraft válido: 3 a 16 letras, números o _.' })); return; }
    out.className = 'sm-results';
    const me = social.account && sameName(social.account, q);
    const friend = !!friendOf(q);
    const sent = isOutgoing(q);
    const incoming = isIncoming(q);
    const label = me ? 'Sos vos' : friend ? 'Ya es tu amigo' : sent ? 'Solicitud enviada' : incoming ? 'Aceptar solicitud' : 'Enviar solicitud';
    const add = h('button', { class: 'sm-add', type: 'button', disabled: me || friend || sent, text: label });
    add.addEventListener('click', async () => {
      add.disabled = true;
      const ok = incoming
        ? await socialAct('accept', q, `Ahora sos amigo de ${q}.`)
        : await socialAct('request', q, (d) => (d && d.accepted ? `Ahora sos amigo de ${d.name || q}.` : `Solicitud enviada a ${(d && d.name) || q}.`));
      if (ok) m.close(); else add.disabled = false;
    });
    out.append(h('div', { class: 'sr static' },
      socialAvatar(q, 44, true),
      h('span', { class: 'sr-main' }, h('span', { class: 'sr-name', text: q }), h('span', { class: 'sr-sub', text: friend ? statusText(q) : 'Jugador de Minecraft' })),
      add));
  };
  let timer = null;
  input.addEventListener('input', () => { clearTimeout(timer); timer = setTimeout(draw, 200); });
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter') { clearTimeout(timer); draw(); out.querySelector('.sm-add:not([disabled])')?.click(); } });

  const m = modal({ title: 'Agregar amigo', icon: 'userPlus', className: 'social-modal', body: [box, out] });
  setTimeout(() => input.focus(), 30);
}

function socialOpenNewChat() {
  const input = h('input', { type: 'text', placeholder: 'buscar amigos', spellcheck: 'false' });
  const box = h('label', { class: 'sx-search' }, icon('search', 20), input);
  const list = h('div', { class: 'sm-list' });
  const hint = h('p', { class: 'sm-hint', text: 'Elegí un amigo para empezar a chatear. Te puede contestar desde el launcher o desde el juego.' });
  const body = h('div', { class: 'sm-stack' }, box, list, hint);
  const m = modal({ title: 'Nuevo chat', icon: 'chat', className: 'social-modal tall', body: [body] });

  const draw = () => {
    const q = input.value.trim().toLowerCase();
    list.textContent = '';
    list.append(h('div', { class: 'sm-label', text: 'Amigos' }));
    const items = sortedFriends().filter((f) => !q || f.name.toLowerCase().includes(q));
    if (!items.length) {
      list.append(h('div', { class: 'sm-none', text: q ? 'Sin resultados.' : 'Todavía no tenés amigos. Agregá uno desde la pestaña Amigos.' }));
    }
    for (const f of items) {
      list.append(h('button', { class: 'sr', type: 'button', onclick: () => { m.close(); socialStartDirect(f.name); } },
        socialAvatar(f.name, 40),
        h('span', { class: 'sr-main' }, h('span', { class: 'sr-name', text: f.name }), h('span', { class: 'sr-sub', text: statusText(f.name) }))));
    }
  };
  input.addEventListener('input', draw);
  draw();
  setTimeout(() => input.focus(), 30);
}

function messageList(c) {
  const msgs = h('div', { class: 'sc-msgs' });
  if (!c.messages.length) msgs.append(h('div', { class: 'sc-note', text: `Empezá la charla con ${c.name}.` }));
  let prev = null;
  for (const m of c.messages) {
    if (!prev || dayKey(prev.ts) !== dayKey(m.ts)) msgs.append(h('div', { class: 'sc-day' }, h('span', { text: dayLabel(m.ts) })));
    const newGroup = !prev || prev.from !== m.from || dayKey(prev.ts) !== dayKey(m.ts) || m.ts - prev.ts > 5 * 60 * 1000;
    if (newGroup) {
      msgs.append(h('div', { class: 'sc-who' }, socialAvatar(m.from, 24, true), h('span', { class: 'sc-name', text: m.from }), h('span', { class: 'sc-clock', text: clockTime(m.ts) })));
    }
    const cls = `sc-bubble${m.pending ? ' pending' : ''}${m.failed ? ' failed' : ''}`;
    msgs.append(h('div', { class: 'sc-row' }, h('span', { class: cls, title: m.failed ? 'No se envió' : '', text: m.text })));
    prev = m;
  }
  setTimeout(() => { msgs.scrollTop = msgs.scrollHeight; }, 0);
  return msgs;
}

function composer(c, where) {
  const canWrite = !!friendOf(c.name);
  const input = h('input', {
    type: 'text', spellcheck: 'false', maxlength: '256', 'data-keep': `composer-${where}`,
    placeholder: canWrite ? `Mensaje a ${c.name}` : 'Solo podés escribirles a tus amigos', disabled: !canWrite
  });
  input.value = social.drafts[c.id] || '';
  input.addEventListener('input', () => { social.drafts[c.id] = input.value; });
  const submit = () => { const t = input.value; input.value = ''; social.drafts[c.id] = ''; socialSend(t); };
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter') submit(); });
  const send = h('button', { class: 'sc-send', type: 'button', title: 'Enviar', 'aria-label': 'Enviar', disabled: !canWrite, onclick: submit }, icon('arrowRight', 22));
  return h('div', { class: 'sc-composer' }, input, send);
}

function chatRow(c, active) {
  const m = lastMsg(c);
  return h('button', { class: `sr${active ? ' active' : ''}`, type: 'button', onclick: () => socialOpenChat(c.id) },
    socialAvatar(c.name, 44),
    h('span', { class: 'sr-main' },
      h('span', { class: 'sr-top' },
        h('span', { class: 'sr-name', text: c.name }),
        h('span', { class: 'sr-time', text: shortAgo(m?.ts) })),
      h('span', { class: 'sr-sub', text: m ? `${m.mine ? 'Tú' : m.from}: ${m.text}` : statusText(c.name) })),
    c.unread ? h('span', { class: 'sr-badge', text: String(c.unread) }) : null);
}

function friendRow(f) {
  return h('div', { class: 'sr static' },
    socialAvatar(f.name, 40),
    h('span', { class: 'sr-main' },
      h('span', { class: 'sr-name', text: f.name }),
      h('span', { class: 'sr-sub', text: statusText(f.name) })),
    h('button', { class: 'sr-act', type: 'button', title: 'Enviar mensaje', 'aria-label': `Enviar mensaje a ${f.name}`, onclick: () => socialStartDirect(f.name) }, icon('chat', 18)),
    h('button', { class: 'sr-act', type: 'button', title: 'Quitar amigo', 'aria-label': `Quitar a ${f.name}`, onclick: () => socialRemoveFriend(f.name) }, icon('trash', 18)));
}

function requestRow(name, incoming) {
  const acts = incoming
    ? [
      h('button', { class: 'sr-act ok', type: 'button', title: 'Aceptar', 'aria-label': `Aceptar a ${name}`, onclick: () => socialAct('accept', name, `Ahora sos amigo de ${name}.`) }, icon('check', 18)),
      h('button', { class: 'sr-act', type: 'button', title: 'Rechazar', 'aria-label': `Rechazar a ${name}`, onclick: () => socialAct('decline', name) }, icon('x', 18))
    ]
    : [h('button', { class: 'sr-act', type: 'button', title: 'Cancelar solicitud', 'aria-label': `Cancelar la solicitud a ${name}`, onclick: () => socialAct('cancel', name) }, icon('x', 18))];
  return h('div', { class: 'sr static' },
    socialAvatar(name, 40, true),
    h('span', { class: 'sr-main' },
      h('span', { class: 'sr-name', text: name }),
      h('span', { class: 'sr-sub', text: incoming ? 'Quiere ser tu amigo' : 'Solicitud enviada' })),
    ...acts);
}

const RANK = { ingame: 0, online: 1 };
const sortedFriends = () => [...social.net.friends].sort((a, b) => ((RANK[a.status] ?? 2) - (RANK[b.status] ?? 2)) || a.name.localeCompare(b.name));

const sortedChats = (q) => social.data.chats
  .filter((c) => !q || c.name.toLowerCase().includes(q.trim().toLowerCase()))
  .sort((a, b) => chatTs(b) - chatTs(a));

const unreadTotal = () => social.data.chats.reduce((n, c) => n + (c.unread || 0), 0);

function noticeBox(list) {
  const e = social.net.error;
  if (!e && social.net.ready) return false;
  list.append(h('div', { class: 'ss-empty' },
    h('strong', { text: e || 'Conectando con tus amigos…' }),
    e ? h('button', { class: 'ss-green', type: 'button', text: 'Reintentar', onclick: () => socialPoll(true) }) : null));
  return true;
}

function listView() {
  const isChats = social.tab === 'chats';
  const pending = social.net.incoming.length;

  const head = h('div', { class: 'sp-head' }, h('span', { class: 'sp-title', text: 'Sociales' }),
    h('span', { class: 'sp-ctl' },
      h('button', { class: 'sh-btn', type: 'button', title: 'Ampliar', 'aria-label': 'Ampliar', onclick: socialOpenBig }, icon('expand', 18)),
      h('button', { class: 'sh-btn', type: 'button', title: 'Minimizar', 'aria-label': 'Minimizar panel', onclick: () => { social.open = false; socialRender(); } }, icon('minus', 18))));

  const tabs = h('div', { class: 'st-tabs', role: 'tablist' },
    h('button', { class: `st-tab${isChats ? ' active' : ''}`, type: 'button', role: 'tab', 'aria-selected': String(isChats),
      onclick: () => { social.tab = 'chats'; social.query = ''; socialRender(); } },
      'Charlas', h('span', { class: 'st-count', text: String(unreadTotal() || social.data.chats.length) })),
    h('button', { class: `st-tab${!isChats ? ' active' : ''}`, type: 'button', role: 'tab', 'aria-selected': String(!isChats),
      onclick: () => { social.tab = 'friends'; social.query = ''; socialRender(); } },
      'Amigos', h('span', { class: `st-count${pending ? ' alert' : ''}`, text: String(pending ? `+${pending}` : social.net.friends.length) })));

  const input = h('input', { type: 'text', placeholder: isChats ? 'Buscar chats' : 'buscar amigos', spellcheck: 'false', value: social.query, 'data-keep': `search-${social.tab}` });
  input.addEventListener('input', () => { social.query = input.value; socialRenderList(); });
  const label = isChats ? 'Nuevo chat' : 'Agregar amigo';
  const search = h('div', { class: 'ss-row' },
    h('label', { class: 'ss-box' }, icon('search', 18), input),
    h('button', { class: 'ss-add', type: 'button', title: label, 'aria-label': label, onclick: isChats ? socialOpenNewChat : socialOpenAddFriend }, icon(isChats ? 'plus' : 'userPlus', 18)));

  return [head, tabs, search, h('div', { class: 'ss-list', id: 'social-list' })];
}

function socialRenderList() {
  const list = $('social-list');
  if (!list) return;
  list.textContent = '';
  const q = social.query.trim().toLowerCase();
  if (social.tab === 'chats') {
    const items = sortedChats(q);
    if (!items.length) {
      if (!q && noticeBox(list)) return;
      list.append(h('div', { class: 'ss-empty' },
        h('strong', { text: q ? 'Sin resultados' : 'Aún no hay charlas' }),
        q ? null : h('button', { class: 'ss-green', type: 'button', text: 'Nuevo chat', onclick: socialOpenNewChat })));
      return;
    }
    list.append(...items.map((c) => chatRow(c, false)));
    return;
  }
  if (noticeBox(list)) return;
  const match = (n) => !q || n.toLowerCase().includes(q);
  const inc = social.net.incoming.filter(match);
  const out = social.net.outgoing.filter(match);
  const friends = sortedFriends().filter((f) => match(f.name));
  if (inc.length) {
    list.append(h('div', { class: 'sm-label', text: `Solicitudes (${inc.length})` }));
    list.append(...inc.map((n) => requestRow(n, true)));
  }
  if (friends.length) {
    if (inc.length || out.length) list.append(h('div', { class: 'sm-label', text: `Amigos (${friends.length})` }));
    list.append(...friends.map(friendRow));
  }
  if (out.length) {
    list.append(h('div', { class: 'sm-label', text: 'Enviadas' }));
    list.append(...out.map((n) => requestRow(n, false)));
  }
  if (!inc.length && !friends.length && !out.length) {
    list.append(h('div', { class: 'ss-empty' },
      h('strong', { text: q ? 'Sin resultados' : 'Aún no hay amigos :(' }),
      q ? null : h('button', { class: 'ss-green', type: 'button', text: 'Agregar amigo', onclick: socialOpenAddFriend })));
  }
}

function miniChatView(c) {
  const head = h('div', { class: 'sc-head' },
    h('button', { class: 'sc-back', type: 'button', title: 'Volver', 'aria-label': 'Volver a la lista', onclick: () => { social.chat = null; socialRender(); } }, icon('back', 18)),
    socialAvatar(c.name, 40),
    h('span', { class: 'sc-title' }, h('span', { class: 'sr-name', text: c.name }), h('span', { class: 'sc-status', text: statusText(c.name) })),
    h('button', { class: 'sh-btn boxed', type: 'button', title: 'Ampliar', 'aria-label': 'Ampliar', onclick: socialOpenBig }, icon('expand', 18)),
    h('button', { class: 'sh-btn boxed', type: 'button', title: 'Cerrar', 'aria-label': 'Cerrar panel', onclick: () => { social.open = false; socialRender(); } }, icon('x', 18)));
  return [head, messageList(c), composer(c, 'mini')];
}

function socialRenderBar() {
  const badge = social.el.badge;
  if (!badge) return;
  const n = unreadTotal() + social.net.incoming.length;
  badge.hidden = !n;
  badge.textContent = n > 99 ? '99+' : String(n);
}

function socialRender() {
  const { root, panel, body } = social.el;
  if (!root) return;
  socialRenderBar();
  const showPanel = social.open && !social.bigModal;
  root.classList.toggle('open', showPanel);
  panel.hidden = !showPanel;
  body.textContent = '';
  if (!showPanel) return;
  const c = social.chat && findChat(social.chat);
  if (!c) social.chat = null;
  body.append(...(c ? miniChatView(c) : listView()));
  if (!c) socialRenderList();
}

function socialOpenBig() {
  if (social.bigModal) return;
  social.open = false;
  const m = modal({
    title: 'Charlas', icon: 'chat', className: 'social-big', body: [],
    onClose: () => { social.bigModal = null; social.bigQuery = ''; socialRender(); }
  });
  m.el.querySelector('.modal-x').before(h('button', {
    class: 'modal-x', type: 'button', title: 'Reducir', 'aria-label': 'Reducir',
    onclick: () => { m.close(); social.open = true; socialRender(); }
  }, icon('shrink', 20)));
  social.bigModal = m;
  socialRender();
  socialRenderBig();
  socialSchedule(400);
}

function socialToggleBig() {
  if (social.bigModal) social.bigModal.close();
  else socialOpenBig();
}

function socialFillBigList() {
  const list = $('social-big-list');
  if (!list) return;
  list.textContent = '';
  const items = sortedChats(social.bigQuery);
  if (!items.length) {
    if (!social.bigQuery.trim() && noticeBox(list)) return;
    list.append(h('div', { class: 'ss-empty' },
      h('strong', { text: social.bigQuery.trim() ? 'Sin resultados' : 'Aún no hay charlas' }),
      social.bigQuery.trim() ? null : h('button', { class: 'ss-green', type: 'button', text: 'Nuevo chat', onclick: socialOpenNewChat })));
    return;
  }
  list.append(...items.map((c) => chatRow(c, c.id === social.chat)));
}

function socialRenderBig() {
  const m = social.bigModal;
  if (!m) return;
  const input = h('input', { type: 'text', placeholder: 'Buscar chats', spellcheck: 'false', value: social.bigQuery, 'data-keep': 'search-big' });
  input.addEventListener('input', () => { social.bigQuery = input.value; socialFillBigList(); });
  const left = h('div', { class: 'sx-left' },
    h('div', { class: 'sx-bar' },
      h('label', { class: 'sx-search' }, icon('search', 18), input),
      h('button', { class: 'sx-plus', type: 'button', title: 'Nuevo chat', 'aria-label': 'Nuevo chat', onclick: socialOpenNewChat }, icon('plus', 20))),
    h('div', { class: 'sx-list', id: 'social-big-list' }));

  const c = social.chat && findChat(social.chat);
  if (!c) social.chat = null;
  const right = h('div', { class: 'sx-right' });
  if (c) {
    right.append(
      h('div', { class: 'sx-chat-head' }, socialAvatar(c.name, 44), h('span', { class: 'sc-title' }, h('span', { class: 'sr-name', text: c.name }), h('span', { class: 'sc-status', text: statusText(c.name) }))),
      messageList(c), composer(c, 'big'));
  } else {
    right.append(h('div', { class: 'sx-blank' }, icon('chat', 42), h('strong', { text: 'Elegí una charla' }), h('span', { text: 'o empezá una nueva con el botón +' })));
  }
  m.body.textContent = '';
  m.body.append(h('div', { class: 'sx' }, left, right));
  socialFillBigList();
}

function socialSwitchAccount(name) {
  social.account = name;
  social.data = name ? socialLoad(name) : { chats: [], lastId: 0 };
  social.net = { friends: [], incoming: [], outgoing: [], error: '', ready: false };
  social.chat = null;
  social.sig = '';
}

function viewing(c) {
  return c.id === social.chat && (social.bigModal || social.open) && !document.hidden;
}

async function socialPoll(force) {
  if (!window.launcher || typeof window.launcher.socialSync !== 'function') return;
  if (social.busy && !force) return;
  const account = state?.account?.name || '';
  if (account !== social.account) socialSwitchAccount(account);
  if (!account) {
    social.net.error = 'Iniciá sesión para usar amigos.';
    socialRefresh();
    return;
  }
  social.busy = true;
  try {
    const r = await window.launcher.socialSync(social.data.lastId || 0);
    if ((state?.account?.name || '') !== account) return;
    if (!r.ok) {
      const changed = social.net.error !== r.error;
      social.net.error = r.error;
      if (changed) socialRefresh();
      return;
    }
    const d = r.data || {};
    if (d.wallet && typeof d.wallet === 'object') {
      window.tridentWallet = d.wallet;
      window.dispatchEvent(new CustomEvent('trident-wallet', { detail: d.wallet }));
    }
    const before = new Set(social.net.incoming.map((n) => n.toLowerCase()));
    const wasReady = social.net.ready;
    social.net = {
      friends: Array.isArray(d.friends) ? d.friends : [],
      incoming: Array.isArray(d.incoming) ? d.incoming : [],
      outgoing: Array.isArray(d.outgoing) ? d.outgoing : [],
      error: '',
      ready: true
    };
    if (wasReady) {
      for (const n of social.net.incoming) if (!before.has(n.toLowerCase())) toast(`${n} te mandó una solicitud de amistad.`);
    }
    let gotMessages = false;
    for (const m of Array.isArray(d.messages) ? d.messages : []) {
      if (!m || typeof m.from !== 'string' || typeof m.text !== 'string') continue;
      const id = Number(m.id) || 0;
      if (id <= social.data.lastId) continue;
      social.data.lastId = id;
      const c = chatFor(m.from);
      c.messages.push({ from: m.from, mine: false, text: m.text, ts: Number(m.ts) || Date.now() });
      if (c.messages.length > 500) c.messages.splice(0, c.messages.length - 500);
      if (!viewing(c)) {
        c.unread = (c.unread || 0) + 1;
        toast(`${m.from}: ${m.text.length > 60 ? `${m.text.slice(0, 57)}…` : m.text}`);
      }
      gotMessages = true;
    }
    if (gotMessages) socialSave();
    const sig = JSON.stringify([social.net, social.data.lastId]);
    if (sig !== social.sig || gotMessages || !wasReady) {
      social.sig = sig;
      socialRefresh();
    }
  } finally {
    social.busy = false;
  }
}

function socialSchedule(ms) {
  clearTimeout(social.timer);
  social.timer = setTimeout(async () => {
    await socialPoll(false);
    socialSchedule(document.hidden ? 30000 : (social.open || social.bigModal ? 4000 : 12000));
  }, ms);
}

function mountSocial() {
  const root = $('social');
  if (!root || social.el.root) return;

  const body = h('div', { class: 'sp-body' });
  const panel = h('section', { class: 'sp', 'aria-label': 'Sociales' }, body);
  panel.hidden = true;

  const badge = h('span', { class: 'sbar-badge', hidden: true });
  const bar = h('div', { class: 'sbar' },
    h('button', { class: 'sbar-main', type: 'button', 'aria-label': 'Abrir charla', onclick: () => {
      if (social.bigModal) return;
      social.open = !social.open;
      socialRender();
      if (social.open) socialSchedule(200);
    } }, icon('chat', 22), h('span', { text: 'Charla' }), badge),
    h('button', { class: 'sbar-add', type: 'button', title: 'Agregar amigo', 'aria-label': 'Agregar amigo', onclick: socialOpenAddFriend }, icon('userPlus', 20)));

  root.append(panel, bar);
  Object.assign(social.el, { root, panel, body, badge });
  socialRender();
  document.addEventListener('visibilitychange', () => { if (!document.hidden) socialSchedule(300); });
  socialSchedule(1500);
}
