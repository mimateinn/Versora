import argparse, asyncio, json
from pathlib import Path
from playwright.async_api import async_playwright

async def run(args):
    async with async_playwright() as p:
        browser=await p.chromium.connect_over_cdp(args.cdp)
        for context in browser.contexts:
            for page in context.pages:
                if await page.evaluate('Boolean(window.__TAURI__?.core?.invoke)'):
                    state=await page.evaluate("window.__TAURI__.core.invoke('get_state')")
                    evidence=Path(args.evidence).resolve();evidence.mkdir(parents=True,exist_ok=True)
                    await page.screenshot(path=str(evidence/'native-current.png'))
                    data={'url':page.url,'title':await page.title(),'dataDir':state.get('dataDir'),'version':state.get('version'),'testMode':state.get('testMode'),'selected':state.get('selected'),'body':(await page.locator('body').inner_text())[:1600]}
                    (evidence/'native-inspect.json').write_text(json.dumps(data,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
                    print(json.dumps(data,indent=2,ensure_ascii=True))
                    return
        raise RuntimeError('No native Tauri page')

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--cdp',required=True);parser.add_argument('--evidence',required=True)
    asyncio.run(run(parser.parse_args()))
