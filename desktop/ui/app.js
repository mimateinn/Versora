import {backend} from './backend.js';
import {icon} from './icons.js';
import {languages,nativeLanguageNames,setLanguage,systemLanguage,t,translateDocument} from './i18n.js';
import {UpdatesPane} from './updates.js';
import {WindowChrome} from './window-chrome.js';
import {UIMotion} from './motion.js';
import {bindHoloField} from './holo-field.js';

const content = document.querySelector('#content');
const motion = new UIMotion();
const initialCliProbes = new Set();
const state = {settings:{},providers:[],purposes:[],projects:[],glossary:[],selected:[],job:null,version:'',dataDir:'',testMode:false};
const view = {page:'translate',pane:'purposes',editingProvider:null,outputDir:null,busy:false,error:null,glossaryDraft:null,orderDraft:null,purposeDraft:null,purposeFrom:'general',pollTimer:null,polling:false,unlisten:[],toastTimer:null,preview:null};
const updates = new UpdatesPane(()=>{if(view.page==='settings'&&view.pane==='updates')render();},()=>{captureDrafts();view.page='settings';view.pane='updates';render();});
// Preserve the live input nodes across navigation. Secret values are never serialized
// into application state or copied into a draft string.
const transientProviderInputs = new Map();
view.projectDraft = '';
const escape = value => String(value ?? '').replace(/[&<>"']/g, char => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
const L = (key, vars) => escape(t(key, vars));
const attr = escape;
// Extended Windows paths remain intact for native commands; show their familiar form.
const displayPath = value => String(value ?? '').replace(/^\\\\\?\\UNC\\/i,'\\\\').replace(/^\\\\\?\\/,'');
const count = value => Number.isFinite(Number(value)) ? Math.max(0,Math.trunc(Number(value))) : 0;
const formatSize = value => {
  const n = Number(value) || 0;
  return n < 1024 ? `${n} B` : n < 1048576 ? `${(n/1024).toFixed(1)} KB` : `${(n/1048576).toFixed(1)} MB`;
};
const button = (action,label,testid,options={}) => `<button type="button" data-action="${attr(action)}" data-testid="${attr(testid)}" ${options.id ? `data-id="${attr(options.id)}"` : ''} class="${options.class || ''}" ${options.disabled ? 'disabled' : ''} ${options.title ? `title="${attr(options.title)}" aria-label="${attr(options.title)}"` : ''}>${options.icon ? icon(options.icon) : ''}${label ? escape(label) : ''}</button>`;
const field = (name,label,body) => `<div class="field"><label for="${attr(name)}">${label}</label>${body}</div>`;
const input = (name,value='',options={}) => `<input id="${attr(name)}" data-testid="${attr(name)}" ${options.setting ? `data-setting="${attr(options.setting)}"` : ''} name="${attr(name)}" type="${options.type || 'text'}" value="${attr(value)}" ${options.readOnly ? 'readonly' : ''} ${options.disabled ? 'disabled' : ''} ${options.placeholder ? `placeholder="${attr(options.placeholder)}"` : ''} ${options.maxLength ? `maxlength="${options.maxLength}"` : ''} ${options.autoComplete ? `autocomplete="${options.autoComplete}"` : ''}>`;
const select = (name,value,options,extra={}) => `<select id="${attr(name)}" name="${attr(name)}" data-testid="${attr(name)}" ${extra.setting ? `data-setting="${attr(extra.setting)}"` : ''} ${extra.disabled ? 'disabled' : ''}>${options.map(option => `<option value="${attr(option.value)}" ${String(option.value) === String(value) ? 'selected' : ''}>${escape(option.label)}</option>`).join('')}</select>`;
const activeJob = () => ['running','cancelling'].includes(state.job?.status);
const controlsBusy = () => view.busy || activeJob();
function effectiveSourceType() {
  if(state.selected.length){const kind=state.selected[0].kind;return ['folder','zip'].includes(kind)?kind:'file';}
  return state.settings.source_type || 'file';
}
function providersInOrder(includeDraft=true) {
  const chain = (includeDraft && view.orderDraft) || (Array.isArray(state.settings.chain) ? state.settings.chain : []);
  const byId = new Map(state.providers.map(provider => [provider.id,provider]));
  const rows = chain.map(row => ({...byId.get(row.id),...row})).filter(row => row.name);
  for (const provider of state.providers) if (!rows.some(row => row.id === provider.id)) rows.push(provider);
  return rows;
}
function languageOptions(includeAuto=false,includeOther=false) {
  return [...(includeAuto ? [{value:'auto',label:t('sidebar.source_auto')}] : []),...languages.map(code => ({value:code,label:t(`target.${code}`)})),...(includeOther ? [{value:'other',label:t('target.other')}] : [])];
}
function providerOptions() {
  return [{value:'auto',label:t('quick.auto_try')},...providersInOrder().filter(provider => provider.id !== 'demo' || state.testMode).map(provider => ({value:provider.id,label:provider.name}))];
}
function purposeOptions(custom=true) {
  return state.purposes.filter(purpose => custom || purpose.id !== 'custom').map(purpose => ({value:purpose.id,label:t(`purpose.${purpose.id}`)}));
}
function toast(message,tone='success') {
  const box = document.querySelector('#toast');
  clearTimeout(view.toastTimer); box.textContent = String(message); box.title = String(message); box.dataset.tone = tone; box.hidden = false;
  view.toastTimer = setTimeout(() => {box.hidden = true;},6000);
}
async function confirmRemoval(name) {
  const dialog = document.querySelector('#confirm-dialog');
  document.querySelector('#confirm-title').textContent = name;
  document.querySelector('#confirm-message').textContent = t('svc.remove_ask');
  translateDocument(dialog);
  dialog.returnValue = 'cancel';
  return new Promise(resolve => {dialog.addEventListener('close',() => resolve(dialog.returnValue === 'confirm'),{once:true});dialog.showModal();});
}
// Two modes share one light/dark tone: Standard (Ivory or Dusk) and Opal (hologram).
// Pressing the active mode again flips the tone; switching modes keeps it.
const modes = ['standard','hologram'];
const themeMode = () => state.settings.theme === 'hologram' ? 'hologram' : 'standard';
const themeTone = () => (themeMode() === 'hologram' ? state.settings.hologram_tone : state.settings.theme) === 'dark' ? 'dark' : 'light';
const resolvedTheme = () => themeMode() === 'hologram' ? 'hologram' : themeTone();
const themePatch = (mode,tone) => mode === 'hologram' ? {theme:'hologram',hologram_tone:tone} : {theme:tone};
const modeTitle = id => `${t(`theme.name.${id}`)} · ${t(themeTone() === 'dark' ? 'theme.dark' : 'theme.light')} · ${t('theme.flip_hint')}`;
const themeChoice = (id,testid,action,lead='') => `<button type="button" class="theme-choice" data-action="${action}" data-id="${id}" data-testid="${attr(testid)}" aria-pressed="false">${lead}${lead ? '' : `<span class="mode-mark" aria-hidden="true">${icon('sun')}${icon('moon')}</span>`}<span class="theme-name"></span></button>`;
function paintThemeChoices(group,titled=false) {
  if (!group) return;
  const mode = themeMode(),tone = themeTone(),busy = controlsBusy();
  group.dataset.value = resolvedTheme();
  for (const node of group.querySelectorAll('.theme-choice')) {
    const id = node.dataset.id;
    node.setAttribute('aria-pressed',String(id === mode));
    node.dataset.tone = tone;
    if (titled) {const title = id === mode ? modeTitle(id) : t(`theme.name.${id}`);node.title = title;node.setAttribute('aria-label',title);}
    node.querySelector('.theme-name').textContent = t(`theme.name.${id}`);
    node.disabled = busy;
  }
}
function paintToneChoices(group) {
  if (!group) return;
  const tone = themeTone(),busy = controlsBusy();
  group.dataset.value = tone;
  for (const node of group.querySelectorAll('button')) {
    const active = node.dataset.id === tone;
    node.classList.toggle('active',active);node.setAttribute('aria-pressed',String(active));node.disabled = busy;
    node.querySelector('.tone-name').textContent = t(`theme.${node.dataset.id}`);
  }
}
// Segmented controls carry one sliding pill; its last position survives a re-render so it glides.
const indicatorMemory = new Map();
function placeIndicators() {
  const still = state.settings.reduced_motion || motion.reduced.matches;
  for (const group of content.querySelectorAll('.segments')) {
    const active = group.querySelector('button.active'),key = group.dataset.testid;
    if (!active) {delete group.dataset.ind;continue;}
    const next = {x:active.offsetLeft,y:active.offsetTop,w:active.offsetWidth,h:active.offsetHeight};
    const apply = pos => {group.style.setProperty('--ind-x',`${pos.x}px`);group.style.setProperty('--ind-y',`${pos.y}px`);group.style.setProperty('--ind-w',`${pos.w}px`);group.style.setProperty('--ind-h',`${pos.h}px`);};
    const previous = indicatorMemory.get(key);
    if (!group.dataset.ind && previous && !still && (previous.x !== next.x || previous.w !== next.w)) {apply(previous);group.dataset.ind = 'init';void group.offsetWidth;}
    if (!group.dataset.ind) group.dataset.ind = 'init';
    if (!still) group.dataset.ind = 'live';
    apply(next);indicatorMemory.set(key,next);
  }
}
const windowChrome = new WindowChrome(message => toast(message,'error'),async()=>{captureDrafts();await backend.unsaved(workSnapshot().unsaved);});
function renderChrome() {
  windowChrome.render();
  motion.theme(resolvedTheme(),state.settings.reduced_motion,themeTone());
  translateDocument();
  const strip = document.querySelector('.theme-switcher');
  if (!strip.children.length) strip.innerHTML = modes.map(id => themeChoice(id,`theme-choice-${id}`,'set-theme')).join('');
  strip.setAttribute('aria-label',t('card.theme'));
  paintThemeChoices(strip,true);
  paintThemeChoices(content.querySelector('#appearance-theme'));
  paintToneChoices(content.querySelector('#appearance-tone'));
  document.querySelector('#route-name').textContent = t(view.page === 'settings' ? 'nav.settings' : 'nav.translate');
  document.querySelectorAll('.nav-button').forEach(node => {node.title = t(`nav.${node.dataset.page}`);node.classList.toggle('active',node.dataset.page === view.page);node.setAttribute('aria-current',node.dataset.page === view.page ? 'page' : 'false');});
  document.querySelector('#version').textContent = state.version ? `v${state.version.replace(/^v/,'')}` : '';
}
const dropArt = `<svg viewBox="0 0 168 96" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 86h140" stroke-opacity=".5"/><rect x="40" y="14" width="50" height="66" rx="4" fill="var(--art-fill)"/><path d="M50 30h30M50 40h30M50 50h22M50 60h26" stroke-opacity=".6"/><rect x="80" y="22" width="50" height="62" rx="4" fill="var(--art-fill)" stroke="var(--accent)" stroke-dasharray="4 4"/><path d="M90 38h28M90 48h24M90 58h28M64 6c10-4 22-2 30 6M90 6l4 6-7 1" stroke="var(--accent)"/></svg>`;
function quickbar() {
  const s = state.settings,busy = controlsBusy();
  return `<div class="quickbar" data-testid="quickbar">${field('source-language',L('quick.from'),select('source-language',s.source_choice || 'auto',languageOptions(true),{setting:'source_choice',disabled:busy}))}${button('swap',null,'swap-languages',{icon:'swap',class:'icon-button',title:t('quick.swap'),disabled:busy || (s.source_choice || 'auto') === 'auto' || s.target_lang === 'other'})}${field('target-language',L('quick.to'),select('target-language',s.target_lang || 'en',languageOptions(false,true),{setting:'target_lang',disabled:busy}))}${field('purpose',L('quick.purpose')+(s.purpose === 'custom' ? ` · <button type="button" data-action="manage-purpose" data-testid="manage-custom-purpose">${L('purpose.edit')}</button>` : ''),select('purpose',s.purpose || 'general',purposeOptions(),{setting:'purpose',disabled:busy}))}${field('translator',`${L('quick.translator')} · <button type="button" data-action="manage-providers" data-testid="manage-translators">${L('quick.manage')}</button>`,select('translator',s.provider || 'auto',providerOptions(),{setting:'provider',disabled:busy}))}</div>${s.target_lang === 'other' ? `<div class="inline-fields" style="margin-top:16px">${field('target-other',L('sidebar.target_other'),input('target-other',s.target_other || '',{setting:'target_other',maxLength:40,disabled:busy}))}</div>` : ''}${s.provider === 'demo' ? `<div class="notice" data-testid="demo-warning">${icon('alert')}${L('desktop.demo')}</div>` : ''}`;
}
function selectedView() {
  const busy = controlsBusy();
  return `<div class="selected-files" data-testid="selected-files">${state.selected.map((file,index) => `<div class="file-row" data-testid="selected-file-${index}">${icon(file.kind === 'folder' ? 'folder' : file.kind === 'zip' ? 'zip' : 'file')}<div class="file-detail"><div class="file-name">${escape(file.name)}</div><div class="file-path" title="${attr(displayPath(file.path))}">${escape(displayPath(file.path))}</div></div><span class="file-size">${formatSize(file.size)}</span></div>`).join('')}</div><div class="run-actions"><span class="hint">${L('desktop.originals')}</span><div class="actions">${state.selected.length === 1 && state.selected[0].kind !== 'folder' ? button('source-preview',t('result.show_preview'),'source-preview',{icon:'file',class:'small',disabled:busy}) : ''}${button('clear-selection',t('main.clear'),'clear-selection',{icon:'close',class:'small',disabled:busy})}${button('pick',t('drop.browse'),'pick-more',{icon:'plus',class:'small',disabled:busy})}</div></div>${view.preview && !state.job ? previewView() : ''}`;
}
function previewView() {
  const preview=view.preview;
  if (!preview) return '';
  return preview.binary ? `<p class="notice" data-testid="binary-preview">${L('main.binary_preview')}</p>` : `<div class="preview-columns" data-testid="preview-columns"><div class="preview-block"><p class="eyebrow">${L('main.preview_src')}</p><pre data-testid="source-text-preview">${escape(preview.source || t('main.preview_empty'))}</pre></div>${preview.translated != null ? `<div class="preview-block"><p class="eyebrow">${L('main.preview_out')}</p><pre data-testid="translated-text-preview">${escape(preview.translated || t('main.preview_empty'))}</pre></div>` : ''}</div>`;
}
function translateView() {
  const busy = controlsBusy(),kind = effectiveSourceType();
  return `<section class="card card-enter" data-testid="translate-card">${quickbar()}<hr class="rule"><div class="segments" data-testid="source-types" role="group" aria-label="${attr(t('main.source_type'))}">${[['file','main.seg_file','file'],['folder','main.seg_folder','folder'],['zip','main.seg_zip','zip']].map(([type,label,glyph]) => `<button type="button" data-action="source-type" data-id="${type}" data-testid="source-type-${type}" aria-pressed="${type === kind}" class="${type === kind ? 'active' : ''}" ${busy ? 'disabled' : ''}>${icon(glyph)}${L(label)}</button>`).join('')}</div>${state.selected.length ? selectedView() : `<div class="dropzone" data-testid="dropzone"><div class="drop-art">${dropArt}</div><div class="drop-copy"><h3>${L(kind === 'folder' ? 'main.source_folder' : kind === 'zip' ? 'drop.zip_title' : 'drop.title')}</h3><p class="muted">${L(kind === 'zip' ? 'drop.zip_hint' : kind === 'folder' ? 'main.folder_hint' : 'drop.hint')}</p><p class="formats">${kind === 'zip' ? 'ZIP' : 'TXT · MD · DOCX · PDF · JSON · CSV · YAML · PO · XLIFF · XLSX · HTML · SRT · VTT'}</p>${button('pick',t(kind === 'folder' ? 'desktop.choose_folder' : 'drop.browse'),'pick-files',{icon:kind === 'folder' ? 'folder_open' : 'upload',class:'primary',disabled:busy})}</div></div>`}${state.selected.length ? `<hr class="divider"><div class="picker-line">${field('output-folder',L('desktop.output'),input('output-folder',displayPath(view.outputDir),{readOnly:true,placeholder:t('desktop.output_hint')}))}${button('choose-output',t('desktop.choose_folder'),'choose-output',{icon:'folder_open',disabled:busy})}</div><div class="run-actions"><span class="hint" data-testid="selection-count">${L('desktop.selected')}: ${state.selected.length}</span>${button('translate',t('main.translate_btn'),'translate-start',{icon:'swap',class:'primary',disabled:busy})}</div>` : ''}${view.error ? `<div class="notice error" role="alert" data-testid="translation-error">${icon('alert')}${escape(view.error)}</div>` : ''}</section>${state.job ? jobView(state.job) : ''}`;
}
function jobView(job) {
  const running = ['running','cancelling'].includes(job.status);
  const failed=count(job.failed),saved=count(job.saved),skipped=count(job.skipped);
  const files = Array.isArray(job.files) ? job.files : [];
  const unfinished = files.filter(file => ['cancelled','pending'].includes(file.status)).length;
  const label = job.status==='done' && job.usedDemo===true ? t('desktop.demo_completed') : running ? t(job.status === 'cancelling' ? 'run.cancelling' : 'state.running') : t(job.status === 'stopped' ? 'done.stopped' : job.status === 'error' ? 'main.failed_title' : failed ? 'done.batch_some' : files.length > 1 || effectiveSourceType() !== 'file' ? 'done.batch' : 'done.file');
  const error = job.status === 'error';
  const states = {pending:'state.waiting',running:'state.running',saved:'batch.saved',failed:'batch.failed',skipped:'batch.skipped',cancelled:'batch.unfinished'};
  const glyphs = {pending:'dot',running:'spinner',saved:'check',failed:'alert',skipped:'file',cancelled:'stop'};
  const chunksTotal=count(job.chunksTotal),chunksDone=Math.min(chunksTotal,count(job.chunksDone));
  const chunkProgress=count(job.total)<=1&&chunksTotal>0;
  const progressTotal=chunkProgress?chunksTotal:Math.max(1,count(job.total));
  const progressDone=chunkProgress?chunksDone:count(job.done);
  const progressLabel=t(chunkProgress?'run.chunks':'run.files',{done:progressDone,total:progressTotal});
  const fileRows = files.map((file,index) => {
    const total=count(file.chunks_total),done=Math.min(total,count(file.chunks_done));
    return `<div class="job-file" data-testid="job-file-${index}" data-status="${attr(file.status)}">${icon(glyphs[file.status] || 'file')}<span class="job-file-title" title="${attr(displayPath(file.relative || file.input))}">${escape(file.relative || file.name)}</span><span class="job-file-state">${L(states[file.status] || 'state.waiting')}${total ? `<span class="file-chunks" data-testid="job-file-chunks-${index}">${L('run.chunks',{done,total})}</span>` : ''}</span>${file.error ? `<div class="job-error">${escape(file.error)}</div>` : ''}</div>`;
  }).join('');
  return `<section class="card card-enter" data-testid="job-card" data-job-status="${attr(job.status)}" data-used-demo="${job.usedDemo===true}" aria-live="polite">
    <div class="${running ? 'progress-heading' : 'result-heading'} ${error ? 'error' : ''}">
      ${icon(running ? 'spinner' : error ? 'alert' : job.status === 'stopped' ? 'stop' : 'check')}
      <div class="progress-title"><h2 data-testid="job-title">${escape(label)}</h2><span class="progress-meta" data-testid="job-progress-label">${L('run.files',{done:count(job.done),total:count(job.total)})}${job.current ? ` · ${escape(job.current)}` : ''}</span>${chunksTotal ? `<span class="progress-meta chunk-progress" data-testid="job-chunk-label">${L('run.chunks',{done:chunksDone,total:chunksTotal})}</span>` : ''}</div>
      ${running ? button('cancel',t(job.status === 'cancelling' ? 'run.cancelling' : 'run.cancel'),'job-cancel',{class:'small',icon:'stop',disabled:job.status === 'cancelling' || view.busy}) : ''}
    </div>
    ${running ? `<progress data-testid="job-progress" data-unit="${chunkProgress?'chunks':'files'}" max="${progressTotal}" value="${progressDone}" aria-label="${attr(progressLabel)}"></progress>` : `<div class="counts" data-testid="job-counts"><span class="saved">${L('done.n_saved',{n:saved})}</span>${failed ? `<span class="failed">${L('done.n_failed',{n:failed})}</span>` : ''}${skipped ? `<span>${L('done.n_skipped',{n:skipped})}</span>` : ''}${unfinished ? `<span>${L('done.n_unfinished',{n:unfinished})}</span>` : ''}</div>`}
    ${job.usedDemo===true ? `<p class="notice" data-testid="job-demo-warning">${L('desktop.demo')}</p>` : ''}
    ${job.message ? `<p class="notice ${error ? 'error' : ''}" data-testid="job-message">${escape(job.message)}</p>` : ''}
    <div class="job-files" data-testid="job-files">${fileRows}</div>
    ${!running && job.outputDir && saved ? `<div class="output-path" data-testid="output-path" title="${attr(displayPath(job.outputDir))}">${escape(displayPath(job.outputDir))}</div>${button('copy-output-path',t('result.copy_path'),'copy-output-path',{icon:'file',class:'link',disabled:view.busy})}` : ''}
    ${!running && saved ? `${button('result-preview',t('result.show_preview'),'result-preview',{icon:'file',class:'link',disabled:view.busy})}${view.preview ? previewView() : ''}` : ''}
    ${!running ? `<div class="actions spread">${failed + unfinished ? button('retry',t('batch.retry_remaining',{n:failed+unfinished}),'job-retry',{icon:'retry',disabled:view.busy}) : '<span></span>'}<div class="actions">${saved && job.outputDir ? button('open-output',t('batch.open_folder'),'open-output',{icon:'folder_open',disabled:view.busy}) : ''}${saved ? button(files.length > 1 || effectiveSourceType() !== 'file' ? 'export-zip' : 'export-file',t(files.length > 1 || effectiveSourceType() !== 'file' ? 'batch.download_all' : 'desktop.save_as'),'export-result',{icon:'download',class:'primary',disabled:view.busy}) : ''}</div></div>` : ''}
  </section>`;
}
function clearToast() {
  clearTimeout(view.toastTimer);
  const box=document.querySelector('#toast');box.textContent='';box.title='';box.hidden=true;
}
const panes = [['purposes','card.purposes','bubble'],['keys','card.translators','key'],['order','card.order','swap'],['glossary','card.glossary','book'],['appearance','card.appearance','monitor'],['updates','updates.title','download']];
function sameRows(left,right,fields) {
  if(!Array.isArray(left)||!Array.isArray(right)||left.length!==right.length)return false;
  return left.every((row,index)=>fields.every(field=>field==='enabled'?(row[field]!==false)===(right[index][field]!==false):String(row[field]??'')===String(right[index][field]??'')));
}
function workSnapshot() {
  const currentPurpose=state.purposes.find(purpose=>purpose.id==='custom') || state.purposes.find(purpose=>purpose.id==='general');
  const unsaved=Boolean(view.purposeDraft!=null&&view.purposeDraft!==currentPurpose?.instructions)||Boolean(view.glossaryDraft&&!sameRows(view.glossaryDraft,state.glossary,['term','translation']))||Boolean(view.orderDraft&&!sameRows(view.orderDraft,providersInOrder(false),['id','model','effort','enabled']));
  rememberProviderInputs();
  let providerUnsaved=false;
  for(const [name,node] of transientProviderInputs){const [,kind,id]=name.match(/^provider-(key|base|path)-(.+)$/);const provider=state.providers.find(provider=>provider.id===id);if(kind==='key'?Boolean(node.value):node.value!==(provider?.[kind==='base'?'baseUrl':'cliPath'] || ''))providerUnsaved=true;}
  return {busy:activeJob(),unsaved:unsaved||providerUnsaved||Boolean(view.projectDraft),pendingPersistence:false};
}
function rememberProviderInputs() {
  content.querySelectorAll('[data-form="provider"] input').forEach(node=>{if(/^provider-(key|base|path)-/.test(node.id))transientProviderInputs.set(node.id,node);});
}
function restoreProviderInputs() {
  content.querySelectorAll('[data-form="provider"] input').forEach(node=>{const retained=transientProviderInputs.get(node.id);if(retained&&retained!==node){retained.disabled=node.disabled;node.replaceWith(retained);}});
}
function clearProviderInputs(id) {
  for(const [name,node] of transientProviderInputs)if(name.endsWith(`-${id}`)){node.value='';node.remove();transientProviderInputs.delete(name);}
}
function reportUnsaved() {
  const unsaved=workSnapshot().unsaved;
  if(view.lastUnsaved===unsaved)return;
  view.lastUnsaved=unsaved;
  backend.unsaved(unsaved).catch(error=>{view.lastUnsaved=undefined;toast(String(error?.message||error),'error');});
}
function settingsView() {
  return `<div class="settings-layout" data-testid="settings-layout"><nav class="settings-rail" aria-label="${attr(t('nav.settings'))}">${panes.map(([pane,label,glyph]) => `<button type="button" data-action="settings-pane" data-id="${pane}" data-testid="settings-pane-${pane}" class="rail-button ${view.pane === pane ? 'active' : ''}" aria-current="${view.pane === pane ? 'page' : 'false'}">${icon(glyph)}${L(label)}</button>`).join('')}</nav><section class="settings-pane" data-testid="settings-content-${attr(view.pane)}"><h1>${L(panes.find(([pane]) => pane === view.pane)?.[1] || 'nav.settings')}</h1>${view.pane==='updates'?updates.render(controlsBusy(),workSnapshot()):({purposes:purposesView,keys:providersView,order:orderView,glossary:glossaryView,appearance:appearanceView}[view.pane] || purposesView)()}</section></div>`;
}
function purposesView() {
  const purpose = state.purposes.find(purpose => purpose.id === 'custom') || state.purposes.find(purpose => purpose.id === 'general') || {instructions:'',version:0};
  return `<form class="card pane-card" data-form="purpose" data-testid="purpose-form"><p class="note">${L('purpose.hint')}</p><div class="picker-line">${field('purpose-from',L('purpose.start_from'),select('purpose-from',view.purposeFrom,purposeOptions(false),{disabled:controlsBusy()}))}${button('copy-purpose',t('purpose.copy'),'copy-purpose',{disabled:controlsBusy()})}</div>${field('purpose-instructions',L('purpose.instructions'),`<textarea id="purpose-instructions" name="purpose-instructions" data-testid="purpose-instructions" maxlength="131072" ${controlsBusy() ? 'disabled' : ''}>${escape(view.purposeDraft ?? purpose.instructions)}</textarea>`)}<div class="actions spread"><span class="hint" data-testid="purpose-version">${L('purpose.version',{v:purpose.version || 0})}</span><button type="submit" class="primary" data-testid="save-purpose" ${controlsBusy() ? 'disabled' : ''}>${L('purpose.save')}</button></div></form>`;
}
function providerStatus(provider) {
  if(provider.id==='demo')return t('desktop.demo');
  if(provider.transportVerified===true)return t('desktop.provider_connected');
  const local=provider.kind==='cli'||String(provider.id).endsWith('_cli');
  if(local){
    if(provider.probePerformed!==true)return t('desktop.provider_unprobed');
    if(provider.nativeDetected===false)return t('desktop.provider_missing');
    if(provider.signedIn===true)return t('desktop.provider_signed_in');
    if(provider.signedIn===false||['signed_out','signedOut','login_required'].includes(provider.status))return t('st.signed_out');
    return t('st.unchecked');
  }
  return provider.configured?t('desktop.provider_configured'):t('st.setup');
}
function sanitizedProviderDetail(value) {
  return String(value??'').slice(0,2048)
    .replace(/\b(?:sk-|xai-)[A-Za-z0-9_-]{8,}/g,'[redacted]')
    .replace(/(bearer\s+)[^\s"']+/ig,'$1[redacted]')
    .replace(/([?&](?:api[-_]?key|key|token)=)[^&\s]+/ig,'$1[redacted]');
}
function providerDetail(provider) {
  if(provider.probePerformed!==true||provider.availableForAttempt===true||!provider.detail)return '';
  return `<div class="provider-check-detail" data-testid="provider-detail-${attr(provider.id)}">${L('main.details')}: ${escape(sanitizedProviderDetail(provider.detail))}</div>`;
}
function providerActions(provider) {
  if(provider.id==='demo')return '';
  const busy=controlsBusy();
  const remove=button('remove-provider',t('svc.remove'),`remove-provider-${provider.id}`,{id:provider.id,class:'link',disabled:busy||!provider.keyPresent});
  return button('probe-provider',t('svc.recheck'),`probe-provider-${provider.id}`,{id:provider.id,class:'link',disabled:busy})+
    button('test-provider',t('svc.test'),`test-provider-${provider.id}`,{id:provider.id,class:'small',disabled:busy||provider.availableForAttempt!==true})+
    button('edit-provider',t(provider.configured?'svc.replace':'order.set_up'),`edit-provider-${provider.id}`,{id:provider.id,class:'link',disabled:busy})+
    (provider.keyPresent?remove:remove.replace('class="link"','class="link action-unavailable" aria-hidden="true" tabindex="-1"'));
}
function providerEdit(provider) {
  const local = provider.kind === 'cli' || provider.id.endsWith('_cli'),busy=controlsBusy();
  return `<div class="provider-edit" data-testid="provider-editor-${attr(provider.id)}"><form data-form="provider" data-id="${attr(provider.id)}" data-testid="provider-form-${attr(provider.id)}">${local ? field(`provider-path-${provider.id}`,L('order.path',{name:provider.name}),input(`provider-path-${provider.id}`,provider.cliPath || '',{maxLength:4096,disabled:busy})) : field(`provider-key-${provider.id}`,L('keys.paste_api'),input(`provider-key-${provider.id}`,'',{type:'password',maxLength:16384,autoComplete:'new-password',disabled:busy}))}${!local && ['openai','xai'].includes(provider.id) ? field(`provider-base-${provider.id}`,L('sidebar.custom_base'),input(`provider-base-${provider.id}`,provider.baseUrl || '',{type:'url',maxLength:4096,disabled:busy})) : ''}<div class="actions">${button('close-provider-editor',t('svc.keep'),`keep-provider-${provider.id}`,{class:'small',disabled:controlsBusy()})}<button type="submit" class="primary small" data-testid="save-provider-${attr(provider.id)}" ${controlsBusy() ? 'disabled' : ''}>${L('keys.save_local')}</button></div></form></div>`;
}
function providersView() {
  const groups = [['cli','svc.local'],['api','svc.online'],...(state.testMode ? [['demo','svc.builtin']] : [])];
  return `<div class="card" data-testid="providers-card">${groups.map(([kind,label]) => {
    const list = state.providers.filter(provider => kind === 'cli' ? provider.id.endsWith('_cli') : kind === 'demo' ? provider.id === 'demo' : !provider.id.endsWith('_cli') && provider.id !== 'demo');
    if (!list.length) return '';
    return `<div class="provider-group"><p class="eyebrow">${L(label)}</p>${list.map(provider => `<div class="provider-row" data-testid="provider-${attr(provider.id)}"><div><div class="provider-heading"><span class="status-dot ${provider.transportVerified===true ? 'ready' : ''}" aria-hidden="true"></span><h3>${escape(provider.name)}</h3></div><div class="provider-status" data-testid="provider-status-${attr(provider.id)}" data-probe-performed="${provider.probePerformed===true}" data-signed-in="${provider.signedIn===true}" data-transport-verified="${provider.transportVerified===true}">${escape(providerStatus(provider))}</div>${providerDetail(provider)}</div><div class="provider-actions" data-testid="provider-actions-${attr(provider.id)}">${providerActions(provider)}</div>${view.editingProvider === provider.id ? providerEdit(provider) : ''}</div>`).join('')}</div>`;
  }).join('')}<p class="key-notice">${L('desktop.keys')}</p><p class="hint" style="margin-top:8px">${L('desktop.transport_note')}</p></div>`;
}
function orderView() {
  const providers = providersInOrder(),busy = controlsBusy();
  return `<div class="card pane-card" data-testid="order-card"><p class="note">${L('order.hint')}</p><div><div class="order-header"><span>${L('order.col_name')}</span><span>${L('order.model')}</span><span class="order-effort">${L('order.effort')}</span><span>${L('order.col_on')}</span><span></span><span></span></div>${providers.map((provider,index) => `<div class="order-row" data-testid="order-row-${attr(provider.id)}"><div class="order-name"><span class="ordinal">${index+1}</span><span class="status-dot ${provider.transportVerified===true ? 'ready' : ''}" aria-hidden="true"></span><span title="${attr(provider.name)}">${escape(provider.name)}</span></div>${input(`order-model-${provider.id}`,provider.model || '',{placeholder:t('order.model_default'),maxLength:64,disabled:busy})}<div class="order-effort">${select(`order-effort-${provider.id}`,provider.effort || '',[{value:'',label:t('order.effort_default')},...['low','medium','high'].map(value => ({value,label:t(`order.${value}`)}))],{disabled:busy})}</div><label class="toggle" title="${attr(t('order.on'))}"><input type="checkbox" data-testid="order-enabled-${attr(provider.id)}" data-order="enabled" data-id="${attr(provider.id)}" aria-label="${attr(`${t('order.on')}: ${provider.name}`)}" ${provider.enabled !== false ? 'checked' : ''} ${busy ? 'disabled' : ''}><span class="toggle-track"></span></label>${button('move-up',null,`order-up-${provider.id}`,{id:provider.id,icon:'upload',class:'icon-button',title:t('order.up'),disabled:busy || index === 0})}${button('move-down',null,`order-down-${provider.id}`,{id:provider.id,icon:'download',class:'icon-button',title:t('order.down'),disabled:busy || index === providers.length-1})}</div>`).join('')}</div><hr class="divider"><div class="limits-line"><label for="global-limit">${L('order.files')}</label>${select('global-limit',state.settings.concurrency || 3,Array.from({length:16},(_,i) => ({value:i+1,label:String(i+1)})),{setting:'concurrency',disabled:busy})}</div><details class="advanced" data-testid="order-advanced"><summary data-testid="order-advanced-toggle">${L('order.advanced')}</summary><div class="advanced-content stack"><div class="limits-line"><label for="provider-limit">${L('order.per')}</label>${select('provider-limit',state.settings.per_provider || 1,Array.from({length:8},(_,i) => ({value:i+1,label:String(i+1)})),{setting:'per_provider',disabled:busy})}</div><p class="hint">${L('order.limits_hint')}</p></div></details><div class="actions">${button('save-order',t('desktop.save_settings'),'save-order',{class:'primary',disabled:busy})}</div></div>`;
}
function glossaryView() {
  const project = state.settings.project || 'default',busy = controlsBusy();
  const rows = view.glossaryDraft || state.glossary || [];
  const projects = state.projects.map(value => typeof value === 'string' ? value : value.id || value.name);
  if (!projects.includes(project)) projects.unshift(project);
  return `<div class="card pane-card" data-testid="glossary-card"><div class="inline-fields">${field('glossary-project',L('sidebar.project'),select('glossary-project',project,projects.map(value => ({value,label:value})),{disabled:busy}))}<form data-form="project" data-testid="project-form" class="picker-line">${field('new-project',L('sidebar.new_project'),input('new-project',view.projectDraft,{maxLength:128,disabled:busy}))}<button type="submit" data-testid="create-project" ${busy ? 'disabled' : ''}>${L('sidebar.create_project')}</button></form></div><form data-form="glossary" data-testid="glossary-form"><div class="glossary-labels"><span>${L('glossary.col_src')}</span><span>${L('glossary.col_dst')}</span><span></span></div><div data-testid="glossary-rows">${rows.map((row,index) => `<div class="glossary-row" data-testid="glossary-row-${index}">${input(`glossary-term-${index}`,row.term ?? row.src ?? '',{maxLength:8192,disabled:busy})}${input(`glossary-translation-${index}`,row.translation ?? row.dst ?? '',{maxLength:8192,disabled:busy})}${button('delete-term',null,`glossary-delete-${index}`,{id:String(index),icon:'close',class:'icon-button',title:t('glossary.delete'),disabled:busy})}</div>`).join('')}</div>${!rows.length ? `<p class="note" data-testid="glossary-empty">${L('glossary.empty')}</p>` : ''}<div class="actions spread">${button('add-term',t('sidebar.add_term'),'add-glossary-term',{icon:'plus',disabled:busy})}<button type="submit" class="primary" data-testid="save-glossary" ${busy ? 'disabled' : ''}>${L('sidebar.save_glossary')}</button></div></form></div>`;
}
function appearanceView() {
  const s = state.settings,busy=controlsBusy();
  return `<div class="card pane-card" data-testid="appearance-card"><div class="field"><span class="field-label" id="appearance-theme-label">${L('card.theme')}</span><div class="theme-row" role="group" id="appearance-theme" data-testid="appearance-theme" aria-labelledby="appearance-theme-label">${modes.map(id => themeChoice(id,`appearance-theme-${id}`,'set-mode','<span class="theme-dot" aria-hidden="true"></span>')).join('')}</div></div><div class="field"><span class="field-label" id="appearance-tone-label">${L('theme.tone')}</span><div class="segments tone-choices" role="group" id="appearance-tone" data-testid="appearance-tone" aria-labelledby="appearance-tone-label">${['light','dark'].map(id => `<button type="button" data-action="set-tone" data-id="${id}" data-testid="appearance-tone-${id}" aria-pressed="false">${icon(id === 'light' ? 'sun' : 'moon')}<span class="tone-name"></span></button>`).join('')}</div></div>${field('interface-language',L('sidebar.language'),select('interface-language',s.ui_lang_follow !== false ? 'system' : s.ui_lang || systemLanguage(),[{value:'system',label:t('lang.follow_system',{name:nativeLanguageNames[systemLanguage()]})},...languages.map(value => ({value,label:nativeLanguageNames[value]}))],{disabled:busy}))}<p class="note">${L('card.lang_count')} · ${L('desktop.os_language')}</p><label class="limits-line"><input type="checkbox" id="reduce-motion" data-testid="reduce-motion" data-setting="reduced_motion" ${s.reduced_motion ? 'checked' : ''} ${busy ? 'disabled' : ''}>${L('desktop.motion')}</label><hr class="divider"><p class="eyebrow">${L('desktop.data')}</p><p class="output-path" data-testid="data-directory" title="${attr(displayPath(state.dataDir))}">${escape(displayPath(state.dataDir))}</p><div class="actions spread">${button('import-legacy',t('desktop.import'),'import-legacy',{icon:'folder_open',disabled:busy})}${button('open-data',t('batch.open_folder'),'open-data',{icon:'folder',disabled:busy})}</div></div>`;
}
function render() {
  rememberProviderInputs();
  const retainedLayout=view.page==='settings'&&content.querySelector('.settings-layout');
  if(retainedLayout){
    // Keep rail controls alive so their selection marker interpolates across panes.
    const template=document.createElement('template');template.innerHTML=settingsView();
    const nextLayout=template.content.querySelector('.settings-layout');
    const rail=retainedLayout.querySelector('.settings-rail');
    rail.setAttribute('aria-label',nextLayout.querySelector('.settings-rail').getAttribute('aria-label'));
    for(const next of nextLayout.querySelectorAll('.rail-button')){
      const current=[...rail.querySelectorAll('.rail-button')].find(button=>button.dataset.id===next.dataset.id);
      if(!current)continue;
      current.className=next.className;current.setAttribute('aria-current',next.getAttribute('aria-current'));
      if(current.innerHTML!==next.innerHTML)current.innerHTML=next.innerHTML;
    }
    retainedLayout.querySelector('.settings-pane').replaceWith(nextLayout.querySelector('.settings-pane'));
  }else content.innerHTML=view.page==='settings'?settingsView():translateView();
  restoreProviderInputs();
  renderChrome();
  placeIndicators();
  motion.enter(content,view.page==='settings'?`settings:${view.pane}`:view.page);
  document.querySelector('#output-folder')?.setAttribute('title',displayPath(view.outputDir));
  content.setAttribute('aria-busy',String(view.busy));
  updates.watchPane(view.page==='settings'&&view.pane==='updates');
  reportUnsaved();
  if(view.page==='settings'&&view.pane==='keys')void recheckInitialCliProviders();
}
async function recheckInitialCliProviders() {
  if(controlsBusy())return;
  const pending=state.providers.filter(provider=>String(provider.id).endsWith('_cli')&&provider.probePerformed!==true&&!initialCliProbes.has(provider.id));
  if(!pending.length)return;
  pending.forEach(provider=>initialCliProbes.add(provider.id));
  // Native probes read installed versions/login status. They never call Test or translate.
  const results=await Promise.allSettled(pending.map(provider=>backend.probeProvider(provider.id)));
  try {
    const fresh=await backend.state();
    if(!Array.isArray(fresh?.providers))throw new Error('The desktop backend returned invalid provider state.');
    // A background availability read must not roll back a concurrent settings save.
    state.providers=fresh.providers;
    if(view.page==='settings'&&view.pane==='keys')patchProviderRows();
    const failed=results.find(result=>result.status==='rejected');
    if(failed&&view.page==='settings'&&view.pane==='keys')toast(String(failed.reason?.message||failed.reason),'error');
  }catch(error){if(view.page==='settings'&&view.pane==='keys')toast(String(error?.message||error),'error');}
}
function patchProviderRows() {
  if(view.page!=='settings'||view.pane!=='keys')return;
  for(const provider of state.providers){
    const row=[...content.querySelectorAll('.provider-row')].find(node=>node.dataset.testid===`provider-${provider.id}`);
    if(!row)continue;
    row.querySelector('.status-dot')?.classList.toggle('ready',provider.transportVerified===true);
    const status=row.querySelector('.provider-status');
    status.textContent=providerStatus(provider);
    status.dataset.probePerformed=String(provider.probePerformed===true);
    status.dataset.signedIn=String(provider.signedIn===true);
    status.dataset.transportVerified=String(provider.transportVerified===true);
    const detail=row.querySelector('.provider-check-detail'),markup=providerDetail(provider);
    if(detail){if(markup)detail.outerHTML=markup;else detail.remove();}
    else if(markup)status.insertAdjacentHTML('afterend',markup);
    const template=document.createElement('template');template.innerHTML=providerActions(provider);
    const actions=row.querySelector('.provider-actions');
    for(const next of template.content.querySelectorAll('button')){
      const current=[...actions.querySelectorAll('button')].find(node=>node.dataset.testid===next.dataset.testid);
      if(!current)continue;
      for(const attribute of [...current.attributes])if(!next.hasAttribute(attribute.name))current.removeAttribute(attribute.name);
      for(const attribute of next.attributes)current.setAttribute(attribute.name,attribute.value);
      if(current.innerHTML!==next.innerHTML)current.innerHTML=next.innerHTML;
    }
  }
}
function syncAppearanceControls() {
  const theme=document.querySelector('#appearance-theme'),reduced=document.querySelector('#reduce-motion');
  paintThemeChoices(theme);paintToneChoices(document.querySelector('#appearance-tone'));placeIndicators();
  if(reduced)reduced.checked=Boolean(state.settings.reduced_motion);
}
function captureDrafts() {
  if (view.page !== 'settings') return;
  if (view.pane === 'purposes') {const node=document.querySelector('#purpose-instructions');if(node)view.purposeDraft=node.value;}
  if (view.pane === 'glossary') view.glossaryDraft=readGlossary();
  if (view.pane === 'order') captureOrder();
}
function readGlossary() {
  return [...document.querySelectorAll('.glossary-row')].map(row => ({term:row.querySelector('input[name^="glossary-term-"]').value.trim(),translation:row.querySelector('input[name^="glossary-translation-"]').value.trim()}));
}
function captureOrder() {
  view.orderDraft = providersInOrder().map(provider => ({id:provider.id,model:document.querySelector(`[data-testid="order-model-${provider.id}"]`)?.value.trim() ?? provider.model ?? '',effort:document.querySelector(`[data-testid="order-effort-${provider.id}"]`)?.value ?? provider.effort ?? '',enabled:document.querySelector(`[data-testid="order-enabled-${provider.id}"]`)?.checked ?? provider.enabled !== false}));
}
async function saveSettings(patch) {
  const updated = await backend.settings(patch);
  state.settings = {...state.settings,...(updated?.settings || updated || patch)};
  if (patch.ui_lang || patch.ui_lang_follow !== undefined) await setLanguage(state.settings.ui_lang_follow !== false ? systemLanguage() : state.settings.ui_lang);
}
async function refresh() {
  const fresh = await backend.state();
  if (!fresh || typeof fresh !== 'object') throw new Error('The desktop backend returned invalid state.');
  Object.assign(state,fresh);
  for (const field of ['providers','purposes','projects','glossary','selected']) if (!Array.isArray(state[field])) state[field] = [];
  state.settings ||= {};
  await setLanguage(state.settings.ui_lang_follow !== false ? systemLanguage() : state.settings.ui_lang);
}
function receiveJob(job) {
  if (!job?.id || (state.job?.id && state.job.id !== job.id)) return;
  const wasActive=activeJob();
  if(state.job && !wasActive && ['running','cancelling'].includes(job.status))return;
  state.job = job;
  if (view.page === 'translate') {
    const card = content.querySelector('[data-testid="job-card"]');
    if (card) card.outerHTML = jobView(job); else content.insertAdjacentHTML('beforeend',jobView(job));
    // Synchronize controls on terminal transition without replacing an edited settings form.
    if (!activeJob()) clearTimeout(view.pollTimer);
  }
  if (!activeJob()) {clearTimeout(view.pollTimer);if(wasActive){captureDrafts();render();}}
}
async function pollJob() {
  clearTimeout(view.pollTimer);
  if (!activeJob() || view.polling) return;
  view.polling = true;
  try {receiveJob(await backend.job(state.job.id));} catch(error) {view.error = String(error);if(view.page==='translate')render();}
  finally {view.polling=false;if(activeJob())view.pollTimer=setTimeout(pollJob,200);}
}
async function loadJob(result) {
  const jobId = typeof result === 'string' ? result : result?.jobId || result?.id;
  if (!jobId) throw new Error('The backend did not return a job identifier.');
  state.job = await backend.job(jobId);
  view.page = 'translate';view.error=null;view.preview=null;
  render();pollJob();
}
async function runAction(action,node) {
  if(['navigate','settings-pane','manage-providers','manage-purpose','source-type','translate','retry'].includes(action))clearToast();
  if(['navigate','settings-pane','manage-providers','manage-purpose'].includes(action))document.querySelector('.app-shell').scrollTop=0;
  const id=node.dataset.id;
  if(action.startsWith('updates-')){await updates.action(action.slice('updates-'.length),workSnapshot());return;}
  switch(action) {
    case 'navigate': captureDrafts();view.page=node.dataset.page;view.error=null;render();return;
    case 'settings-pane':captureDrafts();view.pane=id;render();return;
    case 'manage-providers':captureDrafts();view.page='settings';view.pane='keys';render();return;
    case 'manage-purpose':captureDrafts();view.page='settings';view.pane='purposes';render();return;
    case 'set-theme':case 'set-mode':case 'set-tone':{
      if(activeJob())return;captureDrafts();
      const flip=action==='set-theme'&&id===themeMode(),tone=action==='set-tone'?id:flip?(themeTone()==='dark'?'light':'dark'):themeTone();
      // Paint the choice first, then save; a failed save puts the old look back.
      const patch=themePatch(action==='set-tone'?themeMode():id,tone),previous={theme:state.settings.theme,hologram_tone:state.settings.hologram_tone};
      state.settings={...state.settings,...patch};renderChrome();syncAppearanceControls();
      try{await saveSettings(patch);}catch(error){state.settings={...state.settings,...previous};renderChrome();syncAppearanceControls();throw error;}
      renderChrome();syncAppearanceControls();return;
    }
    case 'source-type':if(activeJob())return;await backend.clearSelection();state.selected=[];state.job=null;view.preview=null;view.error=null;await saveSettings({source_type:id});render();return;
    case 'swap':await saveSettings({source_choice:state.settings.target_lang,target_lang:state.settings.source_choice});render();return;
    case 'pick':state.selected=await backend.pick(effectiveSourceType() === 'file' ? 'files' : effectiveSourceType());state.job=null;view.preview=null;view.error=null;render();return;
    case 'clear-selection':await backend.clearSelection();state.selected=[];state.job=null;view.preview=null;view.error=null;render();return;
    case 'choose-output':{const path=await backend.chooseOutput();if(path)view.outputDir=path;render();return;}
    case 'translate': {
      if (!state.selected.length) throw new Error(t('desktop.no_files'));
      const settings=state.settings,target=settings.target_lang === 'other' ? settings.target_other?.trim() : settings.target_lang || 'en';
      if (!target) throw new Error(t('sidebar.target_other'));
      await loadJob(await backend.start({inputPaths:state.selected.map(file=>file.path),sourceLanguage:settings.source_choice === 'auto' ? null : settings.source_choice,targetLanguage:target,purposeId:settings.purpose || 'general',providerId:settings.provider || 'auto',model:settings.model_by_provider?.[settings.provider] || '',project:settings.project || 'default',outputDir:view.outputDir,mode:settings.content_mode || 'document'}));return;
    }
    case 'cancel':await backend.cancel(state.job.id);state.job=await backend.job(state.job.id);render();pollJob();return;
    case 'retry':await loadJob(await backend.retry(state.job.id));return;
    case 'source-preview':view.preview=await backend.preview(null,state.selected[0].path);render();return;
    case 'result-preview':view.preview=await backend.preview(state.job.id,state.job.files.find(file=>file.status==='saved')?.input || null);render();return;
    case 'copy-output-path':await navigator.clipboard.writeText(state.job.outputDir);toast(t('result.copied'));return;
    case 'open-output':await backend.openPath(state.job.outputDir);return;
    case 'export-file':case 'export-zip':{const result=await backend.exportResult(state.job.id,action === 'export-zip');if(result&&!result.cancelled)toast(typeof result==='string'?displayPath(result):result.path?displayPath(result.path):t('toast.batch_done'));return;}
    case 'edit-provider':view.editingProvider=id;render();return;
    case 'close-provider-editor':clearProviderInputs(view.editingProvider);view.editingProvider=null;render();return;
    case 'remove-provider':if(await confirmRemoval(state.providers.find(p=>p.id===id)?.name || id)){await backend.deleteProvider(id);clearProviderInputs(id);await refresh();toast(t('svc.removed'));render();}return;
    case 'probe-provider':{const result=await backend.probeProvider(id);await refresh();toast(result?.detail || providerStatus(state.providers.find(p=>p.id===id) || {}));render();return;}
    case 'test-provider':{
      const provider=state.providers.find(provider=>provider.id===id);
      if(provider)provider.transportVerified=false;
      render();
      try{const result=await backend.testProvider(id);await refresh();toast(result?.success===false?t('svc.test_fail',{msg:sanitizedProviderDetail(result.message)}):t('svc.test_ok',{secs:result?.secs || '',time:new Date().toLocaleTimeString()}),result?.success===false?'error':'success');}
      catch(error){await refresh();toast(t('svc.test_fail',{msg:sanitizedProviderDetail(error?.message || error)}),'error');}
      render();return;
    }
    case 'copy-purpose':view.purposeDraft=state.purposes.find(p=>p.id===view.purposeFrom)?.instructions || '';render();return;
    case 'move-up':case 'move-down': {captureOrder();const chain=view.orderDraft,index=chain.findIndex(row=>row.id===id),to=index+(action==='move-up'?-1:1);if(index>=0&&to>=0&&to<chain.length){[chain[index],chain[to]]=[chain[to],chain[index]];await saveSettings({chain});view.orderDraft=null;render();}return;}
    case 'save-order':captureOrder();await saveSettings({chain:view.orderDraft,concurrency:state.settings.concurrency,per_provider:state.settings.per_provider});view.orderDraft=null;await refresh();toast(t('desktop.save_settings'));render();return;
    case 'add-term':view.glossaryDraft=[...readGlossary(),{term:'',translation:''}];render();document.querySelector(`#glossary-term-${view.glossaryDraft.length-1}`)?.focus();return;
    case 'delete-term':view.glossaryDraft=readGlossary().filter((_,index)=>index!==Number(id));render();return;
    case 'open-data':await backend.openPath(state.dataDir);return;
    case 'import-legacy':{const result=await backend.importLegacy();if(result&&!result.cancelled){await refresh();view.glossaryDraft=null;view.purposeDraft=null;view.orderDraft=null;toast(result.message || t('desktop.import'));render();}return;}
  }
}
const immediateActions=new Set(['navigate','settings-pane','manage-providers','manage-purpose','updates-check','updates-download','updates-cancel','updates-later','updates-official']);
let operationControls=new Map(),operationFocus=null,operationPane=null;
function beginOperation(lock=true) {
  operationFocus=document.activeElement;operationControls=new Map();
  operationPane=content.querySelector('.settings-pane')||content.firstElementChild;
  view.busy=true;content.setAttribute('aria-busy','true');
  // A theme change paints at once and keeps every control live; its save only holds the busy flag.
  const controls=lock?[...document.querySelectorAll('.theme-switcher button'),...content.querySelectorAll('button,input,select,textarea')]:[];
  for(const node of controls){if(!immediateActions.has(node.dataset.action)){operationControls.set(node,node.disabled);node.disabled=true;}}
}
function finishOperation(preserveContent=false) {
  view.busy=false;content.setAttribute('aria-busy','false');
  if(preserveContent&&operationPane?.isConnected&&content.contains(operationPane)){
    for(const [node,disabled] of operationControls)if(node.isConnected)node.disabled=disabled;
    renderChrome();syncAppearanceControls();patchProviderRows();reportUnsaved();
    if(operationFocus?.isConnected&&!operationFocus.disabled&&document.activeElement===document.body)operationFocus.focus({preventScroll:true});
  }else render();
  operationControls.clear();operationFocus=null;operationPane=null;
}
document.addEventListener('click',async event => {
  const node=event.target.closest('[data-action]');
  if(!node || node.disabled)return;
  event.preventDefault();
  const action=node.dataset.action;
  if(view.busy && !immediateActions.has(action))return;
  const ownsBusy=!immediateActions.has(action),preserveContent=['set-theme','set-mode','set-tone'].includes(action);
  if(ownsBusy)beginOperation(!preserveContent);
  try {await runAction(action,node);} catch(error) {view.error=String(error?.message || error);toast(view.error,'error');if(view.page==='translate'&&!preserveContent)render();}
  finally {if(ownsBusy)finishOperation(preserveContent);}
});
document.addEventListener('change',async event => {
  const node=event.target;
  if(controlsBusy())return;
  if(!['purpose-from','glossary-project','interface-language'].includes(node.id) && !node.dataset.setting && !node.dataset.order && !node.dataset.updatePreference && !node.id.startsWith('order-model-') && !node.id.startsWith('order-effort-'))return;
  let ownsBusy=false;
  const preserveContent=['theme','reduced_motion'].includes(node.dataset.setting);
  try {
    if(node.id==='purpose-from'){view.purposeFrom=node.value;return;}
    if(node.dataset.order || node.id.startsWith('order-model-') || node.id.startsWith('order-effort-')){captureOrder();reportUnsaved();return;}
    ownsBusy=true;beginOperation();
    if(node.dataset.updatePreference){await updates.savePreference(node.dataset.updatePreference,node.type==='checkbox'?node.checked:node.type==='number'?Number(node.value):node.value);return;}
    if(node.id==='glossary-project'){await saveSettings({project:node.value});state.glossary=await backend.glossary(node.value);view.glossaryDraft=null;render();return;}
    if(node.id==='interface-language'){captureDrafts();await saveSettings({ui_lang_follow:node.value==='system',ui_lang:node.value==='system'?systemLanguage():node.value});render();return;}
    if(node.dataset.setting){captureDrafts();const name=node.dataset.setting,value=node.type==='checkbox'?node.checked:['concurrency','per_provider'].includes(name)?Number(node.value):node.value;await saveSettings(name==='purpose'?{purpose:value,content_mode:value==='game'?'game':'document'}:{[name]:value});if(preserveContent){renderChrome();syncAppearanceControls();}else render();return;}
  } catch(error){toast(String(error?.message || error),'error');}
  finally{if(ownsBusy)finishOperation(preserveContent);}
});
document.addEventListener('submit',async event => {
  const form=event.target;
  if(!form.dataset.form)return;
  event.preventDefault();if(controlsBusy())return;
  beginOperation();
  const submit=form.querySelector('[type=submit]');if(submit)submit.disabled=true;
  try {
    if(form.dataset.form==='purpose'){view.purposeDraft=form.elements['purpose-instructions'].value.trim();if(!view.purposeDraft)throw new Error(t('purpose.instructions'));await backend.savePurpose(view.purposeDraft);await refresh();view.purposeDraft=null;toast(t('purpose.saved',{v:state.purposes.find(p=>p.id==='custom')?.version || ''}));}
    if(form.dataset.form==='provider') {
      const id=form.dataset.id,provider={id};
      const key=form.elements[`provider-key-${id}`],base=form.elements[`provider-base-${id}`],path=form.elements[`provider-path-${id}`];
      if(key){const value=key.value.trim();key.value='';if(value)provider.apiKey=value;else if(!state.providers.find(provider=>provider.id===id)?.keyPresent)throw new Error(t('keys.paste_api'));}
      if(base)provider.baseUrl=base.value.trim();if(path)provider.cliPath=path.value.trim();
      try{await backend.saveProvider(provider);}finally{delete provider.apiKey;}
      clearProviderInputs(id);view.editingProvider=null;await refresh();toast(t('sidebar.save_base_ok'));
    }
    if(form.dataset.form==='project'){const name=form.elements['new-project'].value.trim();if(!name)throw new Error(t('sidebar.new_project'));const result=await backend.createProject(name),project=typeof result==='string'?result:result.id || result.name;await saveSettings({project});await refresh();state.glossary=await backend.glossary(project);view.glossaryDraft=null;view.projectDraft='';}
    if(form.dataset.form==='glossary'){const entries=readGlossary().filter(row=>row.term || row.translation);if(entries.some(row=>!row.term || !row.translation))throw new Error(`${t('glossary.col_src')} / ${t('glossary.col_dst')}`);await backend.saveGlossary(state.settings.project || 'default',entries);state.glossary=entries;view.glossaryDraft=null;toast(t('glossary.saved',{name:state.settings.project || 'default'}));}
  }catch(error){toast(String(error?.message || error),'error');}
  finally{finishOperation();}
});
document.addEventListener('input',event=>{
  if(event.target.id==='new-project')view.projectDraft=event.target.value;
  if(event.target.closest('[data-form="purpose"]'))view.purposeDraft=document.querySelector('#purpose-instructions')?.value ?? null;
  if(event.target.closest('[data-form="glossary"]'))view.glossaryDraft=readGlossary();
  if(event.target.id.startsWith('order-model-'))captureOrder();
  reportUnsaved();
});
window.addEventListener('resize',placeIndicators,{passive:true});
// Native drag/drop paths are validated and registered by Rust before this event.
window.addEventListener('beforeunload',() => {clearTimeout(view.pollTimer);clearTimeout(view.toastTimer);updates.dispose();motion.dispose();view.holoField?.();for(const unlisten of view.unlisten)unlisten();});
async function initialize() {
  const words=['VERSORA','TRANSLATE','翻譯','TRADUIRE','ÜBERSETZEN','翻訳','TRADUCIR','번역'];
  document.querySelector('#watermark').innerHTML=['a','b'].map((layer,n)=>`<div class="watermark-layer ${layer}">${Array.from({length:30},(_,index)=>`<div>${escape((words.slice((index+n)%words.length).concat(words.slice(0,(index+n)%words.length)).join(' · ')+' · ').repeat(12))}</div>`).join('')}</div>`).join('');
  document.querySelectorAll('.nav-button').forEach(node=>node.insertAdjacentHTML('afterbegin',icon(node.dataset.page==='settings'?'settings':'swap')));
  view.holoField=bindHoloField(document);
  content.innerHTML=`<div class="loading" data-testid="loading">${icon('spinner')}Versora</div>`;
  try {
    view.unlisten.push(await windowChrome.initialize());
    await refresh();render();
    // Healthy startup is acknowledged only after the real backend state and first render.
    try{await backend.updates.healthAck();await updates.initialize();}catch(error){updates.error=String(error?.message||error);}
    view.unlisten.push(await backend.listenJob(receiveJob),await backend.listenSelection(async()=>{try{await refresh();render();}catch(error){toast(String(error),'error');}}));
    if(activeJob())pollJob();
  }catch(error){content.innerHTML='';const box=document.querySelector('#connection-error');box.textContent=String(error?.message || error);box.hidden=false;content.setAttribute('aria-busy','false');}
}
initialize();
