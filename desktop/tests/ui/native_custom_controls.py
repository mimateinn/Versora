"""Exercise shared controls in an actual owned Tauri/WebView2 window.

Uses the private Demo launch proof, real project creation and ordinary UI actions.
No provider request, owner profile, mocked bridge or injected job state is used.
"""
import argparse
import asyncio
import json
from pathlib import Path
from playwright.async_api import async_playwright, Error
from custom_control_helpers import choose
from native_chrome_journey import OwnedWindow, capture_full_window, owned_test_path, require
from native_journey import navigate, comparable_path


async def idle(page):
    await page.wait_for_function("() => document.querySelector('#content').getAttribute('aria-busy') !== 'true'")
    await asyncio.sleep(.12)


async def pane(page, name):
    await navigate(page, 'settings')
    await page.get_by_test_id('settings-pane-'+name).click()
    await idle(page)


async def geometry(page):
    return await page.locator('.custom-picker').evaluate("""box => {
      const r=box.getBoundingClientRect(),v=visualViewport;
      return {x:r.x,y:r.y,width:r.width,height:r.height,side:box.dataset.side,
        viewport:{x:v.offsetLeft,y:v.offsetTop,width:v.width,height:v.height,scale:v.scale},
        scroller:box.querySelector('.custom-picker-list').clientHeight,
        scrollHeight:box.querySelector('.custom-picker-list').scrollHeight};
    }""")


def unclipped(value):
    v=value['viewport']
    require(value['x'] >= v['x']+7 and value['y'] >= v['y']+7 and
            value['x']+value['width'] <= v['x']+v['width']-7 and
            value['y']+value['height'] <= v['y']+v['height']-7,
            f'Custom picker is clipped: {value}')


async def run(args):
    proof=json.loads(owned_test_path(args.launch_proof).read_text(encoding='utf-8-sig'))
    output=owned_test_path(args.evidence);require(not output.exists(),'Keep earlier test receipts')
    output.mkdir(parents=True)
    result={'sourceSha':proof['sourceSha'],'version':'0.3.1','status':'RUNNING','errors':[],
            'paidApiOrCliRequests':'NOT_RUN','ownerProfileUsed':False,'screenshots':[]}
    native=OwnedWindow(proof)
    try:
        async with async_playwright() as playwright:
            browser=await playwright.chromium.connect_over_cdp('http://127.0.0.1:'+str(proof['debugPort']))
            pages=[p for c in browser.contexts for p in c.pages if await p.evaluate('Boolean(window.__TAURI__?.core?.invoke)')]
            require(len(pages)==1,'One real owned native context required');page=pages[0]
            page.set_default_timeout(10000);page.on('pageerror',lambda error:result['errors'].append(str(error)))
            state=await page.evaluate("window.__TAURI__.core.invoke('get_state')")
            require(state['testMode'] and state['version']=='0.3.1' and comparable_path(state['dataDir'])==comparable_path(proof['dataDir']),'Wrong native version/profile')
            native.restore();native.place(1060,800)
            await pane(page,'appearance');await choose(page,'interface-language','en');await idle(page)
            await navigate(page,'translate');await idle(page)
            if await page.get_by_test_id('clear-selection').count():await page.get_by_test_id('clear-selection').click();await idle(page)
            require(await page.locator('select:visible').count()==0,'An OS select remains visible')
            require(await page.get_by_test_id('source-language-trigger').get_attribute('role')=='combobox','Missing combobox role')
            await choose(page,'target-language','ja');await idle(page)
            state=await page.evaluate("window.__TAURI__.core.invoke('get_state')")
            require(state['settings']['target_lang']=='ja','Custom selection did not reach real Rust settings')
            for _ in range(24):
                await page.get_by_test_id('source-language-trigger').click()
                await page.locator('.custom-picker-filter').press('Escape')
            require(await page.locator('.custom-picker').count()==0,'Repeated opening leaked a popup')
            require(await page.get_by_test_id('source-language-trigger').evaluate('node => node === document.activeElement'),'Escape did not restore focus')
            result['repeatedOpenEscape']=24
            await pane(page,'purposes')
            await page.get_by_test_id('purpose-from-trigger').click()
            listbox=page.locator('.custom-picker-list')
            require(await page.locator('.custom-picker-filter').count()==0,'Small list should keep typeahead')
            await listbox.press('t');await listbox.press('Enter')
            require(await page.get_by_test_id('purpose-from').input_value()=='technical','Typeahead/Enter did not select Technical')
            await page.get_by_test_id('purpose-from-trigger').click();await listbox.press('Home')
            first=await page.locator('.custom-option[data-active=true]').get_attribute('data-value')
            await listbox.press('End');last=await page.locator('.custom-option[data-active=true]').get_attribute('data-value')
            require(first!=last,'Home/End did not navigate');await listbox.press('ArrowUp');await listbox.press('ArrowDown')
            await listbox.press('Escape')
            # A controlled disabled option uses the real shared control, without
            # altering backend settings or pretending a provider result.
            await page.get_by_test_id('purpose-from').evaluate("node => node.options[1].disabled=true")
            await page.get_by_test_id('purpose-from-trigger').click();await listbox.press('Home');await listbox.press('ArrowDown')
            active=await page.locator('.custom-option[data-active=true]').get_attribute('data-value')
            blocked=await page.get_by_test_id('purpose-from').evaluate('node => node.options[1].value')
            require(active!=blocked,'Arrow navigation entered a disabled option')
            require(await page.locator('.custom-option[aria-disabled=true]').count()==1,'Disabled option missing accessible state')
            await listbox.press('Escape');await page.get_by_test_id('purpose-from').evaluate('node => node.options[1].disabled=false')
            result['keyboard']={'typeahead':'PASS','arrowsHomeEndEnterEscape':'PASS','disabledOptionSkipped':'PASS'}

            # Create genuine private projects, then reload the actual app state.
            await page.evaluate("""async () => {
              for(let n=0;n<140;n++)await window.__TAURI__.core.invoke('create_project',{name:'Native UI option '+String(n).padStart(3,'0')});
            }""")
            await page.reload();await page.get_by_test_id('translate-card').wait_for();await idle(page)
            await pane(page,'glossary');await page.get_by_test_id('glossary-project-trigger').click()
            require(await page.locator('.custom-option').count()>=140,'Real large project list missing')
            await page.locator('.custom-picker-filter').fill('no-such-option-9876')
            require(await page.locator('.custom-picker-empty').is_visible(),'No-result state missing')
            require(await page.locator('.custom-picker-filter').get_attribute('aria-activedescendant') is None,'No-result state has stale active option')
            await page.locator('.custom-picker-filter').fill('Native UI option 139');await page.locator('.custom-picker-filter').press('Enter');await idle(page)
            state=await page.evaluate("window.__TAURI__.core.invoke('get_state')")
            require(state['settings']['project']=='Native UI option 139','Filtered selection not saved by Rust')
            await page.get_by_test_id('glossary-project-trigger').click();await page.locator('.custom-picker-filter').press('Tab')
            require(await page.get_by_test_id('new-project').evaluate('node => node === document.activeElement'),'Tab did not continue to next page control')
            await page.get_by_test_id('glossary-project-trigger').click();await page.get_by_test_id('new-project').click()
            require(await page.locator('.custom-picker').count()==0 and await page.get_by_test_id('new-project').evaluate('node => node === document.activeElement'),'Outside dismissal stole focus')
            await page.get_by_test_id('glossary-project-trigger').click();await page.locator('.custom-picker-filter').press('End')
            await asyncio.sleep(.25);shape=await geometry(page);unclipped(shape)
            require(shape['scrollHeight']>shape['scroller'] and shape['scroller']<=245,'Large list lacks bounded scrolling')
            result['largeList']={'realPrivateProjects':140,'geometry':shape,'filterNoResultsTabOutside':'PASS'}
            capture_full_window(native,output/'large-project-list.png');result['screenshots'].append('large-project-list.png')
            await page.locator('.custom-picker-filter').press('Escape')

            result['layouts']=[]
            for width,height in ((1060,800),(800,640)):
                native.place(width,height)
                for mode,tone in (('standard','light'),('standard','dark'),('hologram','light'),('hologram','dark')):
                    await pane(page,'appearance')
                    await page.get_by_test_id('appearance-theme-'+mode).click();await idle(page)
                    await page.get_by_test_id('appearance-tone-'+tone).click();await idle(page)
                    await navigate(page,'translate');await idle(page)
                    await page.get_by_test_id('source-language-trigger').click();await asyncio.sleep(.25)
                    shape=await geometry(page);unclipped(shape)
                    result['layouts'].append({'logicalWindow':[width,height],'mode':mode,'tone':tone,'geometry':shape})
                    if width==800:
                        name=f'custom-list-{mode}-{tone}-800.png';capture_full_window(native,output/name);result['screenshots'].append(name)
                    await page.locator('.custom-picker-filter').press('Escape')
                    caption=page.get_by_test_id('window-minimize');await caption.hover();await asyncio.sleep(.18)
                    require(await page.locator('.custom-tip').is_hidden(),'Tooltip ignored the 380ms delay')
                    await page.locator('.custom-tip').wait_for(state='visible')
                    require(await page.locator('[title]').count()==0,'A duplicate native title tooltip remains')
                    require(await caption.get_attribute('aria-describedby')=='versora-custom-tip','Tooltip lacks accessible description')
                    tip=await page.locator('.custom-tip').bounding_box();view=await page.evaluate('() => ({w:innerWidth,h:innerHeight})')
                    require(tip['x']>=7 and tip['y']>=7 and tip['x']+tip['width']<=view['w']-7 and tip['y']+tip['height']<=view['h']-7,'Tooltip clipping')
                    if width==800 and mode=='hologram' and tone=='dark':
                        capture_full_window(native,output/'custom-caption-tooltip.png');result['screenshots'].append('custom-caption-tooltip.png')
                    await page.keyboard.press('Escape');require(await page.locator('.custom-tip').is_hidden(),'Escape did not dismiss tooltip')
                    await page.mouse.move(400,300)
            # Actual WebView page scaling, within this owned instance. No OS
            # display setting or emulated device/window dimensions are changed.
            session=await page.context.new_cdp_session(page)
            result['zoom']=[]
            for scale in (1.5,2.0):
                await session.send('Emulation.setPageScaleFactor',{'pageScaleFactor':scale})
                await page.get_by_test_id('source-language-trigger').click();await asyncio.sleep(.25)
                shape=await geometry(page);unclipped(shape);result['zoom'].append({'kind':'Owned WebView page-scale stress via CDP','scale':scale,'geometry':shape})
                await page.locator('.custom-picker-filter').press('Escape')
            await session.send('Emulation.setPageScaleFactor',{'pageScaleFactor':1.0})
            await pane(page,'appearance');await page.get_by_test_id('reduce-motion').set_checked(True);await idle(page)
            await navigate(page,'translate');await page.get_by_test_id('source-language-trigger').click()
            reduced=await page.locator('.custom-picker').evaluate('node => ({animation:getComputedStyle(node).animationName,transition:getComputedStyle(node).transitionDuration})')
            require(reduced['animation']=='none','Reduce motion retained picker movement');result['reducedMotion']=reduced
            await page.locator('.custom-picker-filter').press('Escape')
            require(not result['errors'],f'JavaScript errors: {result["errors"]}')
            result['status']='PASS_NATIVE_CUSTOM_CONTROLS';result['window']=native.snapshot()
            try:await page.evaluate('window.__TAURI__.window.getCurrentWindow().close()')
            except Error as error:
                if 'Target page, context or browser has been closed' not in str(error):raise
            for _ in range(60):
                if native.exit_code()!=259:break
                await asyncio.sleep(.1)
            result['exitCode']=native.exit_code();require(result['exitCode']==0,'Private native app did not exit normally')
    except Exception as error:
        result['status']='FAIL';result['failure']=str(error);raise
    finally:
        (output/'ui-check.json').write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
        native.close()
    print(json.dumps({'status':result['status'],'sourceSha':result['sourceSha'],'layouts':len(result['layouts']),'exitCode':result['exitCode']}))


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--launch-proof',required=True);parser.add_argument('--evidence',required=True)
    asyncio.run(run(parser.parse_args()))
