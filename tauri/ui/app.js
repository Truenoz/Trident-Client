'use strict';

const els = {
  views: {
    home: $('view-home'),
    profiles: $('view-profiles'),
    detail: $('view-detail'),
    browse: $('view-browse'),
    cosmetics: $('view-cosmetics'),
    console: $('view-console')
  },

  accountOut: $('account-out'),
  accountIn: $('account-in'),
  accountName: $('account-name'),
  avatar: $('avatar'),
  btnLogout: $('btn-logout'),
  loginBtns: document.querySelectorAll('.js-login'),

  hero: document.querySelector('.hero'),
  heroKicker: $('hero-kicker'),
  heroTitle: $('hero-title'),
  heroSub: $('hero-sub'),
  skin: $('skin'),
  skin3d: $('skin3d'),
  nametag: $('nametag'),
  heroPillText: $('hero-pill-text'),
  heroLogin: $('hero-login'),

  launchLabel: $('launch-label'),
  launchFill: $('launch-fill'),
  profilePick: $('btn-profile-pick'),
  profileOptions: $('btn-profile-options'),
  homeMods: $('home-mods'),
  modsTitle: $('mods-title'),
  modsCount: $('mods-count'),
  newsDots: $('news-dots'),
  accountKind: $('account-kind'),
  progressText: $('progress-text'),
  progressBar: $('progress-bar'),
  homeProgress: $('home-progress'),
  playError: $('play-error'),
  statusPill: $('status-pill'),
  btnManageProfiles: $('btn-manage-profiles'),
  btnAddMods: $('btn-add-mods'),
  newsList: $('news-list'),
  infoRam: $('info-ram'),
  infoJava: $('info-java'),
  infoCount: $('info-count'),

  profileTabs: $('profile-tabs'),
  profileSearch: $('profile-search'),
  profileGroups: $('profile-groups'),

  log: $('log'),
  btnClearLog: $('btn-clear-log'),

  updatePill: $('update-pill'),
  railPlay: $('rail-play')
};

const state = {
  account: null,
  settings: null,
  info: null,
  versions: [],
  versionsLoaded: false,
  profiles: [],
  folders: [],
  busy: false,
  running: false,
  activeId: null,
  view: 'home',
  profileTab: 'all',
  collapsed: new Set(),
  notifications: [],
  unread: 0
};

function mountStaticIcons() {
  $('ico-palette').replaceWith(icon('palette', 18));
  $('ico-bell').replaceWith(icon('bell', 18));
  $('ico-chat').replaceWith(icon('chat', 18));
  $('rail-settings').append(icon('settings', 18));
  $('ico-search-profiles').replaceWith(icon('search'));

  els.profilePick.append(icon('chevDown', 20));
  els.profileOptions.append(icon('settings', 18));
  els.btnManageProfiles.append(icon('grid', 16), 'Perfiles');
  els.btnAddMods.append(icon('plus', 15), 'Agregar');
  $('btn-new-folder').append(icon('folder'), 'Nueva carpeta');
  $('btn-import').append(icon('import'), 'Importar');
  $('btn-new-profile').append(icon('plus'), 'Nuevo perfil');
}

const TABS = [
  { view: 'home', icon: 'play', title: 'Jugar' },
  { view: 'profiles', icon: 'grid', title: 'Perfiles' },
  { view: 'browse', icon: 'cube', title: 'Mods' },
  { view: 'cosmetics', icon: 'sparkle', title: 'Cosméticos' },
  { view: 'console', icon: 'console', title: 'Registro' }
];

function mountRail() {
  const nav = $('rail-nav');
  for (const t of TABS) {
    nav.append(h('button', {
      class: 'tab', type: 'button', dataset: { view: t.view },
      onclick: () => goTo(t.view)
    }, icon(t.icon, 17), h('span', { text: t.title })));
  }

  $('btn-chat').addEventListener('click', () => { if (typeof socialToggleBig === 'function') socialToggleBig(); });
}

function showError(message) {
  els.playError.textContent = message || '';
  els.playError.classList.toggle('hidden', !message);
}

function setProgress(text, percent) {
  els.progressText.textContent = text;
  const bar = els.progressBar;
  if (percent === null || percent === undefined) {
    bar.classList.toggle('indeterminate', state.busy && !state.running);
    bar.style.width = state.busy && !state.running ? '' : '0%';
  } else {
    bar.classList.remove('indeterminate');
    bar.style.width = `${Math.max(0, Math.min(100, percent))}%`;
  }
  state.percent = percent;
  renderLaunch();
  if (typeof detailProgress === 'function') detailProgress(text, percent);
}

function profileById(id) {
  return state.profiles.find((p) => p.id === id) || null;
}

function lastProfile() {
  return profileById(state.settings?.lastProfile) || recentProfiles()[0] || state.profiles[0] || null;
}

function recentProfiles() {
  return [...state.profiles].sort((a, b) => (b.lastPlayed || 0) - (a.lastPlayed || 0) || b.created - a.created);
}

function notify(text) {
  state.notifications.unshift({ text, time: Date.now() });
  state.notifications.length = Math.min(state.notifications.length, 30);
  state.unread++;
  $('bell-dot').classList.remove('hidden');
}

const logQueue = { lines: [], timer: 0, size: 0 };
function appendLog(line) {
  logQueue.lines.push(line.endsWith('\n') ? line : `${line}\n`);
  if (logQueue.lines.length > 4000) logQueue.lines.splice(0, logQueue.lines.length - 3000);
  if (!logQueue.timer) logQueue.timer = setTimeout(flushLog, 250);
}
function flushLog() {
  logQueue.timer = 0;
  if (!logQueue.lines.length) return;
  if (state.view !== 'console' || document.hidden) { logQueue.timer = setTimeout(flushLog, 1000); return; }
  const atBottom = els.log.scrollTop + els.log.clientHeight >= els.log.scrollHeight - 30;
  const chunk = logQueue.lines.join('');
  logQueue.lines = [];
  els.log.append(chunk);
  logQueue.size += chunk.length;
  if (logQueue.size > 200000) {
    els.log.textContent = els.log.textContent.slice(-150000);
    logQueue.size = els.log.textContent.length;
  }
  if (atBottom) els.log.scrollTop = els.log.scrollHeight;
}

function goTo(view, opts) {
  closeMenu();
  state.view = view;
  const railView = view === 'detail' ? 'profiles' : view;
  document.querySelectorAll('.tab[data-view]').forEach((b) => b.classList.toggle('active', b.dataset.view === railView));
  $('rail-settings').classList.remove('active');
  Object.entries(els.views).forEach(([name, el]) => el.classList.toggle('active', name === view));
  const content = document.querySelector('main.content');
  if (content && state.lastView !== view) content.scrollTop = 0;
  state.lastView = view;
  if (view === 'profiles') renderProfiles();
  if (view === 'detail') renderDetail(opts.id, opts.tab);
  if (view === 'browse') renderBrowse();
  if (view === 'cosmetics' && typeof renderCosmetics === 'function') renderCosmetics();
  if (view === 'console') flushLog();
  if (view === 'home') { homeMods.stale = true; renderHome(); }
}

$('btn-min').addEventListener('click', () => window.launcher.minimize());
$('btn-close').addEventListener('click', () => window.launcher.close());
$('btn-max').addEventListener('click', () => window.launcher.maximize());
window.launcher.onMaximized((max) => {
  $('ico-max').classList.toggle('hidden', max);
  $('ico-restore').classList.toggle('hidden', !max);
  $('btn-max').title = max ? 'Restaurar' : 'Maximizar';
});

$('rail-settings').addEventListener('click', () => openSettings());
$('btn-palette').addEventListener('click', () => openSettings('appearance'));
$('btn-bell').addEventListener('click', (e) => {
  state.unread = 0;
  $('bell-dot').classList.add('hidden');
  const items = state.notifications.length
    ? state.notifications.map((n) => ({ label: `${new Date(n.time).toLocaleTimeString('es-AR', { hour: '2-digit', minute: '2-digit' })} · ${n.text}`, icon: 'info', onclick() {} }))
    : [{ label: 'Sin notificaciones', icon: 'check', onclick() {} }];
  popMenu(e.currentTarget, items, h('div', { class: 'pop-head' }, h('strong', { text: 'Notificaciones' })));
});

els.btnManageProfiles.addEventListener('click', () => goTo('profiles'));
els.profileOptions.addEventListener('click', () => {
  const p = lastProfile();
  if (p) openProfileMenu(els.profileOptions, p);
  else openProfileWizard();
});
els.profilePick.addEventListener('click', () => openProfilePicker());
els.btnAddMods.addEventListener('click', () => {
  const p = lastProfile();
  if (p) goTo('detail', { id: p.id, tab: 'content' });
  else goTo('profiles');
});
$('btn-go-settings').addEventListener('click', () => openSettings());
$('btn-news-all').addEventListener('click', () => {
  call(window.launcher.openExternal('https://www.minecraft.net/es-es/articles')).catch((e) => toast(e.message, 'error'));
});

const STEVE = 'https://mc-heads.net/body/MHF_Steve/320';

let viewer3d = null;
try { viewer3d = els.skin3d && window.Skin3D && Skin3D.supported() ? Skin3D.create(els.skin3d, els.hero) : null; } catch { viewer3d = null; }
if (viewer3d) viewer3d.onLost = () => showFlatSkin(state.account);
let skinToken = 0;

function showFlatSkin(account) {
  els.skin3d.classList.add('hidden');
  els.skin.classList.remove('hidden');
  els.skin.classList.toggle('dim', !account);

  els.skin.onerror = () => { els.skin.onerror = () => els.skin.classList.add('hidden'); els.skin.src = STEVE; };
  els.skin.src = account ? `https://mc-heads.net/body/${encodeURIComponent(account.uuid)}/320` : STEVE;
}

async function showSkin(account) {
  const token = ++skinToken;
  els.skin3d.classList.toggle('dim', !account);
  if (!viewer3d) { showFlatSkin(account); return; }
  try {
    const data = await call(account ? window.launcher.getMySkin() : window.launcher.getSkin('MHF_Steve'));
    if (token !== skinToken) return;
    await viewer3d.setSkin(data.skin, data.model);
    if (token !== skinToken) return;
    els.skin.classList.add('hidden');
    els.skin3d.classList.remove('hidden');

    if (account) {
      const face = viewer3d.faceDataURL(64);
      if (face) { els.avatar.onerror = null; els.avatar.src = face; }
    }
  } catch (e) {
    console.warn('[Skin3D]', e);
    if (token === skinToken) showFlatSkin(account);
  }
}

function renderAccount(account) {
  state.account = account;
  els.accountOut.classList.toggle('hidden', !!account);
  els.accountIn.classList.toggle('hidden', !account);
  els.heroLogin.classList.toggle('hidden', !!account);
  $('hero-pill').classList.toggle('hidden', !!account);

  if (account) {
    const id = encodeURIComponent(account.uuid);
    els.accountName.textContent = account.name;
    els.avatar.onerror = () => { els.avatar.onerror = null; els.avatar.src = 'logo.png'; };
    els.avatar.src = `https://mc-heads.net/avatar/${id}/64`;
    els.nametag.textContent = account.name;
    els.heroPillText.textContent = account.offline ? 'Sesión sin cuenta (offline)' : 'Sesión iniciada';
    els.accountKind.textContent = account.offline ? 'Sin cuenta (offline)' : 'Cuenta de Microsoft';
    $('btn-offline-skin').classList.toggle('hidden', !account.offline);
  } else {
    els.nametag.textContent = 'Sin sesión';
    els.heroPillText.textContent = 'Conectá tu cuenta para jugar';
  }
  showSkin(account);
  renderAll();
}

async function doLogin() {
  showError('');
  els.loginBtns.forEach((b) => { b.disabled = true; b.textContent = 'Esperando…'; });
  try {
    renderAccount(await call(window.launcher.login()));
    return true;
  } catch (e) {
    showError(friendlyAuthError(e.message));
    return false;
  } finally {
    els.loginBtns.forEach((b) => {
      b.disabled = false;
      b.textContent = b.id === 'hero-login' ? 'Iniciar sesión' : 'Iniciar sesión con Microsoft';
    });
  }
}

els.loginBtns.forEach((b) => b.addEventListener('click', doLogin));

async function doLogout() {
  await call(window.launcher.logout());
  renderAccount(null);
}
els.btnLogout.addEventListener('click', doLogout);

function skinIsSlim(img) {
  if (img.naturalHeight !== 64) return false;
  try {
    const c = document.createElement('canvas');
    c.width = 64; c.height = 64;
    const x = c.getContext('2d');
    x.drawImage(img, 0, 0);
    return x.getImageData(55, 20, 1, 12).data.every((v, i) => i % 4 !== 3 || v === 0);
  } catch { return false; }
}

async function refreshMySkin() {
  if (typeof cos !== 'undefined') cos.skinFor = null;
  await showSkin(state.account);
  if (state.view === 'cosmetics' && typeof renderCosmetics === 'function') renderCosmetics();
}

$('offline-skin-file').addEventListener('change', (e) => {
  const file = e.target.files && e.target.files[0];
  e.target.value = '';
  if (!file) return;
  if (file.size > 150000) { toast('Esa imagen es demasiado grande para ser una skin.', 'error'); return; }
  const reader = new FileReader();
  reader.onload = () => {
    const url = String(reader.result || '');
    const img = new Image();
    img.onload = async () => {
      if (img.naturalWidth !== 64 || (img.naturalHeight !== 64 && img.naturalHeight !== 32)) {
        toast(`La skin tiene que medir 64×64 o 64×32 píxeles (esta mide ${img.naturalWidth}×${img.naturalHeight}).`, 'error');
        return;
      }
      try {
        await call(window.launcher.setOfflineSkin(url, skinIsSlim(img) ? 'slim' : 'classic'));
        toast('Skin cambiada. Se ve en el launcher de esta PC.', 'success');
        await refreshMySkin();
      } catch (err) {
        toast((err && err.message) || 'No se pudo guardar la skin.', 'error');
      }
    };
    img.onerror = () => toast('No se pudo leer la imagen. Tiene que ser un PNG.', 'error');
    img.src = url;
  };
  reader.readAsDataURL(file);
});

$('btn-offline-skin').addEventListener('click', (e) => {
  popMenu(e.currentTarget, [
    { label: 'Subir skin (PNG)…', icon: 'import', onclick: () => $('offline-skin-file').click() },
    { label: 'Quitar mi skin', icon: 'trash', onclick: async () => {
      try {
        await call(window.launcher.clearOfflineSkin());
        toast('Volviste a la skin por defecto.', 'success');
        await refreshMySkin();
      } catch (err) { toast((err && err.message) || 'No se pudo quitar la skin.', 'error'); }
    } }
  ], h('div', { class: 'pop-head' }, h('strong', { text: 'Skin de cuenta no premium' }), h('span', { class: 'muted small', text: 'Solo se ve en este launcher, no en el juego.' })));
});

function friendlyAuthError(message) {
  const m = (message || '').toLowerCase();
  if (m.includes('closed') || m.includes('cancel')) return 'Se cerró la ventana de inicio de sesión.';
  if (m.includes('game') || m.includes('entitle') || m.includes('profile') || m.includes('owns')) {
    return 'Esta cuenta de Microsoft no parece tener Minecraft: Java Edition.';
  }
  return `No se pudo iniciar sesión: ${message}`;
}

function canPlay() {
  return !!state.account && !state.busy && !state.running;
}

function renderAll() {
  renderLaunch();
  renderStatusPill();
  if (state.view === 'home') renderHome();
  if (state.view === 'profiles') renderProfiles();
  if (state.view === 'detail' && typeof detailRefresh === 'function') detailRefresh();
}

function launchMode() {
  const p = lastProfile();
  if (state.running && (!state.activeId || !p || state.activeId === p.id)) return 'stop';
  if (state.running) return 'running-other';
  if (state.busy) return 'busy';
  if (!state.account) return 'login';
  if (!p) return 'create';
  return 'play';
}

function renderLaunch() {
  const mode = launchMode();
  const btn = els.railPlay;
  btn.className = `launch-btn mode-${mode}`;
  btn.disabled = mode === 'running-other';
  const pct = typeof state.percent === 'number' ? Math.max(0, Math.min(100, state.percent)) : null;
  els.launchFill.style.width = mode === 'busy' && pct !== null ? `${pct}%` : '';
  btn.classList.toggle('indeterminate', mode === 'busy' && pct === null);
  els.launchLabel.textContent = {
    stop: 'DETENER',
    'running-other': 'OTRO PERFIL EN JUEGO',
    busy: pct !== null && pct > 0 ? `PREPARANDO… ${Math.round(pct)}%` : 'PREPARANDO…',
    login: 'INICIAR SESIÓN PARA JUGAR',
    create: 'CREAR UN PERFIL',
    play: 'JUGAR'
  }[mode];
  els.profilePick.disabled = state.busy;
}

function renderStatusPill() {
  els.statusPill.textContent = state.running ? '1 instancia en ejecución' : state.busy ? 'Preparando instancia…' : 'Ninguna instancia en ejecución';
  els.statusPill.classList.toggle('on', state.running);
}

function renderHome() {
  const p = lastProfile();
  if (p) {
    els.heroKicker.textContent = p.name;
    els.heroTitle.textContent = `Minecraft ${p.mc}`;
    const parts = [p.loader === 'fabric' && p.nova !== false ? `${p.loaderLabel} + Trident Client` : p.loaderLabel];
    parts.push(state.activeId === p.id && state.running ? 'Jugando ahora' : timeAgo(p.lastPlayed));
    els.heroSub.textContent = parts.join(' · ');
  } else {
    els.heroKicker.textContent = 'Bienvenido';
    els.heroTitle.textContent = 'Trident Client';
    els.heroSub.textContent = 'Creá tu primer perfil para empezar a jugar';
  }
  els.profileOptions.title = p ? `Opciones de ${p.name}` : 'Nuevo perfil';
  els.infoCount.textContent = String(state.profiles.length);
  els.homeProgress.classList.toggle('idle', !state.busy && !state.running);
  renderStatusPill();
  renderLaunch();
  loadHomeMods();
}

function openProfilePicker() {
  const current = lastProfile();
  const items = recentProfiles().slice(0, 8).map((p) => ({
    label: `${p.name} · ${p.loaderLabel} ${p.mc}`,
    icon: current && p.id === current.id ? 'check' : 'cube',
    onclick: () => selectProfile(p.id)
  }));
  if (items.length) items.push('sep');
  items.push({ label: 'Nuevo perfil…', icon: 'plus', onclick: () => openProfileWizard() });
  items.push({ label: 'Administrar perfiles', icon: 'grid', onclick: () => goTo('profiles') });
  popMenu(els.profilePick, items, h('div', { class: 'pop-head' }, h('strong', { text: 'Elegí un perfil' })));
}

async function selectProfile(id) {
  if (state.settings?.lastProfile === id) return;
  await saveSetting({ lastProfile: id });
  renderAll();
}

const homeMods = { id: null, list: null, loading: false, stale: true };

async function loadHomeMods(force) {
  const p = lastProfile();
  if (!p) { homeMods.id = null; homeMods.list = []; drawHomeMods(); return; }
  if (!force && !homeMods.stale && homeMods.id === p.id && homeMods.list) { drawHomeMods(); return; }
  if (homeMods.loading && homeMods.id === p.id) return;
  homeMods.id = p.id;
  homeMods.loading = true;
  homeMods.stale = false;
  if (!homeMods.list) drawHomeMods();
  try {
    const list = await call(window.launcher.listContent(p.id));
    if (homeMods.id !== p.id) return;
    homeMods.list = list;
  } catch {
    if (homeMods.id === p.id) homeMods.list = [];
  } finally {
    if (homeMods.id === p.id) homeMods.loading = false;
  }
  drawHomeMods();
}

function modStatus(c) {
  if (c.compat?.status === 'bad') return { cls: 'bad', text: 'Incompatible' };
  return c.enabled ? { cls: 'on', text: 'Activo' } : { cls: 'off', text: 'Desactivado' };
}

function drawHomeMods() {
  const p = lastProfile();
  const box = els.homeMods;
  box.textContent = '';
  els.modsTitle.textContent = p ? `Mods · ${p.name}` : 'Mods instalados';
  if (!p) {
    els.modsCount.textContent = '0';
    box.append(h('div', { class: 'mods-empty muted small', text: 'Cuando crees un perfil, sus mods aparecen acá.' }));
    return;
  }
  if (homeMods.list === null || (homeMods.loading && homeMods.id !== p.id)) {
    box.append(h('div', { class: 'mods-empty muted small', text: 'Cargando mods…' }));
    return;
  }
  const playing = state.activeId === p.id && (state.busy || state.running);
  const items = [...homeMods.list].sort((a, b) => (a.type === b.type ? 0 : a.type === 'mod' ? -1 : 1) || (b.enabled - a.enabled) || a.name.localeCompare(b.name));
  const mods = items.filter((c) => c.type === 'mod');
  els.modsCount.textContent = String(mods.length + (p.loader === 'fabric' && p.nova !== false ? 1 : 0));

  if (p.loader === 'fabric' && p.nova !== false) {
    box.append(h('div', { class: 'mod-row builtin', title: 'Viene con el perfil y se actualiza solo' },
      h('img', { class: 'mod-ico pixel', src: 'logo.png', alt: '', width: 34, height: 34 }),
      h('div', { class: 'mod-info' }, h('strong', { text: 'Trident Client' }), h('span', { class: 'muted small', text: 'Integrado' })),
      h('span', { class: 'mod-state on', text: 'Activo' })));
  }
  if (!items.length) {
    box.append(h('div', { class: 'mods-empty muted small' },
      p.loader === 'fabric' ? 'Este perfil todavía no tiene mods extra.' : 'Perfil Vanilla: Minecraft sin mods.',
      p.loader === 'fabric' ? h('button', { class: 'link-btn', type: 'button', text: 'Buscar mods →', onclick: () => goTo('detail', { id: p.id, tab: 'content' }) }) : null));
    return;
  }
  for (const c of items) {
    const st = modStatus(c);
    const sw = toggle(c.enabled, async (v, input) => {
      try {
        await call(window.launcher.toggleContent(p.id, c.file, v));
        c.enabled = v;
        c.file = v ? c.base : `${c.base}.disabled`;
        drawHomeMods();
      } catch (e) { input.checked = !v; toast(e.message, 'error'); }
    }, `Activar ${c.name}`);
    const input = sw.querySelector('input');
    if (input && playing) { input.disabled = true; sw.title = 'No se puede cambiar mientras el perfil está en juego'; }
    const ico = c.icon
      ? h('img', { class: 'mod-ico', src: c.icon, alt: '', width: 34, height: 34, loading: 'lazy' })
      : h('span', { class: 'mod-ico ph' }, icon(c.type === 'mod' ? 'cube' : 'file', 18));
    const row = h('div', { class: `mod-row${c.enabled ? '' : ' off'}` },
      ico,
      h('div', { class: 'mod-info' },
        h('strong', { text: c.name }),
        h('span', { class: 'muted small', text: [c.type === 'mod' ? null : 'Paquete de recursos', c.version || null].filter(Boolean).join(' · ') || c.source })),
      h('span', { class: `mod-state ${st.cls}`, text: st.text }),
      sw);
    sw.addEventListener('click', (e) => e.stopPropagation());
    row.addEventListener('click', () => goTo('detail', { id: p.id, tab: 'content' }));
    box.append(row);
  }
}

async function playProfile(id, opts) {
  if (state.busy || state.running) return;
  const p = profileById(id);
  if (!p) return;
  showError('');
  if (!state.account) {
    toast('Iniciá sesión con tu cuenta de Microsoft para jugar.');
    return;
  }
  state.busy = true;
  state.activeId = id;
  renderAll();
  setProgress('Preparando…', null);
  try {
    await call(window.launcher.launch(id, opts));
  } catch (e) {
    state.busy = false;
    state.running = false;
    state.activeId = null;
    setProgress('Todo listo.', 0);
    showError(e.message);
    notify(`No se pudo iniciar: ${e.message}`);
    renderAll();
  }
}

els.railPlay.addEventListener('click', () => {
  const mode = launchMode();
  if (mode === 'stop') { window.launcher.stop(); return; }
  if (mode === 'login') { doLogin(); return; }
  if (mode === 'create') { openProfileWizard(); return; }
  const p = lastProfile();
  if (mode === 'play' && p) playProfile(p.id);
});

window.launcher.onStatus(async ({ state: s, text, percent, profileId }) => {
  const before = `${state.busy}|${state.running}|${state.activeId}`;
  if (s === 'idle') {
    state.busy = false;
    state.running = false;
    state.activeId = null;
    setProgress(text || 'Todo listo.', 0);
    if (text) notify(text);

    await loadProfiles();
  } else if (s === 'running') {
    state.busy = false;
    state.running = true;
    state.activeId = profileId || state.activeId;
    setProgress(text, 100);
  } else {
    state.busy = true;
    state.running = false;
    state.activeId = profileId || state.activeId;
    setProgress(text, percent);
  }

  document.body.classList.toggle('game-running', state.running);
  if (before !== `${state.busy}|${state.running}|${state.activeId}`) renderAll();
});

window.launcher.onLog(appendLog);
els.btnClearLog.addEventListener('click', () => { els.log.textContent = ''; logQueue.lines = []; logQueue.size = 0; });

window.launcher.onProfileProgress(({ text }) => {
  if (text) toast(text);
});

async function loadProfiles() {
  const data = await call(window.launcher.listProfiles());
  state.profiles = data.profiles;
  state.folders = data.folders;
  homeMods.stale = true;
  if (state.settings && typeof state.settings === 'object') {

    try { state.settings = await call(window.launcher.getSettings()); } catch {  }
  }
  els.infoCount.textContent = String(state.profiles.length);
  renderAll();
}

const PROFILE_TABS = [
  { id: 'all', label: 'Todos', test: () => true },
  { id: 'pinned', label: 'Fijados', test: (p) => p.pinned },
  { id: 'custom', label: 'Personalizados', test: (p) => p.origin !== 'pack' },
  { id: 'pack', label: 'Modpacks', test: (p) => p.origin === 'pack' }
];

function renderProfiles() {

  els.profileTabs.textContent = '';
  for (const t of PROFILE_TABS) {
    const n = state.profiles.filter(t.test).length;
    els.profileTabs.append(h('button', {
      class: `seg-btn${state.profileTab === t.id ? ' active' : ''}`, type: 'button',
      onclick: () => { state.profileTab = t.id; renderProfiles(); }
    }, t.label, h('span', { class: 'count', text: String(n) })));
  }

  const tab = PROFILE_TABS.find((t) => t.id === state.profileTab) || PROFILE_TABS[0];
  const q = els.profileSearch.value.trim().toLowerCase();
  const visible = state.profiles
    .filter(tab.test)
    .filter((p) => !q || p.name.toLowerCase().includes(q) || p.mc.includes(q))
    .sort((a, b) => (b.pinned - a.pinned) || ((b.lastPlayed || 0) - (a.lastPlayed || 0)) || b.created - a.created);

  els.profileGroups.textContent = '';
  if (!state.profiles.length) {
    els.profileGroups.append(h('div', { class: 'empty-card big' },
      h('p', { class: 'muted', text: 'Todavía no tenés perfiles. Un perfil es una instalación aparte de Minecraft con sus propios mods y mundos.' }),
      h('button', { class: 'btn btn-green', type: 'button', onclick: () => openProfileWizard() }, icon('plus'), 'Crear mi primer perfil')));
    return;
  }

  const groups = [...state.folders.map((f) => ({ name: f, key: `f:${f}`, real: true })), { name: 'Desagrupados', key: 'f:', real: false }];
  for (const g of groups) {
    const items = visible.filter((p) => (p.folder || '') === (g.real ? g.name : ''));
    if (g.real === false && !items.length && state.folders.length) continue;
    const collapsed = state.collapsed.has(g.key);
    const head = h('div', { class: 'group-head' },
      h('button', {
        class: 'group-toggle', type: 'button',
        onclick: () => { collapsed ? state.collapsed.delete(g.key) : state.collapsed.add(g.key); renderProfiles(); }
      }, icon(collapsed ? 'chevRight' : 'chevDown', 16), icon('folder', 18), h('span', { text: `${g.name} (${items.length})` })),
      g.real ? h('button', {
        class: 'btn-icon small', type: 'button', title: 'Eliminar carpeta', 'aria-label': 'Eliminar carpeta',
        onclick: async () => {
          if (!(await confirmDialog({ title: 'Eliminar carpeta', text: `Se elimina la carpeta «${g.name}». Los perfiles que tiene pasan a «Desagrupados».`, okLabel: 'Eliminar', danger: true }))) return;
          try { state.folders = await call(window.launcher.folders('remove', g.name)); await loadProfiles(); } catch (e) { toast(e.message, 'error'); }
        }
      }, icon('trash', 16)) : null);
    els.profileGroups.append(head);
    if (!collapsed) {
      const grid = h('div', { class: 'profile-grid' });
      if (!items.length) grid.append(h('div', { class: 'muted small group-empty', text: 'Sin perfiles acá.' }));
      items.forEach((p) => grid.append(profileCard(p)));
      els.profileGroups.append(grid);
    }
  }
}

els.profileSearch.addEventListener('input', renderProfiles);

function profileCard(p) {
  const active = state.activeId === p.id && (state.busy || state.running);
  const running = active && state.running;
  const card = h('div', { class: `profile-card2${running ? ' running' : ''}` });
  const b = banner(p, 'banner-card');
  if (p.pinned) b.append(h('span', { class: 'pin-badge', title: 'Fijado' }, icon('pin', 14)));

  const gear = h('button', { class: 'btn-icon', type: 'button', title: 'Opciones', 'aria-label': 'Opciones del perfil' }, icon('settings'));
  gear.addEventListener('click', (e) => { e.stopPropagation(); openProfileMenu(gear, p); });

  const play = running
    ? h('button', { class: 'btn btn-stop btn-block', type: 'button', onclick: (e) => { e.stopPropagation(); window.launcher.stop(); } }, h('span', { class: 'stop-ico' }), 'Detener')
    : h('button', {
      class: 'btn btn-play btn-block', type: 'button', disabled: !canPlay(),
      onclick: (e) => { e.stopPropagation(); playProfile(p.id); }
    }, icon('play'), state.account ? 'Jugar' : 'Inicia sesión');

  card.append(
    b,
    h('div', { class: 'card-body' },
      h('strong', { class: 'card-title', text: p.name }),
      h('span', { class: 'muted sub' }, icon('fabric', 15), `${p.loaderLabel} ${p.mc}`),
      h('div', { class: 'card-actions' }, play, gear),
      h('span', { class: 'muted small', text: active ? (running ? 'Jugando ahora' : 'Preparando…') : timeAgo(p.lastPlayed) }))
  );
  card.addEventListener('click', () => goTo('detail', { id: p.id }));
  card.addEventListener('contextmenu', (e) => { e.preventDefault(); openProfileMenu(gear, p); });
  return card;
}

function openProfileMenu(anchor, p) {
  const header = h('div', { class: 'pop-head' },
    h('img', { class: 'pixel', src: 'logo.png', alt: '', width: 34, height: 34 }),
    h('div', {}, h('strong', { text: p.name }), h('span', { class: 'muted small', text: `${p.loaderLabel} ${p.mc} · ${p.origin === 'pack' ? 'Paquete' : 'Custom Profile'}` })));
  popMenu(anchor, [
    { label: 'Editar', icon: 'edit', onclick: () => openEditProfile(p) },
    { label: 'Ajustes', icon: 'settings', onclick: () => goTo('detail', { id: p.id, tab: 'settings' }) },
    'sep',
    { label: 'Duplicar', icon: 'layers', onclick: () => duplicateProfile(p) },
    { label: 'Copiar del perfil…', icon: 'import', onclick: () => openCopyFrom(p) },
    { label: 'Exportar…', icon: 'export', onclick: () => exportProfile(p) },
    { label: 'Compartir perfil…', icon: 'share', onclick: () => openShareModal(p) },
    { label: 'Mostrar en carpeta', icon: 'folder', onclick: () => call(window.launcher.openProfileFolder(p.id)).catch((e) => toast(e.message, 'error')) },
    { label: p.pinned ? 'Desfijar' : 'Fijar arriba', icon: 'pin', onclick: () => pinProfile(p) },
    'sep',
    { label: 'Eliminar perfil…', icon: 'trash', danger: true, onclick: () => deleteProfile(p) }
  ], header);
}

async function pinProfile(p) {
  try { await call(window.launcher.updateProfile(p.id, { pinned: !p.pinned })); await loadProfiles(); } catch (e) { toast(e.message, 'error'); }
}

async function duplicateProfile(p) {
  try {
    const c = await call(window.launcher.duplicateProfile(p.id));
    await loadProfiles();
    toast(`Perfil duplicado: ${c.name}`);
  } catch (e) { toast(e.message, 'error'); }
}

async function exportProfile(p) {
  try {
    const file = await call(window.launcher.exportProfile(p.id));
    if (file) toast('Perfil exportado.');
  } catch (e) { toast(e.message, 'error'); }
}

async function deleteProfile(p) {
  const ok = await confirmDialog({
    title: 'Eliminar perfil',
    text: p.legacy
      ? `Se quita «${p.name}» de la lista. Tu carpeta principal del juego (mundos y mods) no se borra.`
      : `Se elimina «${p.name}» con sus mods, mundos y configuración. No se puede deshacer.`,
    okLabel: 'Eliminar',
    danger: true
  });
  if (!ok) return;
  try {
    await call(window.launcher.deleteProfile(p.id));
    await loadProfiles();
    if (state.view === 'detail') goTo('profiles');
    toast('Perfil eliminado.');
  } catch (e) { toast(e.message, 'error'); }
}

function openEditProfile(p) {
  const name = h('input', { type: 'text', class: 'input', value: p.name, spellcheck: 'false', maxlength: 60 });
  const folder = h('select', { class: 'input' }, h('option', { value: '', text: 'Desagrupados' }), state.folders.map((f) => h('option', { value: f, text: f })));
  folder.value = p.folder || '';
  const m = modal({
    title: 'Editar perfil',
    icon: 'edit',
    className: 'small',
    body: [
      h('label', { class: 'field' }, h('span', { class: 'field-label', text: 'Nombre' }), name),
      h('label', { class: 'field' }, h('span', { class: 'field-label', text: 'Carpeta' }), folder),
      h('div', { class: 'field' }, h('span', { class: 'field-label', text: 'Versión' }),
        h('div', { class: 'row-between' },
          h('span', { text: `${p.loaderLabel} ${p.mc}` }),
          h('button', { class: 'btn btn-dark btn-small', type: 'button', text: 'Cambiar versión', onclick: () => { m.close(); changeVersion(p); } })))
    ],
    footer: [
      h('button', { class: 'btn btn-dark', type: 'button', text: 'Cancelar', onclick: () => m.close() }),
      h('button', {
        class: 'btn btn-accent', type: 'button', text: 'Guardar',
        onclick: async () => {
          try {
            await call(window.launcher.updateProfile(p.id, { name: name.value, folder: folder.value }));
            m.close();
            await loadProfiles();
          } catch (e) { toast(e.message, 'error'); }
        }
      })
    ]
  });
  setTimeout(() => name.focus(), 30);
}

function openCopyFrom(p) {
  const others = state.profiles.filter((x) => x.id !== p.id);
  if (!others.length) { toast('No hay otros perfiles para copiar.'); return; }
  const list = h('div', { class: 'pick-list' });
  let chosen = null;
  others.forEach((o) => {
    const row = h('button', { class: 'pick-row', type: 'button' },
      h('img', { class: 'pixel', src: 'logo.png', alt: '', width: 32, height: 32 }),
      h('div', {}, h('strong', { text: o.name }), h('span', { class: 'muted small', text: `${o.loaderLabel} ${o.mc}` })));
    row.addEventListener('click', () => {
      chosen = o;
      list.querySelectorAll('.pick-row').forEach((r) => r.classList.toggle('selected', r === row));
    });
    list.append(row);
  });
  const m = modal({
    title: 'Copiar del perfil',
    icon: 'import',
    className: 'small',
    body: [h('p', { class: 'modal-text', text: `Elegí de qué perfil traer mods, paquetes de recursos y configuración hacia «${p.name}». Lo que ya tenés no se borra.` }), list],
    footer: [
      h('button', { class: 'btn btn-dark', type: 'button', text: 'Cancelar', onclick: () => m.close() }),
      h('button', {
        class: 'btn btn-accent', type: 'button', text: 'Copiar',
        onclick: async () => {
          if (!chosen) { toast('Elegí un perfil.'); return; }
          try { await call(window.launcher.copyFromProfile(p.id, chosen.id)); m.close(); toast('Contenido copiado.'); if (state.view === 'detail') detailRefresh(true); } catch (e) { toast(e.message, 'error'); }
        }
      })
    ]
  });
}

function openShareModal(p) {
  const m = modal({
    title: `Compartir «${p.name}»`,
    icon: 'share',
    body: [shareView(p)],
    footer: [h('button', { class: 'btn btn-dark', type: 'button', text: 'Cerrar', onclick: () => m.close() })]
  });
}

$('btn-new-folder').addEventListener('click', async () => {
  const name = await promptDialog({ title: 'Nueva carpeta', label: 'Nombre de la carpeta', okLabel: 'Crear' });
  if (!name) return;
  try { state.folders = await call(window.launcher.folders('add', name)); await loadProfiles(); } catch (e) { toast(e.message, 'error'); }
});

async function finishImport(r) {
  if (!r) return;
  await loadProfiles();
  notify(`Perfil importado: ${r.profile.name}`);
  if (r.failed.length) {
    const m = modal({
      title: 'Importado con avisos',
      icon: 'warn',
      body: [h('p', { class: 'modal-text', text: 'El perfil se creó, pero estos elementos no se pudieron instalar:' }), h('ul', { class: 'plain-list' }, r.failed.map((f) => h('li', { text: f })))],
      footer: [h('button', { class: 'btn btn-accent', type: 'button', text: 'Entendido', onclick: () => m.close() })]
    });
  } else {
    toast(`Perfil importado: ${r.profile.name}`);
  }
  goTo('detail', { id: r.profile.id, tab: 'content' });
}

$('btn-import').addEventListener('click', (e) => {
  popMenu(e.currentTarget, [
    {
      label: 'Desde un archivo…', icon: 'file',
      async onclick() {
        try { await finishImport(await call(window.launcher.importProfileFile())); } catch (err) { toast(err.message, 'error'); }
      }
    },
    {
      label: 'Desde un código…', icon: 'link',
      async onclick() {
        const code = await promptDialog({ title: 'Importar desde código', label: 'Pegá el código del perfil (empieza con NOVA1:)', okLabel: 'Importar' });
        if (!code) return;
        try { toast('Importando perfil…'); await finishImport(await call(window.launcher.importProfileCode(code))); } catch (err) { toast(err.message, 'error'); }
      }
    }
  ]);
});

$('btn-new-profile').addEventListener('click', () => openProfileWizard());

async function changeVersion(profile) {
  const busy = progressToast({ title: 'Cargando versiones compatibles', text: `Buscando versiones disponibles para ${profile.name}.` });
  try {
    if (!state.versions.length) await loadVersions();
  } finally {
    busy.close();
  }
  openProfileWizard({ profile });
}

function openProfileWizard({ profile = null } = {}) {
  const editing = !!profile;
  const sel = {
    name: editing ? profile.name : '',
    mc: profile?.mc || state.settings?.lastVersion || state.versions[0]?.id || '',
    type: profile ? (profile.loader === 'fabric' ? 'nova' : 'vanilla') : 'nova',
    autoSync: profile ? profile.autoSync !== false : true,
    folder: profile?.folder || ''
  };
  let m = null;
  let step = 1;

  function renderStep1() {
    const name = h('input', { type: 'text', class: 'input', value: sel.name, placeholder: 'Nombre del perfil (opcional)', spellcheck: 'false', maxlength: 60 });
    name.addEventListener('input', () => { sel.name = name.value; });
    const search = h('input', { type: 'text', class: 'input', placeholder: 'Buscar versión (ej: 1.21.4)', spellcheck: 'false' });
    const grid = h('div', { class: 'version-grid' });
    const draw = () => {
      const q = search.value.trim().toLowerCase();
      const list = state.versions.filter((v) => !q || v.id.toLowerCase().includes(q)).slice(0, 120);
      grid.textContent = '';
      if (!list.length) grid.append(h('div', { class: 'version-empty', text: state.versionsLoaded ? 'No hay versiones que coincidan.' : 'Las versiones todavía no se cargaron.' }));
      for (const v of list) {
        grid.append(h('button', {
          class: `ver-card${v.id === sel.mc ? ' selected' : ''}`, type: 'button',
          onclick: () => { sel.mc = v.id; draw(); }
        }, h('strong', { text: v.id }), h('span', { text: v.type === 'snapshot' ? 'Snapshot' : 'Release' })));
      }
    };
    search.addEventListener('input', draw);
    draw();

    const types = h('div', { class: 'type-cards' },
      typeCard('nova', 'Con Trident Client', 'Fabric + HUD, amigos, zoom y más.'),
      typeCard('vanilla', 'Vanilla', 'Minecraft sin mods.'));
    function typeCard(id, title, text) {
      const el = h('button', { class: `type-card${sel.type === id ? ' selected' : ''}`, type: 'button' }, h('strong', { text: title }), h('span', { class: 'muted small', text }));
      el.addEventListener('click', () => {
        sel.type = id;
        types.querySelectorAll('.type-card').forEach((c) => c.classList.toggle('selected', c === el));
        next.textContent = sel.type === 'nova' ? 'Siguiente' : (editing ? 'Guardar' : 'Crear perfil');
      });
      return el;
    }

    const next = h('button', { class: 'btn btn-accent', type: 'button', text: sel.type === 'nova' ? 'Siguiente' : (editing ? 'Guardar' : 'Crear perfil'), onclick: () => (sel.type === 'nova' ? goStep2() : submit()) });
    setModal({
      title: editing ? 'Cambiar versión' : 'Nuevo perfil',
      icon: 'cube',
      wide: true,
      body: [editing ? null : h('label', { class: 'field' }, h('span', { class: 'field-label', text: 'Nombre' }), name), h('div', { class: 'field' }, h('span', { class: 'field-label', text: 'Versión de Minecraft' }), search, grid), h('div', { class: 'field' }, h('span', { class: 'field-label', text: 'Tipo' }), types)],
      footer: [h('button', { class: 'btn btn-dark', type: 'button', text: 'Cancelar', onclick: () => m.close() }), next]
    });
  }

  function renderStep2() {
    const card = (id, label, sub, opts = {}) => {
      const el = h('button', { class: `loader-card ${id}${opts.selected ? ' selected' : ''}${opts.disabled ? ' disabled' : ''}`, type: 'button', disabled: !!opts.disabled },
        h('span', { class: 'loader-art' }, h('img', { src: `loaders/${id}.png`, alt: label, draggable: false, class: id === 'forge' ? '' : 'px' })),
        h('span', { class: 'loader-name' }, label, opts.beta ? h('em', { class: 'beta', text: 'BETA' }) : null),
        h('span', { class: 'loader-sub', text: sub }));
      return el;
    };
    const sync = toggle(sel.autoSync, (v) => { sel.autoSync = v; }, 'Sincronización automática');
    setModal({
      title: 'Elija un cargador',
      icon: 'cube',
      wide: true,
      body: [
        h('p', { class: 'modal-sub', text: `Con Trident · Minecraft ${sel.mc}` }),
        h('div', { class: 'loader-row' },
          card('fabric', 'Fabric', sel.mc, { selected: true }),
          card('neoforge', 'NeoForge', 'Próximamente', { beta: true, disabled: true, icon: 'cube' }),
          card('forge', 'Forge', 'No disponible', { disabled: true, icon: 'layers' }))
      ],
      footer: [
        h('button', { class: 'btn btn-dark', type: 'button', onclick: () => goStep1() }, icon('back'), 'Atrás'),
        h('span', { class: 'spacer' }),
        h('label', { class: 'sync-label' }, 'Sincronización automática', sync),
        h('button', { class: 'btn btn-accent', type: 'button', text: editing ? 'Guardar' : 'Crear perfil', onclick: submit })
      ]
    });
  }

  function setModal(cfg) {
    if (!m) {
      m = modal({ ...cfg, onClose: () => { m = null; } });
    } else {

      const box = m.el;
      box.className = `modal-box${cfg.wide ? ' wide' : ''}`;
      box.querySelector('h3').textContent = cfg.title;
      m.body.textContent = '';
      m.body.append(...cfg.body.filter(Boolean));
      const old = box.querySelector('.modal-foot');
      if (old) old.remove();
      box.append(h('div', { class: 'modal-foot' }, cfg.footer));
    }
  }

  function goStep1() { step = 1; renderStep1(); }
  function goStep2() {
    if (!sel.mc) { toast('Elegí una versión.'); return; }
    step = 2;
    renderStep2();
  }

  async function submit() {
    if (!sel.mc) { toast('Elegí una versión.'); return; }
    const loader = sel.type === 'nova' ? 'fabric' : 'vanilla';
    try {
      if (editing) {
        const busy = progressToast({ title: 'Cambiando versión', text: `Pasando ${profile.name} a Minecraft ${sel.mc}.` });
        try {
          await call(window.launcher.updateProfile(profile.id, { mc: sel.mc, loader, nova: sel.type === 'nova', autoSync: sel.autoSync }));
        } catch (e) { busy.close(0); throw e; }
        m.close();
        await loadProfiles();
        busy.close();
        toast(`Cambié ${profile.name} a ${loader === 'fabric' ? 'Fabric' : 'Vanilla'} ${sel.mc}. Si hay mods, se actualizan al jugar.`, 'success');
      } else {
        const p = await call(window.launcher.createProfile({ name: sel.name, mc: sel.mc, loader, nova: sel.type === 'nova', autoSync: sel.autoSync, folder: sel.folder }));
        m.close();
        await loadProfiles();
        toast(`Perfil creado: ${p.name}`);
        goTo('detail', { id: p.id, tab: 'content' });
      }
    } catch (e) { toast(e.message, 'error'); }
  }

  goStep1();
  void step;
}

function renderBrowse() { renderMods(); }

function formatDate(iso) {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  const days = Math.floor((Date.now() - d.getTime()) / 86400000);
  if (days < 1) return 'Hoy';
  if (days < 30) return `Hace ${days} d`;
  return d.toLocaleDateString('es-AR', { day: 'numeric', month: 'short', year: 'numeric' });
}

let newsLoadedAt = 0;
const NEWS_REFRESH_MS = 24 * 60 * 60 * 1000;
const news = { index: 0, timer: null, count: 0 };

function newsUrl(n) {
  return n.version
    ? `https://www.minecraft.net/es-es/article/minecraft-java-edition-${String(n.version).replace(/\./g, '-')}`
    : 'https://www.minecraft.net/es-es/articles';
}

function showNews(i) {
  if (!news.count) return;
  news.index = (i + news.count) % news.count;
  els.newsList.querySelectorAll('.news-card').forEach((c, k) => c.classList.toggle('show', k === news.index));
  els.newsDots.querySelectorAll('button').forEach((d, k) => d.classList.toggle('active', k === news.index));
}

function scheduleNews() {
  clearInterval(news.timer);
  news.timer = null;
  if (news.count > 1) {
    news.timer = setInterval(() => {
      if (state.view === 'home' && !document.hidden && !state.running) showNews(news.index + 1);
    }, 7000);
  }
}

async function loadNews() {
  const hadCards = els.newsList.querySelector('.news-card');
  try {
    const items = await call(window.launcher.news());
    if (!items.length) throw new Error('vacío');
    els.newsList.textContent = '';
    els.newsDots.textContent = '';
    newsLoadedAt = Date.now();
    items.forEach((n, k) => {
      const card = h('button', { class: 'news-card', type: 'button', title: 'Leer en minecraft.net' },
        h('span', { class: 'news-tag', text: n.version ? `Versión ${n.version}` : 'Novedad' }),
        h('span', { class: 'news-title', text: n.title }),
        h('span', { class: 'news-date', text: formatDate(n.date) }));
      if (n.image) card.style.backgroundImage = `url("${n.image}")`;
      card.addEventListener('click', () => call(window.launcher.openExternal(newsUrl(n))).catch((e) => toast(e.message, 'error')));
      els.newsList.append(card);
      els.newsDots.append(h('button', { type: 'button', 'aria-label': `Novedad ${k + 1}`, onclick: () => { showNews(k); scheduleNews(); } }));
    });
    news.count = items.length;
    showNews(0);
    scheduleNews();
  } catch {
    if (hadCards) return;
    news.count = 0;
    els.newsList.textContent = '';
    els.newsDots.textContent = '';
    els.newsList.append(h('div', { class: 'news-empty muted small', text: 'No se pudieron cargar las novedades.' }));
  }
}

els.newsList.addEventListener('mouseenter', () => { clearInterval(news.timer); news.timer = null; });
els.newsList.addEventListener('mouseleave', scheduleNews);

setInterval(() => { if (Date.now() - newsLoadedAt >= NEWS_REFRESH_MS || !newsLoadedAt) loadNews(); }, 60 * 60 * 1000);
window.addEventListener('focus', () => { if (Date.now() - newsLoadedAt >= NEWS_REFRESH_MS) loadNews(); });

async function loadVersions() {
  state.versionsLoaded = false;
  try {
    state.versions = await call(window.launcher.listVersions(state.settings.showSnapshots));
    state.versionsLoaded = state.versions.length > 0;
  } catch (e) {
    showError(`No se pudo cargar la lista de versiones: ${e.message}`);
  }
}

const upd = { launcher: null };

function renderUpdate() {
  const u = upd.launcher;
  const pill = els.updatePill;
  pill.classList.remove('clickable', 'pulse');
  pill.onclick = null;
  if (!u || ['none', 'checking', 'error', 'unsupported'].includes(u.state)) {
    pill.classList.add('hidden');
    return;
  }
  pill.classList.remove('hidden');
  pill.title = u.version ? `Versión ${u.version}` : '';
  if (u.state === 'available' || u.state === 'countdown') {
    pill.textContent = u.state === 'countdown' ? `Actualización disponible · se instala en ${u.seconds}s` : 'Actualización disponible';
    pill.classList.add('clickable', 'pulse');
    pill.onclick = askUpdate;
  } else if (u.state === 'downloading') {
    pill.textContent = `Descargando actualización${u.version ? ' ' + u.version : ''}… ${u.percent ?? 0}%`;
  } else if (u.state === 'ready') {
    pill.textContent = u.waitingGame ? 'Actualización lista: se instala al cerrar Minecraft' : 'Instalando actualización…';
  }
}

let askingUpdate = false;
async function askUpdate() {
  if (askingUpdate) return;
  askingUpdate = true;
  try {
    await call(window.launcher.holdUpdate());
    const running = !!state.running;
    const ok = await confirmDialog({
      title: 'Actualización disponible',
      text: running
        ? 'Minecraft está abierto. Si continuás, la actualización se descarga ahora y se instala cuando cierres Minecraft.'
        : 'El launcher se va a cerrar y a abrir de nuevo para instalar la actualización.',
      okLabel: 'Continuar'
    });
    if (ok) await call(window.launcher.startUpdate());
  } catch (e) { showError(e.message); } finally { askingUpdate = false; }
}

window.launcher.onUpdate(async (u) => {
  if (u.kind === 'launcher') {
    upd.launcher = u;
    renderUpdate();
    const text = {
      checking: 'Buscando actualizaciones…',
      none: 'Estás en la última versión.',
      available: `Hay una actualización disponible (${u.version || ''}).`,
      countdown: '',
      downloading: `Descargando la versión ${u.version || ''}…`,
      ready: u.waitingGame ? `La versión ${u.version || ''} está lista. Se instala al cerrar Minecraft.` : `Instalando la versión ${u.version || ''}…`,
      unsupported: 'Las actualizaciones automáticas solo funcionan en la app instalada (no con «npm run dev»).',
      error: !u.message ? 'No se pudo buscar actualizaciones (¿sin conexión?).'
        : /^No se pudo/.test(u.message) ? u.message : `No se pudo buscar actualizaciones: ${u.message}`
    }[u.state];

    if (u.state === 'error' && /^No se pudo (descargar|instalar)/.test(u.message || '')) toast(u.message, 'error');
    if (text && typeof setUpdateText === 'function') setUpdateText(text);
    if ((u.state === 'available' || u.state === 'countdown') && upd.notified !== u.version) { upd.notified = u.version; notify(`Actualización ${u.version || ''} disponible.`); }
  } else if (u.kind === 'mods') {
    if (typeof setUpdateText === 'function') setUpdateText(`Mod de Trident actualizado (${u.tag}).`);
    notify(`Mod de Trident actualizado (${u.tag}).`);
    state.info = await call(window.launcher.appInfo());
  }
});

function renderInstallInfo() {
  const gb = state.profiles.length && lastProfile()?.ramGB ? lastProfile().ramGB : state.settings.ramGB;
  els.infoRam.textContent = `${gb} GB`;
  els.infoJava.textContent = state.settings.javaPath ? 'Personalizado' : 'Automático';
}

(async function init() {
  mountStaticIcons();
  mountRail();
  renderAccount(null);
  if (typeof mountSocial === 'function') mountSocial();

  const [info, settings] = await Promise.all([
    call(window.launcher.appInfo()),
    call(window.launcher.getSettings())
  ]);
  state.settings = settings;
  state.info = info;
  applyAppearance(settings);

  if (Number(settings.uiScale) && Number(settings.uiScale) !== 1) saveSetting({ uiScale: 1 }).catch(() => {});
  $('brand-ver').textContent = info.version ? `Versión ${info.version}` : '';
  state.running = !!info.running;
  state.activeId = info.running ? info.running.profileId : null;

  await loadProfiles();
  renderInstallInfo();
  goTo('home');

  loadNews();
  await loadVersions();

  setProgress('Comprobando sesión…', null);
  try {
    renderAccount(await call(window.launcher.restoreSession()));
  } catch {
    renderAccount(null);
  }
  setProgress('Todo listo.', 0);
  if (typeof window.hideBoot === 'function') window.hideBoot();
})();
