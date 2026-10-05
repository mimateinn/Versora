/* Opal's page canvas, ported from Litora (app/holo-field.js and
   app/holo-light.js) so the series shares one material. A fixed canvas sits
   behind the shell; holo-field.worker.js draws the fluid, its rolling words,
   the brush, the lamp and the press ripple on a transferred OffscreenCanvas,
   so the UI thread never paints it. This file only reports state, glass
   rectangles and pointer samples. Versora runs it in Opal only; the other
   palettes keep their own watermark. */

/* Surfaces that are glass in Opal (keep in step with the glass rules in
   styles.css), plus headings: the words are cut out under them, so glass
   shows only fluid and no word runs through a title. */
const GLASS = ['.card', '.dropzone', '.segments', '.seg', '.settings-rail', 'button', 'input', 'select',
  'textarea', '[data-glass]', 'h1', 'h2', 'h3'].join(', ');
const radii = new WeakMap();

export function holeRects(root, page, view = root.ownerDocument?.defaultView) {
  const out = [];
  for (const el of root.querySelectorAll(GLASS)) {
    const b = el.getBoundingClientRect();
    if (b.width < 2 || b.height < 2 || b.bottom < page.y || b.top > page.y + page.h
      || b.right < page.x || b.left > page.x + page.w) continue;
    let r = radii.get(el);
    if (r === undefined) {
      r = parseFloat(view?.getComputedStyle(el).borderTopLeftRadius) || 0;
      radii.set(el, r);
    }
    out.push(b.left, b.top, b.width, b.height, Math.min(r, b.width / 2, b.height / 2));
    if (out.length >= 2500) break;
  }
  return new Float32Array(out);
}

/* Page watermark hue: position on the page, not an angle around the centre,
   so a fast cross of the midline cannot flip the wheel. */
function watermarkPhase(nx, ny) {
  const x = Math.min(1, Math.max(0, nx));
  const y = Math.min(1, Math.max(0, ny));
  return x * 0.72 + y * 0.28;
}

function pageRect(doc) {
  const shell = doc.querySelector('.app-shell');
  const box = shell?.getBoundingClientRect();
  if (!box || box.width < 8 || box.height < 8) return null;
  return { x: box.left, y: box.top, w: shell.clientWidth || box.width, h: shell.clientHeight || box.height };
}

export function bindHoloField(doc = document) {
  const win = doc.defaultView;
  if (!win || typeof win.Worker !== 'function' || typeof win.HTMLCanvasElement !== 'function'
    || !('transferControlToOffscreen' in win.HTMLCanvasElement.prototype)) return () => {};
  const root = doc.documentElement;
  const osReduced = win.matchMedia?.('(prefers-reduced-motion: reduce)');
  let worker = null;
  let size = '';
  let sent = '';
  let page = null;

  function ensure() {
    if (worker) return;
    const canvas = doc.createElement('canvas');
    canvas.className = 'holo-field';
    canvas.setAttribute('aria-hidden', 'true');
    doc.body.prepend(canvas);
    const off = canvas.transferControlToOffscreen();
    worker = new Worker(new URL('./holo-field.worker.js', import.meta.url));
    worker.onerror = (event) => console.warn('holo-field worker', event.message || event);
    size = `${win.innerWidth}x${win.innerHeight}@${win.devicePixelRatio || 1}`;
    worker.postMessage({ type: 'init', canvas: off, width: win.innerWidth, height: win.innerHeight, dpr: win.devicePixelRatio || 1 }, [off]);
  }

  const opal = () => root.dataset.theme === 'hologram';
  const live = () => root.dataset.reducedMotion !== 'true' && !osReduced?.matches;

  function sync() {
    if (!doc.body) return;
    // Nothing is created until Opal is first chosen.
    if (!worker && !opal()) return;
    ensure();
    const nextSize = `${win.innerWidth}x${win.innerHeight}@${win.devicePixelRatio || 1}`;
    if (nextSize !== size) {
      size = nextSize;
      worker.postMessage({ type: 'size', width: win.innerWidth, height: win.innerHeight, dpr: win.devicePixelRatio || 1 });
    }
    page = pageRect(doc);
    const state = {
      running: opal() && doc.visibilityState !== 'hidden',
      live: live(),
      mode: 'field',
      tone: root.dataset.hologramTone === 'dark' ? 'dark' : 'light',
      page,
      quiet: !page,
    };
    const key = JSON.stringify(state);
    holesLater();
    if (key === sent) return;
    sent = key;
    worker.postMessage({ type: 'state', ...state });
  }

  // Glass rectangles follow scrolling and renders, read once per frame.
  let holeFrame = 0;
  let lastHoles = new Float32Array(0);
  function holesLater() {
    if (holeFrame || !worker) return;
    holeFrame = win.requestAnimationFrame(() => {
      holeFrame = 0;
      const shell = doc.querySelector('.app-shell');
      const rects = shell && page ? holeRects(shell, page, win) : new Float32Array(0);
      if (rects.length === lastHoles.length && rects.every((v, i) => Math.abs(v - lastHoles[i]) < 0.5)) return;
      lastHoles = rects;
      const copy = rects.slice();
      worker.postMessage({ type: 'holes', rects: copy }, [copy.buffer]);
    });
  }

  let queued = 0;
  const later = () => {
    if (queued) return;
    queued = win.setTimeout(() => { queued = 0; sync(); }, 0);
  };
  const running = () => worker && sent.includes('"running":true') && sent.includes('"live":true');

  doc.addEventListener('scroll', holesLater, { capture: true, passive: true });
  win.addEventListener('resize', later);
  doc.addEventListener('visibilitychange', later);
  osReduced?.addEventListener?.('change', later);
  new MutationObserver(later).observe(root, { attributes: true, attributeFilter: ['data-theme', 'data-hologram-tone', 'data-reduced-motion'] });
  const shellWatch = new MutationObserver(holesLater);
  const shell = doc.querySelector('.app-shell');
  if (shell) {
    shellWatch.observe(shell, { childList: true, subtree: true });
    if (typeof ResizeObserver !== 'undefined') new ResizeObserver(later).observe(shell);
  }

  // Pointer input: the watermark hue, the brush and the lamp, one sample per frame.
  let pointerFrame = 0;
  let lastPointer = null;
  const flushPointer = () => {
    pointerFrame = 0;
    const event = lastPointer;
    lastPointer = null;
    if (!event || !running()) return;
    const box = page || { x: 0, y: 0, w: win.innerWidth || 1, h: win.innerHeight || 1 };
    const phase = watermarkPhase((event.clientX - box.x) / box.w, (event.clientY - box.y) / box.h);
    worker.postMessage({ type: 'pointer', x: event.clientX, y: event.clientY, t: event.timeStamp, phase });
  };
  const onMove = (event) => {
    lastPointer = event;
    if (!pointerFrame) pointerFrame = win.requestAnimationFrame(flushPointer);
  };
  // A press anywhere drops a ripple into the canvas; it never takes the click.
  const onDown = (event) => {
    if (event.button === 0 && running()) worker.postMessage({ type: 'tap', x: event.clientX, y: event.clientY });
  };
  const cancel = () => {
    if (pointerFrame) win.cancelAnimationFrame(pointerFrame);
    pointerFrame = 0;
    lastPointer = null;
  };
  const onOut = (event) => {
    if (event.relatedTarget) return;
    cancel();
    worker?.postMessage({ type: 'leave' });
  };
  doc.addEventListener('pointermove', onMove, { passive: true });
  doc.addEventListener('pointerdown', onDown, { passive: true });
  doc.addEventListener('pointerout', onOut, { passive: true });
  win.addEventListener('blur', cancel);
  sync();
  return () => {
    cancel();
    win.removeEventListener('blur', cancel);
    doc.removeEventListener('pointermove', onMove);
    doc.removeEventListener('pointerdown', onDown);
    doc.removeEventListener('pointerout', onOut);
    shellWatch.disconnect();
    worker?.terminate();
  };
}
