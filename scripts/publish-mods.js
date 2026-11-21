'use strict';

const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');

const TAG = 'mods-rolling';
const modDir = path.join(__dirname, '..', 'mod');
const only = process.argv.slice(2).filter((a) => !a.startsWith('-'));

const files = fs.existsSync(modDir)
  ? fs.readdirSync(modDir)
    .map((f) => ({ f, mc: /^(?:trident|nova)-client-(.+)\.jar$/.exec(f)?.[1] }))
    .filter((x) => x.mc && (!only.length || only.includes(x.mc)))
    .map((x) => path.join(modDir, x.f))
  : [];

if (!files.length) {
  console.error('No hay jars para subir en mod/. Compilalos antes con: npm run build:mod');
  process.exit(1);
}

const REPO = process.env.NOVA_REPO ? ['--repo', process.env.NOVA_REPO] : [];

const q = (a) => (process.platform === 'win32' ? `"${String(a).replace(/"/g, '\\"')}"` : a);
const gh = (args) => spawnSync('gh', [...args, ...REPO].map(q), { stdio: 'inherit', shell: process.platform === 'win32' });

if (spawnSync('gh', ['--version'], { shell: process.platform === 'win32' }).status !== 0) {
  console.error('Falta GitHub CLI (gh). Instalalo desde https://cli.github.com y corré: gh auth login');
  process.exit(1);
}

const exists = spawnSync('gh', ['release', 'view', TAG, ...REPO].map(q), { stdio: 'ignore', shell: process.platform === 'win32' }).status === 0;
if (!exists) {
  const r = gh(['release', 'create', TAG, '--prerelease', '--title', 'Mods de Nova Client',
    '--notes', 'Nova Client para cada versión de Minecraft. Lo baja el launcher solo; no hace falta descargarlo a mano.']);
  if (r.status !== 0) process.exit(r.status || 1);
}

console.log(`Subiendo ${files.length} jar(s) a «${TAG}»…`);
const up = gh(['release', 'upload', TAG, ...files, '--clobber']);
process.exit(up.status || 0);
