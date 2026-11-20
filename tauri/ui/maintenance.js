'use strict';

// Modo mantenimiento del launcher. Lo activa el bot de Discord con /mantenimiento (o un Developer
// desde la web): el estado vive en Supabase, en la tabla site_status (la misma que usa la web).
// Mientras dura, tapa el launcher con un aviso y una cuenta regresiva y no deja iniciar el juego.
// Si no hay internet o Supabase no responde, el launcher funciona normal (nunca deja a nadie afuera
// por un error de red). Una partida que ya está abierta no se cierra.

(function () {
  const SUPABASE_URL = 'https://dijtdqsvbrnjncdfwytj.supabase.co';
  const SUPABASE_KEY = 'sb_publishable_gt-uuugH-NMWQv-om-X1Eg_aQ3nqWDX'; // clave pública (la misma de la web)
  const CHECK_MS = 30000;

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
      const res = await fetch(`${SUPABASE_URL}/rest/v1/site_status?id=eq.1&select=maintenance,scope,message,until`, {
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

  // Devuelve true si hay mantenimiento ahora (y actualiza la pantalla)
  async function check() {
    if (checking) return checking;
    checking = (async () => {
      try {
        status = await fetchStatus();
      } catch (e) {
        console.warn('[mantenimiento] No pude consultar el estado:', e.message);
        // sin red: se mantiene lo último que se supo
      }
      render();
      return isActive(status);
    })();
    try { return await checking; } finally { checking = null; }
  }

  function left(until) {
    const ms = new Date(until) - new Date();
    if (ms <= 0) return null;
    const s = Math.floor(ms / 1000);
    const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60), x = s % 60;
    const pad = n => String(n).padStart(2, '0');
    return `${d ? `${d} d ` : ''}${pad(h)}:${pad(m)}:${pad(x)}`;
  }

  function build() {
    overlay = document.createElement('div');
    overlay.className = 'maint';
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-modal', 'true');
    overlay.setAttribute('aria-labelledby', 'maint-title');
    overlay.innerHTML = `
      <div class="maint-card">
        <img class="maint-logo pixel" src="logo.png" alt="" width="64" height="64" draggable="false" />
        <p class="maint-eyebrow">Mantenimiento</p>
        <h2 id="maint-title" class="maint-title">Volvemos <span>enseguida</span></h2>
        <p class="maint-msg" data-msg></p>
        <div class="maint-count" data-box>
          <span class="maint-label">Volvemos en</span>
          <span class="maint-clock" data-clock aria-hidden="true">--:--:--</span>
          <span class="maint-ends" data-ends></span>
        </div>
        <p class="maint-note">El launcher se desbloquea solo cuando termina. No hace falta cerrarlo.</p>
        <button class="maint-btn" type="button" data-retry>Volver a comprobar</button>
      </div>`;
    overlay.querySelector('[data-retry]').addEventListener('click', async e => {
      const b = e.currentTarget;
      b.disabled = true; b.textContent = 'Comprobando…';
      await check();
      if (overlay) { b.disabled = false; b.textContent = 'Volver a comprobar'; }
    });
    document.body.appendChild(overlay);
  }

  function render() {
    clearTimeout(tickTimer);
    if (!isActive(status)) {
      if (overlay) { overlay.remove(); overlay = null; }
      document.documentElement.classList.remove('maint-on');
      return;
    }
    if (!overlay) build();
    document.documentElement.classList.add('maint-on');
    overlay.querySelector('[data-msg]').textContent =
      status.message || 'Estamos haciendo mejoras en Trident Client. Gracias por la paciencia.';
    const box = overlay.querySelector('[data-box]');
    box.classList.toggle('hidden', !status.until);
    if (status.until) {
      const end = new Date(status.until);
      const hora = end.toLocaleTimeString('es-ES', { hour: '2-digit', minute: '2-digit' });
      overlay.querySelector('[data-ends]').textContent =
        end.toDateString() === new Date().toDateString() ? `Hoy a las ${hora}`
          : `${end.toLocaleDateString('es-ES', { weekday: 'long', day: 'numeric', month: 'long' })}, ${hora}`;
      tick();
    }
  }

  function tick() {
    if (!overlay || !status || !status.until) return;
    const t = left(status.until);
    overlay.querySelector('[data-clock]').textContent = t || '00:00:00';
    if (!t) { check(); return; } // terminó: confirmar con Supabase y desbloquear
    tickTimer = setTimeout(tick, 1000);
  }

  // No se puede iniciar el juego durante el mantenimiento (se comprueba justo antes de jugar)
  if (typeof playProfile === 'function') {
    const original = playProfile;
    // eslint-disable-next-line no-global-assign
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
