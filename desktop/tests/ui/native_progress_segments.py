"""Focused checks in a real Windows Tauri/WebView2 instance with private Demo data.

Input must be registered through the native executable's ordinary Open With entry.
No provider requests, pretend bridge, injected job state, or emulated viewport.
--baseline records the original behavior without asserting the new UI contract.
"""
import argparse
import asyncio
import hashlib
import json
from pathlib import Path

from playwright.async_api import async_playwright, Error
from native_chrome_journey import OwnedWindow, capture_full_window, owned_test_path, require
from native_journey import comparable_path, navigate


GEOMETRY = """() => {
  const bar=document.querySelector('[data-testid=titlebar-progress]');
  const top=document.querySelector('.topbar').getBoundingClientRect();
  const shell=document.querySelector('.app-shell').getBoundingClientRect();
  const progress=bar?.getBoundingClientRect();
  return {titlebarBottom:top.bottom,titlebarHeight:top.height,shellTop:shell.top,
    x:progress?.x,y:progress?.y,width:progress?.width,height:progress?.height,
    viewport:innerWidth,state:bar?.dataset.state,unit:bar?.dataset.unit,
    value:Number(bar?.getAttribute('aria-valuenow')),max:Number(bar?.getAttribute('aria-valuemax'))};
}"""

SEGMENT_TRACE = """async () => {
  const group=document.querySelector('[data-testid=source-types]');
  const current=()=>document.querySelector('[data-testid=source-types]');
  const widths=()=>[...current().querySelectorAll('button')].map(b=>b.getBoundingClientRect().width);
  const position=()=>new DOMMatrixReadOnly(getComputedStyle(current(),'::before').transform).m41;
  const before=widths(),samples=[];
  // Interrupt the live transition repeatedly through the actual UI event handler.
  for(const id of ['folder','zip','file','zip','folder','file','folder','zip','folder','file']){
    const start=position();current().querySelector(`[data-id=${id}]`).click();
    samples.push({id,retained:current()===group,before:start,after:position(),widths:widths()});
    await new Promise(resolve=>setTimeout(resolve,35));
  }
  return {before,samples};
}"""


async def idle(page):
    await page.wait_for_function("() => document.querySelector('#content').getAttribute('aria-busy')==='false'")


async def fixed_geometry(page, result, fixed, where):
    value = await page.evaluate(GEOMETRY)
    result['geometry'].append({'where':where, **value})
    if fixed:
        require(value['titlebarHeight']==48 and value['height']==2, 'Titlebar or strip height changed')
        require(abs(value['y']-value['titlebarBottom'])<.1 and abs(value['shellTop']-value['y']-2)<.1,
                'Progress must occupy the fixed slot directly below the titlebar')
        require(value['x']==0 and abs(value['width']-value['viewport'])<.1, 'Strip must span the client width')
    return value


async def run(args):
    proof_path = owned_test_path(args.launch_proof)
    proof = json.loads(proof_path.read_text(encoding='utf-8-sig'))
    output = owned_test_path(args.evidence)
    output.mkdir(parents=True, exist_ok=False)
    result = {'sourceSha':proof['sourceSha'],'exeSha256':proof['executableSHA256'],
              'baseline':args.baseline,'native':False,'liveProviderTranslation':'NOT_RUN',
              'demo':'NOT_RUN','geometry':[],'errors':[],'screenshots':[],'status':'RUNNING'}
    native = OwnedWindow(proof)
    fixed = not args.baseline
    try:
        async with async_playwright() as p:
            browser = await p.chromium.connect_over_cdp(f"http://127.0.0.1:{proof['debugPort']}")
            pages = [page for context in browser.contexts for page in context.pages
                     if await page.evaluate('Boolean(window.__TAURI__?.core?.invoke)')]
            require(len(pages)==1, 'Exactly one actual Tauri context is required')
            page = pages[0]
            page.on('pageerror', lambda error:result['errors'].append(str(error)))
            await navigate(page,'translate')
            await page.get_by_test_id('translate-card').wait_for()
            await idle(page)
            state = await page.evaluate("window.__TAURI__.core.invoke('get_state')")
            require(state['testMode'] and comparable_path(state['dataDir'])==comparable_path(proof['dataDir']),
                    'Only a fresh explicitly opted-in private Demo profile is eligible')
            require(len(state['selected'])==1, 'One genuine native registered chunk input is required')
            source = Path(proof['fixture'])
            require(comparable_path(state['selected'][0]['path'])==comparable_path(source), 'Registered input differs')
            original = hashlib.sha256(source.read_bytes()).hexdigest()
            result['native'] = True
            native.restore();native.place(1060,800)
            result['nativeWindow'] = native.snapshot()
            await navigate(page,'settings')
            await page.get_by_test_id('settings-pane-appearance').click()
            await page.get_by_test_id('interface-language').select_option('zh-Hant')
            await idle(page)
            await navigate(page,'translate')
            await page.get_by_test_id('translator').select_option('demo')
            await idle(page)
            await page.evaluate('document.fonts.ready')
            await asyncio.sleep(.8)
            await fixed_geometry(page,result,fixed,'idle')
            if fixed:
                await page.evaluate("""() => {
                  window.__progressSamples=[];
                  window.__progressObserver=new MutationObserver(()=>window.__progressSamples.push(
                    document.querySelector('[data-testid=titlebar-progress]').dataset.state));
                  window.__progressObserver.observe(document.querySelector('[data-testid=titlebar-progress]'),{attributes:true});
                }""")
            await page.get_by_test_id('translate-start').click()
            progress_selector = 'titlebar-progress' if fixed else 'job-progress'
            await page.wait_for_function("id => {const n=document.querySelector(`[data-testid=${id}]`);return n?.dataset.unit==='chunks'&&(Number(n.getAttribute('aria-valuenow'))||n.value)>0&&(Number(n.getAttribute('aria-valuenow'))||n.value)<(Number(n.getAttribute('aria-valuemax'))||n.max);}", arg=progress_selector)
            running = await fixed_geometry(page,result,fixed,'translation-running')
            if fixed:
                require(running['state']=='determinate' and running['unit']=='chunks', 'Real chunk progress is required')
            await page.get_by_test_id('job-card').scroll_into_view_if_needed()
            image = output / ('before-progress.png' if args.baseline else 'after-progress.png')
            capture_full_window(native,image);result['screenshots'].append(image.name)
            if fixed:
                await navigate(page,'settings')
                for pane in ('purposes','keys','order','glossary','appearance','updates'):
                    await page.get_by_test_id(f'settings-pane-{pane}').click()
                    await fixed_geometry(page,result,True,f'running-settings-{pane}')
                native.place(800,640)
                await fixed_geometry(page,result,True,'native-minimum-800x640')
                native.place(1060,800)
                await fixed_geometry(page,result,True,'native-resize-1060x800')
                await navigate(page,'translate')
            await page.wait_for_function("() => document.querySelector('[data-testid=job-card]')?.dataset.jobStatus==='done'",timeout=90000)
            require(await page.get_by_test_id('job-card').get_attribute('data-used-demo')=='true', 'Demo provenance required')
            require(hashlib.sha256(source.read_bytes()).hexdigest()==original, 'Original input changed')
            done = await fixed_geometry(page,result,fixed,'translation-done')
            if fixed:require(done['state']=='idle', 'Completed work must release the strip')
            # Repeat the actual registered input, cancel after at least one real chunk.
            await page.get_by_test_id('translate-start').click()
            await page.wait_for_function("() => document.querySelector('[data-testid=job-card]')?.dataset.jobStatus==='running'")
            await page.wait_for_function("id => {const n=document.querySelector(`[data-testid=${id}]`);return n?.dataset.unit==='chunks'&&(Number(n.getAttribute('aria-valuenow'))||n.value)>0;}",arg=progress_selector)
            await page.get_by_test_id('job-cancel').click()
            await page.wait_for_function("() => document.querySelector('[data-testid=job-card]')?.dataset.jobStatus==='stopped'")
            await idle(page)
            cancelled = await fixed_geometry(page,result,fixed,'translation-cancelled')
            if fixed:
                require(cancelled['state']=='idle', 'Stopped work must release the strip')
                result['progressStates'] = await page.evaluate('window.__progressSamples')
                require('determinate' in result['progressStates'] and 'idle' in result['progressStates'], 'Progress lifecycle was not observed')
                await page.evaluate('window.__progressObserver.disconnect();delete window.__progressObserver;delete window.__progressSamples')
            result['demo'] = 'PASS_REAL_NATIVE_OFFLINE_PIPELINE_AND_CANCELLATION'
            require(hashlib.sha256(source.read_bytes()).hexdigest()==original, 'Cancellation changed original input')
            # Source controls intentionally clear only selection, as before this fix.
            await page.get_by_test_id('source-type-file').click()
            await idle(page);await asyncio.sleep(.35)
            image = output / ('before-mode-row.png' if args.baseline else 'after-mode-row.png')
            capture_full_window(native,image);result['screenshots'].append(image.name)
            trace = await page.evaluate(SEGMENT_TRACE)
            await idle(page);await asyncio.sleep(.35)
            result['segmentTrace'] = trace
            actual = await page.evaluate("window.__TAURI__.core.invoke('get_state')")
            result['persistedSourceType']=actual['settings']['source_type']
            if fixed:
                require(actual['settings']['source_type']=='file', 'Latest source choice did not persist')
                require(all(sample['retained'] for sample in trace['samples']), 'Source group was replaced during switching')
                require(all(abs(sample['after']-sample['before'])<2 for sample in trace['samples']), 'Interrupted indicator jumped')
                require(all(all(abs(left-right)<.1 for left,right in zip(trace['before'],sample['widths'])) for sample in trace['samples']), 'Button widths shifted with selected weight')
            result['segmentLayout'] = await page.get_by_test_id('source-types').evaluate("""group => [...group.querySelectorAll('button')].map(button=>{
              const glyph=button.querySelector('.vi').getBoundingClientRect();
              const label=button.querySelector('.segment-label-text');
              const box=label?.getBoundingClientRect();
              return {id:button.dataset.id,iconWidth:glyph.width,iconHeight:glyph.height,
                cssGap:getComputedStyle(button).gap,actualGap:box?box.left-glyph.right:null,
                centerDifference:box?(box.top+box.height/2)-(glyph.top+glyph.height/2):null,
                height:button.getBoundingClientRect().height,transform:getComputedStyle(button).transform};
            })""")
            if fixed:
                for item in result['segmentLayout']:
                    require(item['cssGap']=='4px' and abs(item['actualGap']-4)<.1, 'Icon-label gap is not 4px')
                    require(abs(item['centerDifference'])<.1 and item['height']==32 and item['iconWidth']==16 and item['iconHeight']==16, 'Source controls are not centered at their preserved sizes')
                for mode in ('preference','css-media'):
                    await navigate(page,'settings');await page.get_by_test_id('settings-pane-appearance').click()
                    await page.get_by_test_id('reduce-motion').set_checked(mode=='preference');await idle(page)
                    await page.emulate_media(reduced_motion='reduce' if mode=='css-media' else 'no-preference')
                    await navigate(page,'translate')
                    await page.get_by_test_id('source-type-folder').click();await idle(page)
                    settled = await page.get_by_test_id('source-types').evaluate("""g=>({state:g.dataset.ind,duration:getComputedStyle(g,'::before').transitionDuration,
                      painted:new DOMMatrixReadOnly(getComputedStyle(g,'::before').transform).m41,target:g.querySelector('button.active').offsetLeft})""")
                    require(settled['state']=='init' and abs(settled['painted']-settled['target'])<.1, 'Reduced motion must settle immediately')
                    result.setdefault('reducedMotion',{})[mode]=settled
                await page.emulate_media(reduced_motion='no-preference')
            require(not result['errors'], f"Frontend errors: {result['errors']}")
            result['status'] = 'PASS_NATIVE_FIX' if fixed else 'RECORDED_NATIVE_BASELINE'
            try:await page.evaluate('window.__TAURI__.window.getCurrentWindow().close()')
            except Error as error:
                if 'Target page, context or browser has been closed' not in str(error):raise
            for _ in range(40):
                if native.exit_code()!=259:break
                await asyncio.sleep(.1)
            result['exitCode'] = native.exit_code()
            require(result['exitCode']==0, 'Native test did not close normally')
    except Exception as error:
        result['status']='FAIL';result['failure']=str(error)
        raise
    finally:
        native.release_handle()
        (output/'ui-check.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps({key:result[key] for key in ('status','native','demo','liveProviderTranslation','errors','screenshots','exitCode')}))


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--launch-proof',required=True)
    parser.add_argument('--evidence',required=True)
    parser.add_argument('--baseline',action='store_true')
    asyncio.run(run(parser.parse_args()))
