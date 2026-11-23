'use strict';

const bedrock = { busy: new Set(), req: 0, status: null, left: null, right: null, id: null, poll: null };

function bedrockSize(bytes) {
  if (!bytes) return '0 MB';
  const mb = bytes / (1024 * 1024);
  if (mb >= 1024) return `${(mb / 1024).toFixed(1).replace('.', ',')} GB`;
  if (mb >= 10) return `${Math.round(mb)} MB`;
  return `${mb.toFixed(1).replace('.', ',')} MB`;
}

async function bedrockLoadStatus() {
  try {
    bedrock.status = await call(window.launcher.bedrockStatus());
  } catch (e) {
    bedrock.status = { installed: false, canImport: false, worlds: [], error: e.message };
  }
  return bedrock.status;
}

async function bedrockPlayProfile(p) {
  const st = bedrock.status || await bedrockLoadStatus();
  if (!st.installed) {
    bedrockInstall();
    return;
  }
  try {
    await call(window.launcher.bedrockPlay(p.id));
    toast('Abriendo Minecraft Bedrock…');
    await loadProfiles();
  } catch (e) {
    toast(e.message, 'error');
    bedrockLoadStatus().then(() => bedrockRefreshDetail(p));
  }
}

async function bedrockInstall() {
  try {
    await call(window.launcher.bedrockInstall());
    toast('Confirmá la descarga en la ventana que se abrió. Cuando termine, Bedrock queda listo acá.');
    bedrockWatchInstall();
  } catch (e) {
    toast(e.message, 'error');
  }
}

function bedrockWatchInstall() {
  clearInterval(bedrock.poll);
  let tries = 0;
  bedrock.poll = setInterval(async () => {
    tries += 1;
    const st = await bedrockLoadStatus();
    if (st.installed || tries > 120) {
      clearInterval(bedrock.poll);
      if (st.installed) toast('Minecraft Bedrock está listo para jugar.', 'success');
      const p = detailProfile();
      if (state.view === 'detail' && isBedrock(p)) bedrockRefreshDetail(p);
      renderAll();
    }
  }, 5000);
}

async function bedrockImport(p) {
  try {
    const names = await call(window.launcher.bedrockImport());
    if (!names) return;
    toast(names.length === 1 ? `Mundo «${names[0]}» importado.` : `${names.length} mundos importados.`);
    await bedrockLoadStatus();
    bedrockRefreshDetail(p);
  } catch (e) {
    toast(e.message, 'error');
  }
}

async function bedrockBackup(w, btn) {
  if (bedrock.busy.has(w.path)) return;
  bedrock.busy.add(w.path);
  const label = btn.textContent;
  btn.disabled = true;
  btn.textContent = 'Guardando…';
  try {
    const saved = await call(window.launcher.bedrockBackup(w.path));
    if (saved) toast(`Copia de «${w.name}» guardada.`);
  } catch (e) {
    toast(e.message, 'error');
  } finally {
    bedrock.busy.delete(w.path);
    btn.disabled = false;
    btn.textContent = label;
  }
}

function bedrockWorldRow(w) {
  const backup = h('button', { class: 'btn btn-dark btn-small', type: 'button', text: 'Copia de seguridad' });
  backup.addEventListener('click', () => bedrockBackup(w, backup));
  const meta = [timeAgo(w.modified), bedrockSize(w.size)];
  if (w.source === 'uwp') meta.push('versión anterior');
  return h('div', { class: 'world-row' },
    w.icon ? h('img', { class: 'c-icon', src: w.icon, alt: '', width: 48, height: 48 }) : h('span', { class: 'c-icon ph' }, icon('globe', 24)),
    h('div', { class: 'grow' }, h('strong', { text: w.name }), h('span', { class: 'muted small', text: meta.join(' · ') })),
    backup,
    h('button', {
      class: 'btn btn-dark btn-small', type: 'button', text: 'Abrir carpeta',
      onclick: () => call(window.launcher.bedrockOpen(w.path)).catch((e) => toast(e.message, 'error'))
    }));
}

function bedrockDrawLeft(p) {
  const box = bedrock.left;
  if (!box) return;
  const st = bedrock.status;
  const gear = h('button', { class: 'art-gear', type: 'button', title: 'Opciones', 'aria-label': 'Opciones del perfil' }, icon('settings'));
  gear.addEventListener('click', () => openProfileMenu(gear, p));
  let action;
  if (!st) {
    action = h('button', { class: 'btn btn-play btn-big btn-block', type: 'button', disabled: true }, 'Buscando Bedrock…');
  } else if (st.installed) {
    action = h('button', { class: 'btn btn-play btn-big btn-block', type: 'button', onclick: () => bedrockPlayProfile(p) }, icon('play'), 'Jugar Bedrock');
  } else {
    action = h('button', { class: 'btn btn-play btn-big btn-block', type: 'button', onclick: bedrockInstall }, icon('download', 20), 'Descargar Bedrock');
  }
  box.replaceChildren(...[
    h('div', { class: 'detail-art' }, banner(p, 'banner-detail'), gear),
    h('div', { class: 'detail-tags' }, h('span', { class: 'tag tag-bedrock' }, editionImg(p, 14), 'Bedrock')),
    h('div', { class: 'muted small', text: st && !st.installed ? 'Descargá Minecraft Bedrock para empezar a jugar.' : 'Bedrock Edition: juego cruzado con Xbox y celular.' }),
    action,
    st && !st.installed ? h('button', { class: 'btn btn-dark btn-block', type: 'button', text: 'Ya terminó la descarga', onclick: async () => { await bedrockLoadStatus(); bedrockRefreshDetail(p); renderAll(); } }) : null,
    h('div', { class: 'info-list' },
      infoRow('globe', 'Mundos', st ? String(st.worlds.length) : '—'),
      infoRow('clock', 'Jugado por última vez', timeAgo(p.lastPlayed)))].filter(Boolean));
}

function bedrockDrawRight(p) {
  const box = bedrock.right;
  if (!box) return;
  const st = bedrock.status;
  const importBtn = h('button', { class: 'btn btn-dark', type: 'button', onclick: () => bedrockImport(p) }, icon('import', 18), 'Importar .mcworld');
  const head = h('div', { class: 'content-toolbar bedrock-toolbar' }, h('h3', { text: 'Mundos' }), h('span', { class: 'spacer' }), importBtn);
  if (!st) {
    box.replaceChildren(head, h('div', { class: 'empty-card' }, h('span', { class: 'muted', text: 'Buscando mundos…' })));
    importBtn.disabled = true;
    return;
  }
  importBtn.disabled = !st.canImport;
  if (st.error) {
    box.replaceChildren(head, h('div', { class: 'empty-card' }, h('span', { class: 'muted', text: st.error })));
    return;
  }
  if (!st.installed && !st.worlds.length) {
    box.replaceChildren(head, h('div', { class: 'empty-card big' },
      h('img', { src: 'editions/bedrock.png', alt: '', width: 64, height: 64, draggable: false }),
      h('strong', { text: 'Descargá Minecraft Bedrock' }),
      h('span', { class: 'muted', text: 'Tocá «Descargar Bedrock» y confirmá la descarga. Cuando termine, el perfil queda listo para jugar y tus mundos aparecen acá.' })));
    return;
  }
  if (!st.worlds.length) {
    box.replaceChildren(head, h('div', { class: 'empty-card big' },
      icon('globe', 34),
      h('strong', { text: 'Todavía no hay mundos' }),
      h('span', { class: 'muted', text: st.canImport ? 'Creá uno en el juego o importá un archivo .mcworld.' : 'Abrí Minecraft Bedrock una vez para que cree su carpeta de mundos.' })));
    return;
  }
  box.replaceChildren(head, ...st.worlds.map(bedrockWorldRow));
}

function bedrockRefreshDetail(p) {
  if (!p || bedrock.id !== p.id) return;
  const title = $('detail-title');
  if (title) title.textContent = p.name;
  bedrockDrawLeft(p);
  bedrockDrawRight(p);
}

async function renderBedrockDetail(root, p) {
  const req = ++bedrock.req;
  bedrock.id = p.id;
  bedrock.left = h('aside', { class: 'detail-left' });
  bedrock.right = h('section', { class: 'detail-right' });
  root.append(
    h('div', { class: 'detail-head' },
      h('button', { class: 'btn btn-back', type: 'button', onclick: () => goTo('profiles') }, icon('back'), 'Atrás'),
      h('h2', { id: 'detail-title', text: p.name })),
    h('div', { class: 'detail-grid' }, bedrock.left, bedrock.right));
  bedrockDrawLeft(p);
  bedrockDrawRight(p);
  await bedrockLoadStatus();
  if (req !== bedrock.req) return;
  bedrockRefreshDetail(p);
}
