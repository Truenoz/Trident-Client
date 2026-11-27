'use strict';

(function () {
  const TOTAL = 15;
  const INTERVAL = 8000;
  const a = document.getElementById('hero-bg-a');
  const b = document.getElementById('hero-bg-b');
  if (!a || !b) return;

  const files = [];
  for (let i = 1; i <= TOTAL; i++) files.push('backgrounds/bg-' + String(i).padStart(2, '0') + '.jpg');

  for (let i = files.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [files[i], files[j]] = [files[j], files[i]];
  }

  const layers = [a, b];
  const walls = [document.getElementById('wall-a'), document.getElementById('wall-b')];
  let idx = 0;
  let front = 0;
  let timer = null;

  function preload(src) {
    return new Promise((resolve) => {
      const img = new Image();
      img.onload = () => resolve(true);
      img.onerror = () => resolve(false);
      img.src = src;
    });
  }

  function paint(layer, src) {
    layer.style.backgroundImage = 'url("' + src + '")';
    const w = walls[layers.indexOf(layer)];
    if (w) w.style.backgroundImage = 'url("' + src + '")';
  }

  function show(layer, on) {
    layer.classList.toggle('show', on);
    const w = walls[layers.indexOf(layer)];
    if (w) w.classList.toggle('show', on);
  }

  async function next() {
    if (document.body.classList.contains('game-running')) return;
    const nextIdx = (idx + 1) % files.length;
    const ok = await preload(files[nextIdx]);
    idx = nextIdx;
    if (!ok) return;
    const incoming = layers[1 - front];
    paint(incoming, files[idx]);
    show(incoming, true);
    show(layers[front], false);
    front = 1 - front;
  }

  function reduced() { return document.body.classList.contains('reduce-motion'); }

  function schedule() {
    clearInterval(timer);
    timer = null;
    if (document.hidden) return;
    timer = setInterval(next, INTERVAL);
  }

  preload(files[0]).then(() => {
    paint(a, files[0]);
    show(a, true);
    schedule();
  });

  document.addEventListener('visibilitychange', schedule);
  new MutationObserver(schedule).observe(document.body, { attributes: true, attributeFilter: ['class'] });
})();
