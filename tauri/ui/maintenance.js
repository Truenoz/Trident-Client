'use strict';

(function () {
  const SUPABASE_URL = 'https://dijtdqsvbrnjncdfwytj.supabase.co';
  const SUPABASE_KEY = 'sb_publishable_gt-uuugH-NMWQv-om-X1Eg_aQ3nqWDX';
  const DISCORD = 'https://discord.gg/9Gj9pH6hnu';
  const CHECK_MS = 30000;

  if (!document.querySelector('link[data-tcm-css]')) {
    const link = document.createElement('link');
    link.rel = 'stylesheet'; link.href = 'maintenance.css'; link.dataset.tcmCss = '';
    document.head.appendChild(link);
  }

  let status = null;
  let overlay = null;
  let tickTimer = null;
  let checking = null;

  function isActive(s) {
    return !!(s && s.maintenance && (s.scope === 'all' || s.scope === 'launcher') &&
      (!s.until || new Date(s.until) > new Date()));
  }

  async function fetchStatus() {
    const ctrl = new AbortController();
    const t = setTimeout(() => ctrl.abort(), 8000);
    try {
      const res = await fetch(`${SUPABASE_URL}/rest/v1/site_status?id=eq.1&select=maintenance,scope,message,until,updated_at`, {
        headers: { apikey: SUPABASE_KEY, Authorization: `Bearer ${SUPABASE_KEY}`, Accept: 'application/json' },
        cache: 'no-store',
        signal: ctrl.signal
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const rows = await res.json();
      return rows[0] || null;
    } finally {
      clearTimeout(t);
    }
  }

  async function check() {
    if (checking) return checking;
    checking = (async () => {
      try {
        status = await fetchStatus();
      } catch (e) {
        console.warn('[mantenimiento] No pude consultar el estado:', e.message);

      }
      render();
      return isActive(status);
    })();
    try { return await checking; } finally { checking = null; }
  }

  const pad = n => String(n).padStart(2, '0');
  function parts(until) {
    const ms = new Date(until) - new Date();
    if (ms <= 0) return null;
    const s = Math.floor(ms / 1000);
    return { d: Math.floor(s / 86400), h: Math.floor((s % 86400) / 3600), m: Math.floor((s % 3600) / 60), s: s % 60 };
  }
  const hora = dt => dt.toLocaleTimeString('es-AR', { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
  function endsText(until) {
    const dt = new Date(until), now = new Date(), man = new Date(now);
    man.setDate(now.getDate() + 1);
    if (dt.toDateString() === now.toDateString()) return `hoy a las ${hora(dt)}`;
    if (dt.toDateString() === man.toDateString()) return `mañana a las ${hora(dt)}`;
    return `el ${dt.toLocaleDateString('es-AR', { weekday: 'long', day: 'numeric', month: 'long' })} a las ${hora(dt)}`;
  }

  const ICON_DISCORD = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8.5 12.5h.01M15.5 12.5h.01"/><path d="M7 16.5c3 1.5 7 1.5 10 0"/><path d="M8.3 5.5A13 13 0 0 0 5 6.6C3 9.8 2.5 13 2.8 16.1A13 13 0 0 0 7 18.4l1-1.7M15.7 5.5A13 13 0 0 1 19 6.6c2 3.2 2.5 6.4 2.2 9.5A13 13 0 0 1 17 18.4l-1-1.7M9 5.2a12 12 0 0 1 6 0"/></svg>';
  const ICON_REFRESH = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 11a8 8 0 1 0-2.3 5.7"/><path d="M20 5v6h-6"/></svg>';

  function build() {
    overlay = document.createElement('div');
    overlay.className = 'tcm';
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-modal', 'true');
    overlay.setAttribute('aria-labelledby', 'tcm-title');
    overlay.setAttribute('aria-describedby', 'tcm-msg');
    overlay.innerHTML = `
      <div class="tcm__wrap">
        <div class="tcm__top"><span class="tcm__pill"><i class="tcm__dot" aria-hidden="true"></i>En mantenimiento</span></div>
        <div class="tcm__main">
          <div class="tcm__copy">
            <p class="tcm__kicker tcm__in">Estamos mejorando el cliente</p>
            <h2 id="tcm-title" class="tcm__title tcm__in d1">Volvemos <em>enseguida</em></h2>
            <p id="tcm-msg" class="tcm__msg tcm__in d2">El launcher está en pausa mientras aplicamos cambios. Cuando terminemos, esta pantalla desaparece sola.</p>
            <div class="tcm__note tcm__in d3 hidden" data-note><b>Mensaje del equipo</b><p data-note-text></p></div>
            <div class="tcm__count tcm__in d4" data-count>
              <span class="tcm__label">Tiempo restante</span>
              <div class="tcm__tiles" aria-hidden="true">
                <div class="tcm__tile" data-dbox><span class="tcm__num" data-d>0</span><span class="tcm__unit">Días</span></div>
                <span class="tcm__sep" data-dsep>:</span>
                <div class="tcm__tile"><span class="tcm__num" data-h>00</span><span class="tcm__unit">Horas</span></div>
                <span class="tcm__sep">:</span>
                <div class="tcm__tile"><span class="tcm__num" data-m>00</span><span class="tcm__unit">Min</span></div>
                <span class="tcm__sep">:</span>
                <div class="tcm__tile"><span class="tcm__num" data-s>00</span><span class="tcm__unit">Seg</span></div>
              </div>
              <div class="tcm__bar" aria-hidden="true"><div class="tcm__fill" data-fill></div></div>
              <p class="tcm__ends">Volvemos <strong data-ends></strong></p>
            </div>
            <p class="tcm__noeta tcm__in d4 hidden" data-noeta>Todavía no hay hora estimada de regreso. Avisamos en Discord apenas esté todo listo.</p>
            <div class="tcm__actions tcm__in d5">
              <button class="tcm__btn tcm__btn--main" type="button" data-discord>${ICON_DISCORD}Novedades en Discord</button>
              <button class="tcm__btn tcm__btn--ghost" type="button" data-retry>${ICON_REFRESH}<span>Volver a comprobar</span></button>
            </div>
          </div>
          <div class="tcm__art tcm__in d2" aria-hidden="true"><span class="tcm__shadow"></span><img class="tcm__char" src="maintenance/personaje.png" alt="" draggable="false"></div>
        </div>
        <div class="tcm__foot"><span class="tcm__live">El launcher se desbloquea solo cuando termina, no hace falta cerrarlo</span><span>Trident Client</span></div>
      </div>`;
    overlay.querySelector('[data-discord]').addEventListener('click', () => {
      try {
        const p = window.launcher && window.launcher.openExternal && window.launcher.openExternal(DISCORD);
        if (p && p.catch) p.catch(e => typeof toast === 'function' && toast(e.message || 'No se pudo abrir Discord.', 'error'));
      } catch (e) { if (typeof toast === 'function') toast('No se pudo abrir Discord.', 'error'); }
    });
    overlay.querySelector('[data-retry]').addEventListener('click', async e => {
      const b = e.currentTarget, label = b.querySelector('span');
      b.disabled = true; b.classList.add('is-busy'); label.textContent = 'Comprobando…';
      await check();
      if (overlay) { b.disabled = false; b.classList.remove('is-busy'); label.textContent = 'Volver a comprobar'; }
    });
    document.body.appendChild(overlay);

    if (document.activeElement && !overlay.contains(document.activeElement)) document.activeElement.blur();
  }

  function render() {
    clearTimeout(tickTimer);
    if (!isActive(status)) {
      if (overlay) { overlay.remove(); overlay = null; }
      document.documentElement.classList.remove('tcm-on');
      return;
    }
    if (!overlay) build();
    document.documentElement.classList.add('tcm-on');
    const msg = (status.message || '').trim();
    overlay.querySelector('[data-note]').classList.toggle('hidden', !msg);
    overlay.querySelector('[data-note-text]').textContent = msg;
    const has = !!status.until;
    overlay.querySelector('[data-count]').classList.toggle('hidden', !has);
    overlay.querySelector('[data-noeta]').classList.toggle('hidden', has);
    if (has) {
      overlay.querySelector('[data-ends]').textContent = endsText(status.until);
      tick();
    }
  }

  function paint(p) {
    const q = k => overlay.querySelector(k);
    const showD = !!(p && p.d);
    q('[data-dbox]').classList.toggle('hidden', !showD);
    q('[data-dsep]').classList.toggle('hidden', !showD);
    q('[data-d]').textContent = p ? p.d : 0;
    q('[data-h]').textContent = pad(p ? p.h : 0);
    q('[data-m]').textContent = pad(p ? p.m : 0);
    q('[data-s]').textContent = pad(p ? p.s : 0);
    const from = status.updated_at ? new Date(status.updated_at) : null, to = new Date(status.until);
    const pct = from && to > from ? Math.min(100, Math.max(0, (new Date() - from) / (to - from) * 100)) : 0;
    q('[data-fill]').style.width = `${p ? pct : 100}%`;
  }

  function tick() {
    if (!overlay || !status || !status.until) return;
    const p = parts(status.until);
    paint(p);
    if (!p) { check(); return; }
    tickTimer = setTimeout(tick, 1000);
  }

  document.addEventListener('keydown', e => {
    if (!overlay || e.key !== 'Tab') return;
    const f = [...overlay.querySelectorAll('button:not([disabled])')];
    if (!f.length) return;
    const i = f.indexOf(document.activeElement);
    if (e.shiftKey ? i <= 0 : i === f.length - 1 || i === -1) { e.preventDefault(); f[e.shiftKey ? f.length - 1 : 0].focus(); }
  }, true);

  if (typeof playProfile === 'function') {
    const original = playProfile;

    playProfile = async function (id, opts) {
      if (await check()) {
        if (typeof toast === 'function') toast('Trident Client está en mantenimiento. Probá cuando termine la cuenta regresiva.');
        return;
      }
      return original(id, opts);
    };
  }

  check();
  setInterval(() => { if (!document.hidden) check(); }, CHECK_MS);
  document.addEventListener('visibilitychange', () => { if (!document.hidden) check(); });
  window.addEventListener('focus', () => check());
  window.tridentMaintenance = { check, isActive: () => isActive(status) };
})();
