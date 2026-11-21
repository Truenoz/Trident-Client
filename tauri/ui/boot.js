'use strict';

(function () {
  const MIN_MS = 1500;
  const MAX_MS = 15000;
  const started = performance.now();
  let hidden = false;

  function hide() {
    if (hidden) return;
    hidden = true;
    const el = document.getElementById('boot');
    if (!el) return;
    el.classList.add('boot-out');
    setTimeout(() => el.remove(), 450);
  }

  window.hideBoot = function hideBoot() {
    const wait = Math.max(0, MIN_MS - (performance.now() - started));
    setTimeout(hide, wait);
  };

  setTimeout(hide, MAX_MS);
})();
