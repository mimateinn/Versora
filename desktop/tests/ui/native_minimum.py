"""Check layout in a truly resized native HWND; never emulate a browser viewport."""
import argparse
import asyncio
import json
from pathlib import Path
from playwright.async_api import async_playwright


async def run(args):
    directory=Path(args.evidence).resolve()
    directory.mkdir(parents=True,exist_ok=True)
    result={'native':False,'checks':[],'browserViewportEmulation':False}
    async with async_playwright() as p:
        browser=await p.chromium.connect_over_cdp(args.cdp)
        page=None
        for context in browser.contexts:
            for candidate in context.pages:
                if await candidate.evaluate('Boolean(window.__TAURI__?.core?.invoke)'):
                    page=candidate
                    break
        if page is None:
            raise RuntimeError('Actual native Tauri context required.')
        result['native']=True
        try:
            dimensions=await page.evaluate('() => ({width:window.innerWidth,height:window.innerHeight})')
            if dimensions['width']!=800:
                raise RuntimeError('Resize the actual native window to its 800 logical pixel minimum before this check.')
            await page.get_by_test_id('nav-settings').click()
            for pane in ['purposes','keys','order','glossary','appearance','updates']:
                await page.get_by_test_id('settings-pane-'+pane).click()
                dimensions=await page.evaluate('() => ({width:window.innerWidth,scrollWidth:document.documentElement.scrollWidth})')
                if dimensions['scrollWidth']>dimensions['width']:
                    raise RuntimeError(f'Horizontal overflow in {pane}: {dimensions}')
                result['checks'].append({'pane':pane,**dimensions})
            await page.get_by_test_id('settings-pane-appearance').click()
            path=page.get_by_test_id('data-directory')
            assert await path.get_attribute('title')==await path.text_content()
            assert await path.evaluate('node=>getComputedStyle(node).textOverflow')=='ellipsis'
            result['dataPathEllipsisAndTooltip']=True
            result['status']='PASS'
        except Exception as error:
            result.update(status='FAILED',error=str(error))
            await page.screenshot(path=str(directory/'native-minimum-failure-client.png'))
            raise
        finally:
            (directory/'native-minimum.json').write_text(json.dumps(result,indent=2),encoding='utf-8')
            await browser.close()
    return result


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--cdp',required=True)
    parser.add_argument('--evidence',required=True)
    print(json.dumps(asyncio.run(run(parser.parse_args())),indent=2))
