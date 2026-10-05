/* Litora color-only theme timeline and route-entry timings, using local styles. */
export class UIMotion {
  constructor() {
    this.root=document.documentElement;
    this.initialized=false;
    this.themeTimer=null;
    this.themeGeneration=0;
    this.routeTimer=null;
    this.route=null;
    this.reduced=window.matchMedia('(prefers-reduced-motion:reduce)');
    this.onMotion=()=>{if(this.reduced.matches)this.settleTheme();};
    this.reduced.addEventListener('change',this.onMotion);
  }
  settleTheme() {
    this.themeGeneration++;
    clearTimeout(this.themeTimer);
    this.themeTimer=null;
    if(this.root.dataset.themeTransition){
      for(const animation of this.root.getAnimations({subtree:true})){
        if(['background-color','color','border-color','border-top-color','border-right-color','border-bottom-color','border-left-color','outline-color','fill','stroke'].includes(animation.transitionProperty))animation.cancel();
      }
      delete this.root.dataset.themeTransition;
    }
  }
  theme(theme,reduced,tone='light') {
    this.root.dataset.reducedMotion=String(Boolean(reduced));
    const changed=this.root.dataset.theme!==theme||this.root.dataset.hologramTone!==tone;
    if(!changed){if(reduced||this.reduced.matches)this.settleTheme();this.initialized=true;return;}
    clearTimeout(this.themeTimer);
    const generation=++this.themeGeneration;
    const animate=this.initialized&&!reduced&&!this.reduced.matches;
    if(animate){this.root.dataset.themeTransition='running';void this.root.offsetWidth;}
    else this.settleTheme();
    this.root.dataset.theme=theme;
    this.root.dataset.hologramTone=tone;
    void this.root.offsetWidth;
    this.initialized=true;
    if(animate)this.themeTimer=setTimeout(()=>{if(generation!==this.themeGeneration)return;delete this.root.dataset.themeTransition;this.themeTimer=null;},280);
  }
  enter(content,route) {
    if(route===this.route)return;
    this.route=route;
    clearTimeout(this.routeTimer);
    content.classList.remove('is-enter');
    if(this.reduced.matches)return;
    void content.offsetWidth;
    content.classList.add('is-enter');
    this.routeTimer=setTimeout(()=>{content.classList.remove('is-enter');this.routeTimer=null;},800);
  }
  dispose() {
    this.settleTheme();clearTimeout(this.routeTimer);
    this.reduced.removeEventListener('change',this.onMotion);
  }
}
