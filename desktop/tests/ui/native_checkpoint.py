"""Capture startup or Demo checkpoints from the actual native WebView2 only."""
import argparse
import asyncio
import hashlib
import json
from pathlib import Path
from playwright.async_api import async_playwright
from native_journey import comparable_path


async def run(args):
    directory=Path(args.evidence).resolve()
    directory.mkdir(parents=True,exist_ok=True)
    async with async_playwright() as p:
        browser=await p.chromium.connect_over_cdp(args.cdp)
        page=None
        for context in browser.contexts:
            for candidate in context.pages:
                if await candidate.evaluate("Boolean(window.__TAURI__?.core?.invoke)"):
                    page=candidate
                    break
        if page is None or await page.title()!='Versora':
            raise RuntimeError('A verified native Tauri window is required.')
        state=await page.evaluate("window.__TAURI__.core.invoke('get_state')")
        if not state.get('testMode') or comparable_path(state['dataDir'])!=comparable_path(args.data_dir):
            raise RuntimeError('Explicit Demo and the expected isolated profile are required.')
        await page.get_by_test_id('nav-settings').click()
        await page.get_by_test_id('settings-pane-appearance').click()
        await page.get_by_test_id('interface-language').select_option('en')
        await page.wait_for_function("() => document.documentElement.lang==='en'")
        await page.get_by_test_id('appearance-theme').select_option(args.theme)
        await page.wait_for_function("theme => document.documentElement.dataset.theme===theme",arg=args.theme)
        await page.get_by_test_id('nav-translate').click()
        if args.stage=='demo':
            # Exercise the real negative boundary, then prove context clears its alert.
            await page.get_by_test_id('nav-settings').click()
            await page.get_by_test_id('settings-pane-keys').click()
            await page.get_by_test_id('edit-provider-openai').click()
            await page.get_by_test_id('save-provider-openai').click()
            await page.wait_for_function("() => document.querySelector('[data-testid=main-content]')?.getAttribute('aria-busy')==='false'")
            await page.get_by_test_id('nav-translate').click()
            assert await page.get_by_test_id('toast').is_hidden()
            assert state['selected']
            original={item['path']:hashlib.sha256(Path(item['path']).read_bytes()).hexdigest() for item in state['selected']}
            await page.get_by_test_id('translator').select_option('demo')
            await page.get_by_test_id('translate-start').click()
            await page.wait_for_function("() => document.querySelector('[data-testid=job-card]')?.dataset.jobStatus==='done'",timeout=30000)
            assert await page.get_by_test_id('job-title').inner_text()=='Demo completed'
            assert await page.get_by_test_id('job-demo-warning').is_visible()
            assert await page.get_by_test_id('toast').is_hidden()
            state=await page.evaluate("window.__TAURI__.core.invoke('get_state')")
            assert state['job']['usedDemo'] is True
            assert all(hashlib.sha256(Path(path).read_bytes()).hexdigest()==digest for path,digest in original.items())
            await page.get_by_test_id('job-card').scroll_into_view_if_needed()
        else:
            await page.evaluate('() => {document.querySelector("[data-testid=app-shell]").scrollTop=0;}')
        screenshot=directory/f'native-{args.stage}-{args.theme}-client.png'
        await page.screenshot(path=str(screenshot))
        result={'native':True,'stage':args.stage,'theme':args.theme,'url':page.url,'dataDir':state['dataDir'],'version':state['version'],'testMode':state['testMode'],'paidTranslation':'NOT_RUN','clientScreenshot':str(screenshot),'job':state.get('job')}
        (directory/f'native-{args.stage}-{args.theme}.json').write_text(json.dumps(result,indent=2),encoding='utf-8')
        await browser.close()
        return {'native':True,'stage':args.stage,'theme':args.theme,'screenshot':str(screenshot),'usedDemo':state.get('job',{}).get('usedDemo') if state.get('job') else None}


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--cdp',required=True)
    parser.add_argument('--data-dir',required=True)
    parser.add_argument('--evidence',required=True)
    parser.add_argument('--stage',choices=['startup','demo'],required=True)
    parser.add_argument('--theme',choices=['light','dark'],default='light')
    args=parser.parse_args()
    print(json.dumps(asyncio.run(run(args)),indent=2))
