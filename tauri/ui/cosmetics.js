'use strict';

const cos = {
  wallet: null,
  tab: 'shop',
  selected: null,
  busy: false,
  viewer: null,
  skinFor: null,
  mounted: false
};

function cosWallet() {
  return cos.wallet || window.tridentWallet || null;
}

function cosItems() {
  const w = cosWallet();
  const shop = w && Array.isArray(w.shop) ? w.shop : [];
  const owned = new Set(w && Array.isArray(w.owned) ? w.owned : []);
  const equipped = new Set(w && Array.isArray(w.equipped) ? w.equipped : []);
  return shop
    .filter((it) => it && typeof it.id === 'string')
    .map((it) => ({ id: it.id, name: it.name || it.id, price: Number(it.price) || 0, owned: owned.has(it.id), equipped: equipped.has(it.id) }))
    .filter((it) => cos.tab === 'shop' || it.owned);
}

function cosSetWallet(w) {
  if (!w || typeof w !== 'object') return;
  cos.wallet = w;
  window.tridentWallet = w;
  if (typeof viewer3d !== 'undefined' && viewer3d && typeof viewer3d.setCosmetics === 'function') {
    viewer3d.setCosmetics(Array.isArray(w.equipped) ? w.equipped : []);
  }
  if (cos.mounted && state.view === 'cosmetics') cosRender();
}

window.addEventListener('trident-wallet', (e) => cosSetWallet(e.detail));

async function cosCall(op, id, on) {
  if (cos.busy) return;
  if (!window.launcher || typeof window.launcher.shopCall !== 'function') return;
  cos.busy = true;
  cosRender();
  try {
    const data = await call(window.launcher.shopCall(op, id, on));
    if (data && data.wallet) cosSetWallet(data.wallet);
    if (op === 'buy') toast('¡Listo! Ya es tuyo. También aparece en el juego.', 'success');
    if (op === 'equip') toast(on ? 'Equipado. Se aplica también en Minecraft.' : 'Quitado.', 'info');
  } catch (e) {
    toast((e && e.message) || 'No se pudo conectar con Trident Net.', 'error');
  } finally {
    cos.busy = false;
    cosRender();
  }
}

function cosMount() {
  if (cos.mounted) return;
  cos.mounted = true;
  const canvas = $('cos3d');
  try {
    cos.viewer = canvas && window.Skin3D && Skin3D.supported() ? Skin3D.create(canvas, $('cos-stage')) : null;
  } catch {
    cos.viewer = null;
  }
  document.querySelectorAll('#cos-tabs [data-tab]').forEach((b) => {
    b.addEventListener('click', () => { cos.tab = b.dataset.tab; cosRender(); });
  });
  $('cos-action').addEventListener('click', () => {
    const w = cosWallet();
    const it = (w?.shop || []).find((x) => x.id === cos.selected);
    if (!it) return;
    const owned = (w.owned || []).includes(it.id);
    const equipped = (w.equipped || []).includes(it.id);
    if (!owned) cosCall('buy', it.id);
    else cosCall('equip', it.id, !equipped);
  });
}

async function cosLoadSkin() {
  if (!cos.viewer) return;
  const key = state?.account?.uuid || 'steve';
  if (cos.skinFor === key) return;
  cos.skinFor = key;
  try {
    const data = await call(state?.account ? window.launcher.getMySkin() : window.launcher.getSkin('MHF_Steve'));
    await cos.viewer.setSkin(data.skin, data.model);
  } catch (e) {
    cos.skinFor = null;
    console.warn('[Cosméticos] skin', e);
  }
}

function cosPreview() {
  if (!cos.viewer) return;
  cos.viewer.setCosmetics(cos.selected ? [cos.selected] : []);
}

function cosRender() {
  if (!cos.mounted) return;
  const w = cosWallet();
  const known = !!w;
  $('cos-coins').textContent = known ? `${Number(w.coins) || 0} coins` : '— coins';
  const daily = $('cos-daily');
  daily.classList.toggle('hidden', !(known && w.canClaim));
  daily.textContent = `Tenés ${(w && w.daily) || 15} Trident Coins para reclamar hoy: abrí Minecraft con Trident Client.`;
  document.querySelectorAll('#cos-tabs [data-tab]').forEach((b) => b.classList.toggle('active', b.dataset.tab === cos.tab));

  const items = cosItems();
  const grid = $('cos-grid');
  grid.replaceChildren();
  if (!known) {
    grid.append(h('div', { class: 'cos-empty muted', text: state?.account ? 'Conectando con Trident Net…' : 'Iniciá sesión con tu cuenta de Minecraft para ver tus cosméticos.' }));
  } else if (!items.length) {
    grid.append(h('div', { class: 'cos-empty muted', text: cos.tab === 'mine' ? 'Todavía no tenés cosméticos. Conseguilos en la Tienda.' : 'No hay cosméticos en la tienda.' }));
  }
  const before = cos.selected;
  if (cos.selected && !(w?.shop || []).some((x) => x.id === cos.selected)) cos.selected = null;
  if (!cos.selected && items.length) cos.selected = items[0].id;
  if (before !== cos.selected) cosPreview();
  for (const it of items) {
    const status = it.equipped ? 'Equipado' : it.owned ? 'En tu colección' : `${it.price} coins`;
    grid.append(h('button', {
      class: `cos-card${it.id === cos.selected ? ' active' : ''}${it.equipped ? ' on' : ''}`,
      type: 'button',
      onclick: () => { cos.selected = it.id; cosPreview(); cosRender(); }
    }, h('span', { class: 'cos-card-name', text: it.name }), h('span', { class: `cos-badge${it.owned ? ' owned' : ''}`, text: status })));
  }

  const sel = (w?.shop || []).find((x) => x.id === cos.selected);
  const action = $('cos-action');
  if (!sel) {
    $('cos-name').textContent = 'Cosméticos';
    $('cos-tag').textContent = 'Elegí un cosmético para verlo en tu personaje.';
    action.classList.add('hidden');
    return;
  }
  const owned = (w.owned || []).includes(sel.id);
  const equipped = (w.equipped || []).includes(sel.id);
  const price = Number(sel.price) || 0;
  const coins = Number(w.coins) || 0;
  $('cos-name').textContent = sel.name || sel.id;
  $('cos-tag').textContent = equipped ? 'Equipado en el launcher y en Minecraft' : owned ? 'En tu colección' : `${price} Trident Coins`;
  action.classList.remove('hidden');
  action.disabled = cos.busy || (!owned && coins < price);
  action.textContent = cos.busy ? 'Un momento…' : equipped ? 'Quitar' : owned ? 'Equipar' : coins >= price ? `Comprar por ${price} coins` : `Te faltan ${price - coins} coins`;
  action.classList.toggle('btn-accent', !equipped);
  action.classList.toggle('btn-ghost', equipped);
}

function renderCosmetics() {
  cosMount();
  cosLoadSkin().then(cosPreview);
  cosRender();
  if (typeof socialPoll === 'function') socialPoll(true);
}
