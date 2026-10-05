import {backend} from './backend.js';

const labels = {
  en:['Minimize','Maximize','Restore','Close'],
  'zh-Hant':['最小化','最大化','還原','關閉'],
  'zh-Hans':['最小化','最大化','还原','关闭'],
  ja:['最小化','最大化','元に戻す','閉じる'],
  ko:['최소화','최대화','복원','닫기'],
  es:['Minimizar','Maximizar','Restaurar','Cerrar'],
  fr:['Réduire','Agrandir','Restaurer','Fermer'],
  de:['Minimieren','Maximieren','Wiederherstellen','Schließen'],
  pt:['Minimizar','Maximizar','Restaurar','Fechar'],
  vi:['Thu nhỏ','Phóng to','Khôi phục','Đóng'],
  th:['ย่อหน้าต่าง','ขยายหน้าต่าง','คืนค่า','ปิด'],
  id:['Minimalkan','Maksimalkan','Pulihkan','Tutup'],
};
const svg = body => `<svg viewBox="0 0 12 12" aria-hidden="true">${body}</svg>`;
const glyphs = {
  minimize:svg('<path d="M1 6.5h10"/>'),
  maximize:svg('<rect x="1.5" y="1.5" width="9" height="9"/>'),
  restore:svg('<path d="M3.5 1.5h7v7M1.5 3.5h7v7h-7z"/>'),
  close:svg('<path d="m1.5 1.5 9 9m0-9-9 9"/>'),
};

export class WindowChrome {
  constructor(onError, beforeClose) {
    this.onError=onError;
    this.beforeClose=beforeClose;
    this.ready=false;
    this.state=null;
    this.generation=0;
    this.pending=false;
    this.controls=document.querySelector('[data-testid=window-controls]');
    this.controls.addEventListener('click',event=>this.action(event));
    document.querySelector('.window-resize-handles').addEventListener('mousedown',event=>{
      const edge=event.target.closest('[data-resize-direction]');
      if(!edge||event.button!==0||!this.ready||this.state?.maximized)return;
      event.preventDefault();event.stopPropagation();
      backend.window.resize(edge.dataset.resizeDirection).catch(error=>this.report(error));
    });
    this.render();
  }
  report(error) {this.onError(String(error?.message||error));}
  render() {
    const copy=labels[document.documentElement.lang]||labels.en;
    const maximized=this.state?.maximized===true;
    const names={minimize:copy[0],maximize:maximized?copy[2]:copy[1],close:copy[3]};
    for(const button of this.controls.querySelectorAll('[data-window-action]')){
      const action=button.dataset.windowAction;
      button.innerHTML=glyphs[action==='maximize'&&maximized?'restore':action];
      button.setAttribute('aria-label',names[action]);button.title=names[action];
      button.disabled=!this.ready||this.pending;
    }
    if(this.state){
      document.documentElement.dataset.windowMaximized=String(maximized);
      document.querySelector('[data-testid=topbar]').dataset.windowFocus=String(this.state.focused);
    }
  }
  async refresh() {
    const generation=++this.generation;
    const state=await backend.window.state();
    if(generation===this.generation){this.state=state;this.render();}
  }
  async initialize() {
    const remove=await backend.window.listen(()=>{this.refresh().catch(error=>this.report(error));});
    try {await this.refresh();this.ready=true;this.render();}
    catch(error){remove();throw error;}
    return ()=>{++this.generation;remove();};
  }
  async action(event) {
    const button=event.target.closest('[data-window-action]');
    if(!button||button.disabled||this.pending)return;
    const action=button.dataset.windowAction;
    if(!['minimize','maximize','close'].includes(action))return;
    event.preventDefault();event.stopPropagation();
    this.pending=true;this.render();
    try {if(action==='close')await this.beforeClose();await backend.window[action]();if(action!=='close')await this.refresh();}
    catch(error){this.report(error);}
    finally{this.pending=false;this.render();}
  }
}
