// Shared custom controls. Geometry and motion follow Litora's pickerField/openPicker
// (app/pages/system.js), .ctx and .litora-tip (app/styles/app.css). Native selects
// remain hidden form/value carriers; only the custom controls enter the focus order.
const WORDS = {
  en:['Filter options','Type to filter','No matching options'],
  'zh-Hant':['篩選選項','輸入文字篩選','冇符合嘅選項'],
  'zh-Hans':['筛选选项','输入文字筛选','没有匹配的选项'],
  ja:['選択肢を絞り込む','入力して絞り込む','一致する選択肢がありません'],
  ko:['옵션 필터','입력하여 필터','일치하는 옵션 없음'],
  fr:['Filtrer les options','Saisir pour filtrer','Aucune option correspondante'],
  de:['Optionen filtern','Zum Filtern tippen','Keine passenden Optionen'],
  es:['Filtrar opciones','Escribe para filtrar','Ninguna opción coincide'],
  pt:['Filtrar opções','Digite para filtrar','Nenhuma opção correspondente'],
  vi:['Lọc tùy chọn','Nhập để lọc','Không có tùy chọn phù hợp'],
  th:['กรองตัวเลือก','พิมพ์เพื่อกรอง','ไม่มีตัวเลือกที่ตรงกัน'],
  id:['Filter pilihan','Ketik untuk memfilter','Tidak ada pilihan yang cocok'],
};
const CARET = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6.5 9.5 12 15l5.5-5.5"></path></svg>';
const normalized = value => String(value).normalize('NFKC').toLocaleLowerCase();
const disabledOption = option => option.disabled || Boolean(option.parentElement?.disabled);
const optionRevision = select => JSON.stringify([...select.options].map(o=>[o.value,o.label,o.disabled,Boolean(o.parentElement?.disabled),o.hidden,o.selected]));
const setAttribute = (node,name,value) => {if(node.getAttribute(name)!==String(value))node.setAttribute(name,String(value));};

export class CustomControls {
  constructor(root=document) {
    this.root=root;this.records=new WeakMap();this.popup=null;this.focusRestore=null;
    this.controller=new AbortController();this.frame=null;this.serial=0;
    this.tip=document.createElement('div');this.tip.className='custom-tip';this.tip.id='versora-custom-tip';
    this.tip.setAttribute('role','tooltip');this.tip.hidden=true;document.body.append(this.tip);
    this.tipHost=null;this.tipTimer=null;this.tipLeave=null;
    this.observer=new MutationObserver(()=>this.sync());
    this.observer.observe(document.body,{subtree:true,childList:true,attributes:true,attributeFilter:['title','data-tip','disabled','selected','hidden']});
    const on=(host,event,fn,options={})=>host.addEventListener(event,fn,{...options,signal:this.controller.signal});
    on(document,'pointerdown',event=>{
      this.hideTip();
      if(this.popup&&!this.popup.box.contains(event.target)&&!this.popup.record.button.contains(event.target))this.close(false);
    },{capture:true});
    on(document,'change',event=>{if(event.target instanceof HTMLSelectElement)this.syncSelect(event.target);});
    on(document,'pointerover',event=>{
      if(this.tip.contains(event.target)){clearTimeout(this.tipLeave);return;}
      const host=event.target.closest?.('[data-tip]');if(host)this.queueTip(host);
    });
    on(document,'pointerout',event=>this.leaveTip(event));
    on(document,'focusin',event=>{
      const host=event.target.closest?.('[data-tip]');if(host)this.queueTip(host);else this.hideTip();
    });
    on(document,'focusout',event=>{if(this.tipHost===event.target)this.hideTip();});
    on(document,'keydown',event=>{if(event.key==='Escape')this.hideTip();});
    on(window,'blur',()=>{this.hideTip();this.close(false);});
    on(window,'resize',()=>{this.hideTip();this.schedulePosition();});
    on(window,'scroll',event=>{
      if(this.popup?.box.contains(event.target))return;
      this.hideTip();this.schedulePosition();
    },{capture:true,passive:true});
    if(window.visualViewport){on(visualViewport,'resize',()=>this.schedulePosition());on(visualViewport,'scroll',()=>this.schedulePosition());}
    this.sync();
  }
  words(){return WORDS[document.documentElement.lang]||WORDS.en;}
  sync() {
    // Migrate any remaining title before it can produce a second OS tooltip.
    for(const node of this.root.querySelectorAll('[title]')){
      const text=node.getAttribute('title');if(text)node.dataset.tip=text;node.removeAttribute('title');
    }
    for(const select of this.root.querySelectorAll('select'))this.syncSelect(select);
    if(this.popup){
      const old=this.popup.record;
      if(!old.select.isConnected){
        const replacement=document.getElementById(old.select.id);
        const next=replacement instanceof HTMLSelectElement?this.records.get(replacement):null;
        if(next&&!next.button.disabled){
          this.popup.record=next;setAttribute(next.button,'aria-expanded','true');
          setAttribute(next.button,'aria-controls',this.popup.list.id);
          this.popup.resize.disconnect();this.popup.resize.observe(next.button);
          this.rebuild(this.popup.filter?.value||'');this.schedulePosition();
        }else this.close(false);
      }else if(old.button.disabled)this.close(false);
      else if(this.popup.revision!==optionRevision(old.select)){this.rebuild(this.popup.filter?.value||'');this.schedulePosition();}
    }
    if(this.tipHost&&!this.tipHost.isConnected)this.hideTip();
    else if(this.tipHost&&!this.tip.hidden&&this.tip.textContent!==this.tipHost.dataset.tip){this.tip.textContent=this.tipHost.dataset.tip||'';this.placeTip(this.tipHost);}
    if(this.focusRestore&&Date.now()<this.focusRestore.until){
      const trigger=document.getElementById(this.focusRestore.id);
      if(trigger&&!trigger.disabled&&document.activeElement===document.body){trigger.focus({preventScroll:true});this.focusRestore=null;}
    }
  }
  syncSelect(select) {
    if(select.multiple||select.size>1)return;
    let record=this.records.get(select);
    if(!record){
      const button=document.createElement('button');button.type='button';button.className='custom-select';
      button.id=(select.id||`versora-select-${++this.serial}`)+'-trigger';
      if(select.dataset.testid)button.dataset.testid=select.dataset.testid+'-trigger';
      button.setAttribute('role','combobox');button.setAttribute('aria-haspopup','listbox');
      button.setAttribute('aria-expanded','false');button.setAttribute('aria-autocomplete','none');
      const labelIds=[];
      for(const [index,label] of [...select.labels].entries()){
        if(!label.id)label.id=button.id+'-label-'+index;
        label.htmlFor=button.id;labelIds.push(label.id);
      }
      if(labelIds.length)button.setAttribute('aria-labelledby',labelIds.join(' '));
      else {
        const row=select.closest('.order-row'),name=row?.querySelector('.order-name')?.textContent.trim();
        const heading=row?.parentElement.querySelector('.order-header .order-effort')?.textContent.trim();
        button.setAttribute('aria-label',select.getAttribute('aria-label')||(name?[heading,name].filter(Boolean).join(': '):select.name||select.id));
      }
      const value=document.createElement('span');value.className='custom-select-value';
      const caret=document.createElement('span');caret.className='custom-select-caret';caret.innerHTML=CARET;
      button.append(value,caret);select.after(button);select.hidden=true;select.tabIndex=-1;select.setAttribute('aria-hidden','true');
      select.classList.add('custom-select-native');record={select,button,value};this.records.set(select,record);
      button.addEventListener('click',()=>{if(this.popup?.record===record)this.close(true);else this.open(record);},{signal:this.controller.signal});
      button.addEventListener('keydown',event=>{
        if(['ArrowDown','ArrowUp','Enter',' '].includes(event.key)){
          event.preventDefault();this.open(record,event.key==='ArrowUp'?'last':null);
        }else if(event.key==='Home'||event.key==='End'){
          event.preventDefault();this.open(record,event.key==='Home'?'first':'last');
        }else if(event.key.length===1&&!event.ctrlKey&&!event.metaKey&&!event.altKey){
          event.preventDefault();this.open(record);if(this.popup?.filter){this.popup.filter.value=event.key;this.rebuild(event.key);}else this.typeahead(event.key);
        }
      },{signal:this.controller.signal});
    }
    const text=select.selectedOptions[0]?.label||'';
    if(record.value.textContent!==text)record.value.textContent=text;
    const disabled=select.disabled||!select.options.length;if(record.button.disabled!==disabled)record.button.disabled=disabled;
    setAttribute(record.button,'aria-disabled',disabled);
    if(select.dataset.tip)setAttribute(record.button,'data-tip',select.dataset.tip);
    if(select.hasAttribute('aria-describedby'))setAttribute(record.button,'aria-describedby',select.getAttribute('aria-describedby'));
    return record;
  }
  open(record,edge=null) {
    if(record.button.disabled)return;
    this.close(false);this.hideTip();
    const id=`versora-picker-${++this.serial}`;
    const box=document.createElement('div');box.className='custom-picker';box.dataset.testid='custom-picker';
    const list=document.createElement('div');list.className='custom-picker-list';list.id=id;
    list.setAttribute('role','listbox');list.tabIndex=-1;list.dataset.testid='custom-picker-list';
    const name=record.button.getAttribute('aria-labelledby');
    if(name)list.setAttribute('aria-labelledby',name);else list.setAttribute('aria-label',record.button.getAttribute('aria-label'));
    const filter=record.select.options.length>8?document.createElement('input'):null;
    if(filter){
      const words=this.words();filter.type='text';filter.className='custom-picker-filter';filter.dataset.testid='custom-picker-filter';
      filter.setAttribute('role','searchbox');filter.setAttribute('aria-label',words[0]);filter.placeholder=words[1];
      filter.autocomplete='off';filter.spellcheck=false;filter.setAttribute('aria-controls',id);box.append(filter);
    }
    box.append(list);document.body.append(box);
    const resize=new ResizeObserver(()=>this.schedulePosition());resize.observe(record.button);
    this.popup={record,box,list,filter,resize,rows:[],options:[],current:-1,buffer:'',typedAt:0};
    setAttribute(record.button,'aria-expanded','true');setAttribute(record.button,'aria-controls',id);
    box.addEventListener('keydown',event=>this.keydown(event));
    box.addEventListener('pointerdown',event=>{if(event.target.closest('.custom-option'))event.preventDefault();});
    box.addEventListener('pointermove',event=>{const row=event.target.closest('.custom-option');if(row)this.paint(Number(row.dataset.index));});
    box.addEventListener('click',event=>{const row=event.target.closest('.custom-option');if(row)this.commit(Number(row.dataset.index));});
    filter?.addEventListener('input',()=>{this.rebuild(filter.value);this.position();});
    this.rebuild('');
    if(edge)this.paint(edge==='first'?this.next(-1,1):this.next(this.popup.rows.length,-1));
    this.position();(filter||list).focus({preventScroll:true});
  }
  rebuild(query) {
    const p=this.popup;if(!p)return;
    const previous=p.options[p.current]?.value;
    p.revision=optionRevision(p.record.select);
    const q=normalized(query.trim());p.options=[...p.record.select.options].filter(option=>!option.hidden&&(!q||normalized(option.label).includes(q)));
    p.list.replaceChildren();p.rows=[];p.current=-1;
    for(const node of [p.list,p.record.button,p.filter].filter(Boolean))node.removeAttribute('aria-activedescendant');
    for(const [index,option] of p.options.entries()){
      const row=document.createElement('div');row.className='custom-option';row.setAttribute('role','option');
      row.id=p.list.id+'-option-'+index;row.dataset.index=String(index);row.dataset.value=option.value;
      row.setAttribute('aria-selected',String(option.selected));row.setAttribute('aria-disabled',String(disabledOption(option)));
      const label=document.createElement('span');label.className='custom-option-label';label.textContent=option.label;
      if(option.dataset.tip||option.title)label.dataset.tip=option.dataset.tip||option.title;
      const tick=document.createElement('span');tick.className='custom-option-tick';tick.setAttribute('aria-hidden','true');tick.textContent=option.selected?'✓':'';
      row.append(label,tick);p.rows.push(row);p.list.append(row);
    }
    if(!p.rows.length){const empty=document.createElement('div');empty.className='custom-picker-empty';empty.setAttribute('role','status');empty.textContent=this.words()[2];p.list.append(empty);}
    let selected=p.options.findIndex(option=>option.value===(previous??p.record.select.value)&&!disabledOption(option));
    if(selected<0)selected=this.next(-1,1);this.paint(selected);
  }
  next(start,direction) {
    const p=this.popup;if(!p?.options.length)return -1;
    for(let step=1;step<=p.options.length;step++){
      const index=(start+direction*step+p.options.length*2)%p.options.length;
      if(!disabledOption(p.options[index]))return index;
    }
    return -1;
  }
  paint(index) {
    const p=this.popup;if(!p||index<0||!p.options[index]||disabledOption(p.options[index]))return;
    p.current=index;p.rows.forEach((row,n)=>{row.dataset.active=String(n===index);});
    for(const node of [p.list,p.record.button,p.filter].filter(Boolean))node.setAttribute('aria-activedescendant',p.rows[index].id);
    p.rows[index].scrollIntoView({block:'nearest',behavior:'instant'});
  }
  keydown(event) {
    const p=this.popup;if(!p||event.isComposing)return;
    let index=p.current;
    if(event.key==='ArrowDown')index=this.next(index,1);
    else if(event.key==='ArrowUp')index=this.next(index,-1);
    else if(event.key==='Home')index=this.next(-1,1);
    else if(event.key==='End')index=this.next(p.options.length,-1);
    else if(event.key==='PageDown'||event.key==='PageUp'){
      const direction=event.key==='PageDown'?1:-1;for(let n=0;n<8;n++)index=this.next(index,direction);
    }else if(event.key==='Enter'){event.preventDefault();this.commit(index);return;}
    else if(event.key==='Escape'){event.preventDefault();event.stopPropagation();this.close(true);return;}
    else if(event.key==='Tab'){this.close(true);return;}
    else if(!p.filter&&(event.key===' '||event.key.length===1)&&!event.ctrlKey&&!event.metaKey&&!event.altKey){event.preventDefault();this.typeahead(event.key);return;}
    else return;
    event.preventDefault();this.paint(index);
  }
  typeahead(character) {
    const p=this.popup;if(!p)return;
    const now=Date.now(),char=normalized(character);p.buffer=now-p.typedAt>600?char:p.buffer+char;p.typedAt=now;
    const query=[...p.buffer].every(value=>value===char)?char:p.buffer;
    for(let n=1;n<=p.options.length;n++){
      const index=(p.current+n+p.options.length)%p.options.length;
      if(!disabledOption(p.options[index])&&normalized(p.options[index].label).startsWith(query)){this.paint(index);return;}
    }
  }
  commit(index) {
    const p=this.popup,option=p?.options[index];if(!option||disabledOption(option))return;
    const select=p.record.select,changed=select.value!==option.value;
    select.value=option.value;this.syncSelect(select);this.close(true);
    if(changed){select.dispatchEvent(new Event('input',{bubbles:true}));select.dispatchEvent(new Event('change',{bubbles:true}));}
  }
  close(restore=true) {
    const p=this.popup;if(!p)return;
    this.popup=null;p.resize.disconnect();p.box.remove();
    setAttribute(p.record.button,'aria-expanded','false');p.record.button.removeAttribute('aria-controls');p.record.button.removeAttribute('aria-activedescendant');
    if(restore&&p.record.button.isConnected){
      this.focusRestore={id:p.record.button.id,until:Date.now()+2000};p.record.button.focus({preventScroll:true});
    }
  }
  viewport() {
    const vv=window.visualViewport;return {left:vv?.offsetLeft||0,top:vv?.offsetTop||0,width:vv?.width||innerWidth,height:vv?.height||innerHeight};
  }
  schedulePosition(){if(this.frame!==null)return;this.frame=requestAnimationFrame(()=>{this.frame=null;this.position();});}
  position() {
    const p=this.popup;if(!p)return;
    const r=p.record.button.getBoundingClientRect(),v=this.viewport(),right=v.left+v.width,bottom=v.top+v.height;
    if(!p.record.button.isConnected||r.bottom<v.top||r.top>bottom||r.right<v.left||r.left>right){this.close(false);return;}
    const width=Math.min(Math.max(r.width,200),v.width-16);
    p.box.style.width=Math.max(40,width)+'px';
    const below=bottom-r.bottom-12,above=r.top-v.top-12;
    const space=Math.max(above,below),filterHeight=p.filter?p.filter.offsetHeight+4:0;
    p.list.style.maxHeight=Math.max(24,Math.min(244,space-filterHeight-14))+'px';
    const height=p.box.offsetHeight;
    const up=below<height&&above>below;
    p.box.dataset.side=up?'above':'below';p.box.style.transformOrigin=up?'bottom left':'top left';
    p.box.style.left=Math.max(v.left+8,Math.min(r.left,right-width-8))+'px';
    p.box.style.top=Math.max(v.top+8,Math.min(up?r.top-height-4:r.bottom+4,bottom-height-8))+'px';
  }
  queueTip(host) {
    if(this.popup||!host.dataset.tip?.trim()||host===this.tipHost)return;
    this.hideTip();const text=host.dataset.tip.trim();
    const visible=(host.textContent||'').replace(/\s+/g,' ').trim();
    if(visible===text&&host.scrollWidth<=host.clientWidth)return;
    this.tipHost=host;this.tipTimer=setTimeout(()=>{
      if(this.tipHost!==host||!host.isConnected||this.popup)return;
      this.tip.textContent=host.dataset.tip.trim();this.tip.hidden=false;
      const descriptions=(host.getAttribute('aria-describedby')||'').split(/\s+/).filter(Boolean);
      if(!descriptions.includes(this.tip.id))host.setAttribute('aria-describedby',[...descriptions,this.tip.id].join(' '));
      this.placeTip(host);this.tip.classList.add('is-on');
    },380);
  }
  placeTip(host) {
    const r=host.getBoundingClientRect(),v=this.viewport(),w=this.tip.offsetWidth,h=this.tip.offsetHeight;
    const top=r.top-h-8<v.top+8?r.bottom+8:r.top-h-8;
    this.tip.style.left=Math.round(Math.max(v.left+8,Math.min(r.left+(r.width-w)/2,v.left+v.width-w-8)))+'px';
    this.tip.style.top=Math.round(Math.max(v.top+8,Math.min(top,v.top+v.height-h-8)))+'px';
  }
  leaveTip(event) {
    if(!this.tipHost)return;const next=event.relatedTarget;
    if(next&&(this.tipHost.contains(next)||this.tip.contains(next))){clearTimeout(this.tipLeave);return;}
    if(this.tipHost.contains(event.target)||this.tip.contains(event.target)){
      clearTimeout(this.tipLeave);this.tipLeave=setTimeout(()=>this.hideTip(),80);
    }
  }
  hideTip() {
    clearTimeout(this.tipTimer);clearTimeout(this.tipLeave);
    if(this.tipHost){const ids=(this.tipHost.getAttribute('aria-describedby')||'').split(/\s+/).filter(id=>id&&id!==this.tip.id);if(ids.length)this.tipHost.setAttribute('aria-describedby',ids.join(' '));else this.tipHost.removeAttribute('aria-describedby');}
    this.tipHost=null;this.tip.hidden=true;this.tip.classList.remove('is-on');
  }
  dispose(){this.observer.disconnect();this.controller.abort();this.close(false);this.hideTip();this.tip.remove();if(this.frame!==null)cancelAnimationFrame(this.frame);}
}
