/* The sole bridge to Rust. No HTTP/server, filesystem or Python fallback. */
function native() {
  const api = window.__TAURI__?.core;
  if (!api?.invoke) throw new Error('Versora must be opened as its desktop application. The native backend is not available.');
  return api;
}
const text = (value, name, max = 4096) => {
  if (typeof value !== 'string' || value.length > max || value.includes('\0')) throw new TypeError(`Invalid ${name}`);
  return value;
};
const id = value => text(value, 'identifier', 128);
const object = value => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new TypeError('Expected an object');
  return value;
};
function nativeWindow() {
  const get = window.__TAURI__?.window?.getCurrentWindow;
  if (typeof get !== 'function') throw new Error('The native window controls are unavailable.');
  const current = get();
  if (current.label !== 'main') throw new Error('Only the Versora main window may use these controls.');
  return current;
}
async function call(command, args = {}) { return native().invoke(command, args); }
export const backend = {
  window: {
    async state() {
      const current = nativeWindow();
      const [maximized, focused] = await Promise.all([current.isMaximized(), current.isFocused()]);
      if (typeof maximized !== 'boolean' || typeof focused !== 'boolean') throw new TypeError('Invalid native window state.');
      return {maximized, focused};
    },
    minimize: () => nativeWindow().minimize(),
    maximize: () => nativeWindow().toggleMaximize(),
    // close() emits Rust CloseRequested; destroy()/exit() would bypass its guards.
    close: () => nativeWindow().close(),
    resize: direction => {
      if (!['North','East','South','West','NorthWest','NorthEast','SouthWest','SouthEast'].includes(direction)) throw new TypeError('Invalid resize direction.');
      return nativeWindow().startResizeDragging(direction);
    },
    async listen(handler) {
      if (typeof handler !== 'function') throw new TypeError('Invalid window event handler.');
      const current = nativeWindow();
      const unlisten = await Promise.all([current.onResized(() => handler()), current.onFocusChanged(() => handler())]);
      return () => unlisten.forEach(remove => remove());
    },
  },
  state: () => call('get_state'),
  pick: kind => {
    if (!['files', 'folder', 'zip'].includes(kind)) throw new TypeError('Invalid picker kind');
    return call('pick_files', {kind});
  },
  clearSelection: () => call('clear_selection'),
  chooseOutput: () => call('choose_output_dir'),
  settings: settings => call('save_settings', {settings: object(settings)}),
  unsaved: unsaved => call('set_unsaved', {unsaved: Boolean(unsaved)}),
  saveProvider: provider => {
    object(provider); id(provider.id);
    for (const name of ['apiKey', 'model', 'effort', 'baseUrl', 'cliPath']) if (provider[name] != null) text(provider[name], name, name === 'apiKey' ? 16384 : 4096);
    return call('save_provider', {provider});
  },
  deleteProvider: value => call('delete_provider', {id: id(value)}),
  probeProvider: value => call('probe_provider', {id: id(value)}),
  testProvider: value => call('test_provider', {id: id(value)}),
  createProject: name => call('create_project', {name: text(name, 'project name', 128)}),
  glossary: project => call('load_glossary', {project: id(project)}),
  saveGlossary: (project, entries) => {
    if (!Array.isArray(entries) || entries.length > 10000) throw new TypeError('Invalid glossary entries');
    for (const entry of entries) { object(entry); text(entry.term, 'term', 8192); text(entry.translation, 'translation', 8192); }
    return call('save_glossary', {project: id(project), entries});
  },
  savePurpose: instructions => call('save_purpose', {purpose: {id: 'custom', instructions: text(instructions, 'instructions', 131072)}}),
  importLegacy: () => call('import_legacy_data'),
  start: request => { object(request); return call('start_translation', {request}); },
  job: jobId => call('get_job', {jobId: id(jobId)}),
  cancel: jobId => call('cancel_job', {jobId: id(jobId)}),
  retry: jobId => call('retry_job', {jobId: id(jobId)}),
  openPath: path => call('open_path', {path: text(path, 'path', 32768)}),
  exportResult: (jobId, zip) => call('export_result', {jobId: id(jobId), zip: Boolean(zip)}),
  preview: (jobId = null, input = null) => call('get_preview', {jobId: jobId == null ? null : id(jobId), input: input == null ? null : text(input, 'path', 32768)}),
  async listenJob(handler) {
    if (!window.__TAURI__?.event?.listen) return () => {};
    return window.__TAURI__.event.listen('job-progress', event => handler(object(event.payload)));
  },
  async listenSelection(handler) {
    if (!window.__TAURI__?.event?.listen) return () => {};
    return window.__TAURI__.event.listen('selection-changed', () => handler());
  },
  updates: {
    preferences: () => call('updates_get_preferences'),
    savePreferences: preferences => call('updates_set_preferences', {preferences: object(preferences)}),
    state: () => call('updates_get_state'),
    check: () => call('updates_check'),
    download: () => call('updates_download'),
    cancel: () => call('updates_cancel'),
    later: () => call('updates_later'),
    requestInstall: workSnapshot => call('updates_request_install', {workSnapshot: object(workSnapshot)}),
    officialRelease: () => call('updates_open_official_release'),
    healthAck: () => call('updates_health_ack'),
    async listen(handler) {
      if (!window.__TAURI__?.event?.listen) return () => {};
      return window.__TAURI__.event.listen('suite-update-state-changed',event => handler(object(event.payload)));
    },
  },
};
