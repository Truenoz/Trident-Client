'use strict';

(function () {
  const T = window.__TAURI__;
  if (!T || !T.core) {
    console.error('Tauri no está disponible (withGlobalTauri).');
    return;
  }
  const invoke = T.core.invoke;
  const listen = T.event.listen;

  const msg = (e) => (typeof e === 'string' ? e : (e && e.message) || String(e));

  const cmd = (name, args) => async (...a) => {
    try {
      return { ok: true, data: await invoke(name, args ? args(...a) : {}) };
    } catch (e) {
      return { ok: false, error: msg(e) };
    }
  };
  function subscribe(channel, callback) {
    let off = null;
    let dead = false;
    listen(channel, (e) => callback(e.payload)).then((f) => { if (dead) f(); else off = f; });
    return () => { dead = true; if (off) off(); };
  }

  async function copyText(text) {
    try {
      await navigator.clipboard.writeText(text);
      return;
    } catch (_) {  }
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.style.cssText = 'position:fixed;opacity:0;pointer-events:none';
    document.body.appendChild(ta);
    ta.select();
    try { document.execCommand('copy'); } catch (_) {  }
    ta.remove();
  }

  const fire = (name) => () => { invoke(name).catch(() => {}); };

  window.launcher = {

    appInfo: cmd('app_info'),
    getSettings: cmd('settings_get'),
    setSettings: cmd('settings_set', (patch) => ({ patch: patch || {} })),
    listVersions: cmd('versions_list'),
    news: cmd('news_list'),
    versionArt: cmd('version_art'),
    openExternal: cmd('shell_external', (url) => ({ url })),
    openFolder: cmd('folder_open'),
    openDataFolder: cmd('data_open'),

    listProfiles: cmd('profiles_list'),
    createProfile: cmd('profiles_create', (input) => ({ input: input || {} })),
    updateProfile: cmd('profiles_update', (id, patch) => ({ id, patch: patch || {} })),
    deleteProfile: cmd('profiles_delete', (id) => ({ id })),
    duplicateProfile: cmd('profiles_duplicate', (id) => ({ id })),
    folders: cmd('profiles_folders', (action, name) => ({ action, name: name || '' })),
    openProfileFolder: cmd('profiles_open_folder', (id) => ({ id })),

    clearCache: cmd('danger_clear_cache'),
    resetSettings: cmd('danger_reset_settings'),

    minimize: fire('win_minimize'),
    maximize: fire('win_maximize'),
    close: fire('win_close'),

    restoreSession: cmd('auth_restore'),
    login: cmd('auth_login'),
    offlineLogin: cmd('auth_offline', (name) => ({ name: String(name || '') })),
    logout: cmd('auth_logout'),
    launch: cmd('game_launch', (id, opts) => ({ profileId: String(id), opts: opts || {} })),
    stop: cmd('game_stop'),
    clearJava: cmd('danger_clear_java'),

    copyFromProfile: cmd('profiles_copy_from', (id, sourceId) => ({ id, sourceId })),
    exportProfile: cmd('profiles_export', (id) => ({ id })),
    shareProfile: async (id) => {
      const r = await cmd('profiles_share', (x) => ({ id: x }))(id);
      if (r.ok) await copyText(r.data);
      return r;
    },
    importProfileFile: cmd('profiles_import_file'),
    importProfileCode: cmd('profiles_import_code', (code) => ({ code: String(code || '') })),

    listContent: cmd('content_list', (id) => ({ id })),
    toggleContent: cmd('content_toggle', (id, file, enabled) => ({ id, file, enabled: !!enabled })),
    removeContent: cmd('content_remove', (id, file) => ({ id, file })),
    searchContent: cmd('content_search', (id, q) => ({ id, q: q || {} })),
    browseContent: cmd('content_browse', (q) => ({ q: q || {} })),
    installContent: cmd('content_install', (id, projectId, type) => ({ id, projectId: String(projectId), kind: type || 'mod' })),
    updateContent: cmd('content_update', (id, file) => ({ id, file })),
    identifyContent: cmd('content_identify', (id) => ({ id })),
    checkContentUpdates: cmd('content_check_updates', (id) => ({ id })),
    pickLocalContent: cmd('content_pick_local', (id) => ({ id })),
    addLocalContent: cmd('content_add_local', (id, paths) => ({ id, paths: Array.isArray(paths) ? paths.map(String) : [] })),
    getPerf: cmd('perf_get', (id) => ({ id })),

    listWorlds: cmd('worlds_list', (id) => ({ id })),
    openWorld: cmd('worlds_open', (id, name) => ({ id, name: String(name) })),
    readLog: cmd('logs_read', (id) => ({ id })),

    socialSync: cmd('social_sync', (after) => ({ after: Number(after) || 0 })),
    socialAction: cmd('social_action', (action, name) => ({ action: String(action), name: String(name) })),
    socialSend: cmd('social_send', (to, text) => ({ to: String(to), text: String(text) })),

    getSkin: cmd('skin_get', (query, opts) => ({ query: String(query || ''), force: !!(opts && opts.force) })),
    getMySkin: cmd('skin_mine', (opts) => ({ force: !!(opts && opts.force) })),
    exportDebug: cmd('debug_export'),

    checkUpdates: cmd('update_check'),
    holdUpdate: cmd('update_hold'),
    startUpdate: cmd('update_start'),

    onStatus: (cb) => subscribe('game:status', cb),
    onLog: (cb) => subscribe('game:log', cb),
    onMaximized: (cb) => subscribe('win:maximized', cb),
    onUpdate: (cb) => subscribe('update:status', cb),
    onProfileProgress: (cb) => subscribe('profile:progress', cb)
  };

  function regionAt(el) {
    for (; el && el.nodeType === 1; el = el.parentElement) {
      const v = getComputedStyle(el).getPropertyValue('-webkit-app-region');
      if (v === 'no-drag') return 'no-drag';
      if (v === 'drag') return 'drag';
    }
    return 'none';
  }
  document.addEventListener('mousedown', (e) => {
    if (e.button !== 0 || e.detail > 1) return;
    if (regionAt(e.target) === 'drag') invoke('win_drag').catch(() => {});
  });
  document.addEventListener('dblclick', (e) => {
    if (regionAt(e.target) === 'drag') invoke('win_maximize').catch(() => {});
  });

  document.addEventListener('click', (e) => {
    const a = e.target.closest && e.target.closest('a[href]');
    if (!a) return;
    const href = a.getAttribute('href') || '';
    if (/^https:\/\//i.test(href)) {
      e.preventDefault();
      invoke('shell_external', { url: href }).catch(() => {});
    }
  }, true);

  let reported = 0;
  const report = (kind, detail) => {
    if (reported >= 20) return;
    reported += 1;
    invoke('report_error', { kind, detail: String(detail) }).catch(() => {});
  };
  window.addEventListener('error', (e) => {
    const where = `${e.filename || ''}:${e.lineno || 0}:${e.colno || 0}`;
    report('error de la interfaz', `${e.message} (${where})${e.error && e.error.stack ? `\n${e.error.stack}` : ''}`);
  });
  window.addEventListener('unhandledrejection', (e) => {
    report('promesa rechazada en la interfaz', (e.reason && e.reason.stack) || msg(e.reason));
  });
})();
