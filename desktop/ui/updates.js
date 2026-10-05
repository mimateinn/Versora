import {backend} from './backend.js';
import {icon} from './icons.js';
import {t} from './i18n.js';

const escape=value=>String(value??'').replace(/[&<>"']/g,char=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
const label=key=>escape(t(`updates.${key}`));
const bytes=value=>{const n=Math.max(0,Number(value)||0);return n<1024?`${n} B`:n<1048576?`${(n/1024).toFixed(1)} KB`:`${(n/1048576).toFixed(1)} MB`;};
const btn=(action,key,disabled=false,glyph=null,primary=false)=>`<button type="button" data-action="updates-${action}" data-testid="updates-${action}" class="${primary?'primary':''}" ${disabled?'disabled':''}>${glyph?icon(glyph):''}${label(key)}</button>`;
const select=(name,value,options,disabled)=>`<select id="updates-${name}" data-testid="updates-${name}" data-update-preference="${name}" ${disabled?'disabled':''}>${options.map(([id,key])=>`<option value="${escape(id)}" ${id===value?'selected':''}>${label(key)}</option>`).join('')}</select>`;

export class UpdatesPane {
  constructor(onChange) {this.onChange=onChange;this.nativeState=null;this.preferences=null;this.error=null;this.watching=false;this.pollTimer=null;this.polling=false;this.unlisten=null;this.pending=new Set();}
  async initialize() {
    const [nativeState,preferences]=await Promise.all([backend.updates.state(),backend.updates.preferences()]);
    this.nativeState=nativeState;this.preferences=preferences;this.error=null;
    this.unlisten=await backend.updates.listen(value=>{this.nativeState=value;if(value.preferences)this.preferences=value.preferences;this.onChange();});
  }
  async refresh() {
    const result=await backend.updates.state();
    this.nativeState=result;if(result.preferences)this.preferences=result.preferences;this.error=null;
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
  dispose() {this.watching=false;clearTimeout(this.pollTimer);if(this.unlisten)this.unlisten();}
  render(workBusy,workSnapshot) {
    const s=this.nativeState,p=this.preferences||s?.preferences;
    if(!s||!p)return `<div class="card loading" data-testid="updates-loading">${icon('spinner')}${label('loading')}${this.error?`<p class="notice error" data-testid="updates-error">${escape(this.error)}</p>`:''}</div>`;
    const active=Boolean(s.active)||['checking','downloading'].includes(s.status)||this.pending.has('check')||this.pending.has('download'),busy=workBusy||active;
    const trust=s.trust===true,install=s.installCapabilities===true;
    const status=this.pending.has('check')?'checking':['idle','checking','available','downloading','ready','failed','cancelled'].includes(s.status)?s.status:'idle';
    const glyph=active?'spinner':status==='failed'?'alert':status==='cancelled'?'stop':status==='available'?'download':'check';
    const blockedWork=Boolean(workSnapshot.busy||workSnapshot.unsaved||workSnapshot.pendingPersistence);
    const reason=s.capabilityReason||t('updates.blocked');
    return `<div class="card pane-card" data-testid="updates-card"><div class="update-state" data-testid="updates-state" data-status="${escape(status)}" role="status" aria-live="polite">${icon(glyph)}<div><h2>${label(status)}</h2><p class="hint" data-testid="updates-current">${label('current')}: ${escape(s.currentVersion||'—')}</p>${s.candidateVersion?`<p class="hint" data-testid="updates-candidate">${label('candidate')}: ${escape(s.candidateVersion)}</p>`:''}</div></div>${status==='downloading'?`<progress data-testid="updates-progress" max="${Math.max(1,Number(s.totalBytes)||1)}" value="${Math.max(0,Number(s.bytesReceived)||0)}" aria-label="${escape(t('updates.downloading'))}"></progress><p class="hint" data-testid="updates-bytes">${bytes(s.bytesReceived)} / ${s.totalBytes?bytes(s.totalBytes):'—'}</p>`:''}${s.deferred?`<p class="notice" data-testid="updates-deferred">${label('deferred')}</p>`:''}${s.error||this.error?`<p class="notice error" role="alert" data-testid="updates-error">${escape(s.error||this.error)}</p>`:''}${!trust||!install?`<div class="notice" data-testid="updates-capability-reason">${icon('alert')}<div><p>${label('blocked')}</p><p class="hint">${escape(reason)}</p></div></div>`:''}<div class="inline-fields"><div class="field"><label for="updates-policy">${label('policy')}</label>${select('policy',p.policy,[['startup','startup'],['manual','manual'],['periodic','periodic']],busy)}</div><div class="field"><label for="updates-channel">${label('channel')}</label>${select('channel',p.channel,[['stable','stable'],['preview','preview']],busy)}</div></div>${p.policy==='periodic'?`<div class="field"><label for="updates-intervalHours">${label('interval')} · ${label('hours')}</label><input type="number" id="updates-intervalHours" data-testid="updates-intervalHours" data-update-preference="intervalHours" min="1" max="168" step="1" value="${escape(p.intervalHours)}" ${busy?'disabled':''}></div>`:''}<label class="limits-line"><input type="checkbox" data-testid="updates-autoDownload" data-update-preference="autoDownload" ${p.autoDownload?'checked':''} ${busy||!trust?'disabled':''}>${label('auto_download')}</label>${s.nextCheckAt?`<p class="hint" data-testid="updates-next-check">${label('next')}: ${escape(s.nextCheckAt)}</p>`:''}${s.highestHealthyVersion?`<p class="hint" data-testid="updates-highest-healthy">${label('healthy')}: ${escape(s.highestHealthyVersion)}</p>`:''}<hr class="divider"><div class="actions spread"><div class="actions">${btn('check','check',busy,'retry')}${active?btn('cancel','cancel',this.pending.has('cancel'),'stop'):''}</div><div class="actions">${s.candidateVersion?btn('later','later',busy):''}${btn('download','download',busy||!trust||!s.candidateVersion,'download')}${btn('install','install',busy||!trust||!install||status!=='ready'||blockedWork,'check',true)}</div></div><div class="actions">${btn('official','official',workBusy,'folder_open')}</div></div>`;
  }
  async savePreference(name,value) {
    if(!['policy','channel','intervalHours','autoDownload'].includes(name))throw new Error('Invalid update preference');
    const preferences={...(this.preferences||this.nativeState?.preferences),[name]:value};
    if(!Number.isInteger(preferences.intervalHours)||preferences.intervalHours<1||preferences.intervalHours>168)throw new Error('Update interval must be 1–168 hours.');
    const result=await backend.updates.savePreferences(preferences);
    this.preferences=result.preferences||result;await this.refresh();
  }
  async action(name,workSnapshot) {
    const calls={check:()=>backend.updates.check(),download:()=>backend.updates.download(),cancel:()=>backend.updates.cancel(),later:()=>backend.updates.later(),install:()=>backend.updates.requestInstall(workSnapshot),official:()=>backend.updates.officialRelease()};
    if(!calls[name])throw new Error('Unknown update action');
    if(this.pending.has(name))return;
    this.pending.add(name);this.error=null;this.onChange();
    try{await calls[name]();await this.refresh();}catch(error){this.error=String(error?.message||error);throw error;}
    finally{this.pending.delete(name);this.onChange();}
  }
}
