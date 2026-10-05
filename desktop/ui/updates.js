import {backend} from './backend.js';
import {icon} from './icons.js';
import {t} from './i18n.js';

const escape=value=>String(value??'').replace(/[&<>"']/g,char=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
const label=(key,vars)=>escape(t(`updates.${key}`,vars));
const bytes=value=>{const n=Math.max(0,Number(value)||0);return n<1024?`${n} B`:n<1048576?`${(n/1024).toFixed(1)} KB`:`${(n/1048576).toFixed(1)} MB`;};
const btn=(action,key,disabled=false,glyph=null,primary=false,hidden=false)=>`<button type="button" data-action="updates-${action}" data-testid="updates-${action}" class="${primary?'primary':''}" ${disabled?'disabled':''} ${hidden?'hidden':''}>${glyph?icon(glyph):''}${label(key)}</button>`;
const select=(name,value,options,disabled)=>`<select id="updates-${name}" data-testid="updates-${name}" data-update-preference="${name}" ${disabled?'disabled':''}>${options.map(([id,key])=>`<option value="${escape(id)}" ${id===value?'selected':''}>${label(key)}</option>`).join('')}</select>`;
const when=value=>{const date=new Date(Number(value));return Number.isFinite(date.getTime())&&value?date.toLocaleString(document.documentElement.lang||undefined,{dateStyle:'medium',timeStyle:'short'}):null;};
const percent=s=>Number(s.totalBytes)>0?Math.min(100,Math.floor(Number(s.bytesReceived)*100/Number(s.totalBytes))):0;

export class UpdatesPane {
  constructor(onChange,onOpen=()=>{}) {this.onChange=onChange;this.onOpen=onOpen;this.nativeState=null;this.preferences=null;this.error=null;this.watching=false;this.pollTimer=null;this.polling=false;this.unlisten=null;this.pending=new Set();this.pillDismissed=false;this.installing=false;this.announced=false;this.toastTimer=null;}
  async initialize() {
    const [nativeState,preferences]=await Promise.all([backend.updates.state(),backend.updates.preferences()]);
    this.nativeState=nativeState;this.preferences=preferences;this.error=null;
    this.unlisten=await backend.updates.listen(value=>{this.nativeState=value;if(value.preferences)this.preferences=value.preferences;this.renderPill();this.onChange();});
    this.renderPill();this.announce();
  }
  async refresh() {
    const result=await backend.updates.state();
    this.nativeState=result;if(result.preferences)this.preferences=result.preferences;this.error=null;this.renderPill();
  }
  watchPane(visible) {
    if(visible===this.watching)return;
    this.watching=visible;clearTimeout(this.pollTimer);
    if(visible)this.poll();
  }
  async poll() {
    if(!this.watching||this.polling)return;
    this.polling=true;
    const previous=JSON.stringify([this.nativeState,this.error]);
    try{await this.refresh();}catch(error){this.error=String(error?.message||error);}
    finally{this.polling=false;if(this.watching){if(previous!==JSON.stringify([this.nativeState,this.error]))this.onChange();this.pollTimer=setTimeout(()=>this.poll(),700);}}
  }
  dispose() {this.watching=false;clearTimeout(this.pollTimer);clearTimeout(this.toastTimer);if(this.unlisten)this.unlisten();}
  // Release notes arrive from the network: they are only ever inserted as escaped text.
  notesView(s) {
    const notes=s.releaseNotes;
    if(!notes||![s.candidateVersion,s.currentVersion].includes(notes.version))return '';
    return `<section class="update-notes" data-testid="updates-notes" aria-label="${label('notes_title',{v:notes.version})}"><h3>${label('notes_title',{v:notes.version})}</h3>${notes.text?`<pre data-testid="updates-notes-text">${escape(notes.text)}</pre>`:`<p class="hint">${label('notes_empty')}</p>`}${notes.truncated?`<button type="button" class="link" data-action="updates-official" data-testid="updates-notes-more">${icon('folder_open')}${label('notes_more')}</button>`:''}</section>`;
  }
  render(workBusy,workSnapshot) {
    const s=this.nativeState,p=this.preferences||s?.preferences;
    if(!s||!p)return `<div class="card loading" data-testid="updates-loading">${icon('spinner')}${label('loading')}${this.error?`<p class="notice error" data-testid="updates-error">${escape(this.error)}</p>`:''}</div>`;
    const active=Boolean(s.active)||['checking','downloading'].includes(s.status)||this.pending.has('check')||this.pending.has('download'),busy=workBusy||active;
    const installed=s.installed!==false,capable=s.downloadCapability===true&&s.installCapabilities===true;
    const status=this.pending.has('check')?'checking':['idle','checking','available','downloading','ready','failed','cancelled'].includes(s.status)?s.status:'idle';
    const glyph=active?'spinner':status==='failed'?'alert':status==='cancelled'?'stop':['available','ready'].includes(status)?'download':'check';
    const v=s.candidateVersion||'';
    const heading={available:label('available_v',{v}),downloading:label('downloading_p',{p:percent(s)}),ready:label('ready_v',{v}),failed:label('failed_e',{e:s.error||this.error||''})}[status]||label(status);
    const checked=when(s.lastCheckAt);
    const error=status!=='failed'&&(s.error||this.error);
    const showDownload=capable&&!p.autoDownload&&status==='available',showInstall=capable&&status==='ready';
    return `<div class="card pane-card" data-testid="updates-card"><div class="update-state" data-testid="updates-state" data-status="${escape(status)}" role="status" aria-live="polite">${icon(glyph)}<div><h2>${heading}</h2><p class="hint" data-testid="updates-current">${label('current')}: ${escape(s.currentVersion||'—')} · ${label('last_checked')}: <span data-testid="updates-last-checked">${escape(checked||t('updates.never'))}</span></p></div></div>${status==='downloading'?`<progress data-testid="updates-progress" max="${Math.max(1,Number(s.totalBytes)||1)}" value="${Math.max(0,Number(s.bytesReceived)||0)}" aria-label="${label('downloading_p',{p:percent(s)})}"></progress><p class="hint" data-testid="updates-bytes">${bytes(s.bytesReceived)} / ${s.totalBytes?bytes(s.totalBytes):'—'}</p>`:''}${showInstall&&s.installOnQuit?`<p class="hint" data-testid="updates-install-on-quit">${label('install_on_quit')}</p>`:''}${error?`<p class="notice error" role="alert" data-testid="updates-error">${escape(error)}</p>`:''}${!installed?`<div class="notice" data-testid="updates-capability-reason">${icon('alert')}<p>${label('not_installed')}</p></div>`:!capable?`<div class="notice" data-testid="updates-capability-reason">${icon('alert')}<p>${label('no_trust')}</p></div>`:''}${this.notesView(s)}<div class="actions spread"><div class="actions">${btn('check','check',busy||!installed,'retry')}${active?btn('cancel','cancel',this.pending.has('cancel'),'stop'):''}</div><div class="actions">${!capable||status==='failed'?btn('official','official',workBusy,'folder_open'):''}${btn('download','download',busy||!showDownload,'download',false,!showDownload)}${btn('install','install',busy||!showInstall||this.installing,'retry',true,!showInstall)}</div></div><hr class="divider"><label class="limits-line"><input type="checkbox" data-testid="updates-autoDownload" data-update-preference="autoDownload" ${p.autoDownload?'checked':''} ${busy||!installed?'disabled':''}>${label('auto_download')}</label><div class="inline-fields"><div class="field"><label for="updates-channel">${label('channel')}</label>${select('channel',p.channel,[['stable','stable'],['preview','preview']],busy||!installed)}</div><div class="field"><label for="updates-policy">${label('policy')}</label>${select('policy',p.policy,[['periodic','periodic'],['startup','startup'],['manual','manual']],busy||!installed)}</div></div>${p.policy==='periodic'?`<div class="field"><label for="updates-intervalHours">${label('interval')} · ${label('hours')}</label><input type="number" id="updates-intervalHours" data-testid="updates-intervalHours" data-update-preference="intervalHours" min="1" max="168" step="1" value="${escape(p.intervalHours)}" ${busy||!installed?'disabled':''}></div>`:''}</div>`;
  }
  // B3/B5: persistent accent pill beside the settings entry once a verified package is ready.
  renderPill() {
    const s=this.nativeState,show=Boolean(s&&s.status==='ready'&&s.installCapabilities===true&&s.candidateVersion&&!this.pillDismissed);
    let pill=document.querySelector('#update-pill');
    if(!show){pill?.remove();return;}
    if(!pill){pill=document.createElement('div');pill.id='update-pill';pill.className='update-pill';pill.dataset.testid='update-pill';pill.setAttribute('role','group');}
    const dock=document.querySelector('.page-dock');
    if(dock&&pill.parentElement!==dock)dock.prepend(pill);else if(!dock&&pill.parentElement!==document.body)document.body.append(pill);
    const markup=`<button type="button" class="update-pill-main" data-action="updates-install" data-testid="update-pill-install" ${this.installing?'disabled':''}>${icon(this.installing?'spinner':'download')}<span>${this.installing?label('restarting'):label('pill',{v:s.candidateVersion})}</span></button><button type="button" class="update-pill-later" data-action="updates-dismiss" data-testid="update-pill-later" title="${label('pill_later')}" aria-label="${label('pill_later')}" ${this.installing?'disabled':''}>${icon('close')}</button>`;
    if(pill.innerHTML!==markup)pill.innerHTML=markup;
    pill.setAttribute('aria-label',t('updates.pill',{v:s.candidateVersion}));
  }
  // B6: first launch after an update.
  announce() {
    const done=this.nativeState?.justUpdated;
    if(this.announced||!done?.to)return;
    this.announced=true;
    let box=document.querySelector('#update-toast');
    if(!box){box=document.createElement('div');box.id='update-toast';box.className='toast update-toast';box.dataset.testid='update-toast';box.setAttribute('role','status');box.setAttribute('aria-live','polite');document.body.append(box);}
    box.innerHTML=`<span>${label('updated_to',{v:done.to})}</span><button type="button" class="link" data-action="updates-whatsnew" data-testid="update-toast-whatsnew">${label('whats_new')}</button><button type="button" class="update-toast-close" data-action="updates-toast-close" data-testid="update-toast-close" title="${label('close')}" aria-label="${label('close')}">${icon('close')}</button>`;
    box.hidden=false;clearTimeout(this.toastTimer);this.toastTimer=setTimeout(()=>this.hideToast(),15000);
  }
  hideToast() {clearTimeout(this.toastTimer);const box=document.querySelector('#update-toast');if(box)box.hidden=true;}
  // B4: active or unsaved work needs an explicit choice before the app restarts.
  confirmWork() {
    let dialog=document.querySelector('#update-confirm');
    if(!dialog){dialog=document.createElement('dialog');dialog.id='update-confirm';dialog.dataset.testid='update-confirm';document.body.append(dialog);}
    dialog.innerHTML=`<form method="dialog"><h2>${label('confirm_title')}</h2><p>${label('confirm_busy')}</p><div class="actions"><button value="cancel" data-testid="update-confirm-cancel">${label('confirm_cancel')}</button><button value="confirm" class="primary" data-testid="update-confirm-accept">${label('confirm_update')}</button></div></form>`;
    dialog.returnValue='cancel';
    return new Promise(resolve=>{dialog.addEventListener('close',()=>resolve(dialog.returnValue==='confirm'),{once:true});dialog.showModal();});
  }
  async install(workSnapshot) {
    let work={busy:Boolean(workSnapshot.busy),unsaved:Boolean(workSnapshot.unsaved),pendingPersistence:Boolean(workSnapshot.pendingPersistence)};
    if(work.busy||work.unsaved){if(!await this.confirmWork())return;work={...work,confirmed:true};}
    const result=await backend.updates.requestInstall(work);
    // On approval Versora stops jobs, waits for saves, starts the verified installer and exits.
    if(result?.installApproved===true){this.installing=true;this.renderPill();}
  }
  async savePreference(name,value) {
    if(!['policy','channel','intervalHours','autoDownload'].includes(name))throw new Error('Invalid update preference');
    const preferences={...(this.preferences||this.nativeState?.preferences),[name]:value};
    if(!Number.isInteger(preferences.intervalHours)||preferences.intervalHours<1||preferences.intervalHours>168)throw new Error('Update interval must be 1–168 hours.');
    const result=await backend.updates.savePreferences(preferences);
    this.preferences=result.preferences||result;await this.refresh();
  }
  async action(name,workSnapshot) {
    if(name==='dismiss'){this.pillDismissed=true;this.renderPill();return;}
    if(name==='toast-close'){this.hideToast();return;}
    if(name==='whatsnew'){this.hideToast();this.onOpen();return;}
    const calls={check:()=>backend.updates.check(),download:()=>backend.updates.download(),cancel:()=>backend.updates.cancel(),later:()=>backend.updates.later(),install:()=>this.install(workSnapshot),official:()=>backend.updates.officialRelease()};
    if(!calls[name])throw new Error('Unknown update action');
    if(this.pending.has(name))return;
    this.pending.add(name);this.error=null;this.onChange();
    try{await calls[name]();await this.refresh();}catch(error){this.error=String(error?.message||error);throw error;}
    finally{this.pending.delete(name);this.onChange();}
  }
}
