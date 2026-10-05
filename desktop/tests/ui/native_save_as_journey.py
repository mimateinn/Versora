"""Focused actual-native Save As observations; no file dialog or IPC replacement."""
import argparse
import asyncio
import hashlib
import json
from pathlib import Path
from playwright.async_api import async_playwright
from native_journey import comparable_path


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


async def run(args):
    evidence=Path(args.evidence).resolve()
    if evidence.exists():
        raise RuntimeError('Preserve existing evidence; use a new result path.')
    async with async_playwright() as p:
        browser=await p.chromium.connect_over_cdp(args.cdp)
        page=None
        for context in browser.contexts:
            for candidate in context.pages:
                if await candidate.evaluate('Boolean(window.__TAURI__?.core?.invoke)'):
                    page=candidate
        if page is None or await page.title()!='Versora':
            raise RuntimeError('Owned native Tauri page required.')
        state=await page.evaluate("window.__TAURI__.core.invoke('get_state')")
        if not state.get('testMode') or comparable_path(state['dataDir'])!=comparable_path(args.data_dir):
            raise RuntimeError('Unexpected native profile or non-explicit Demo.')
        job=state['job']
        if job['status']!='done' or not job['usedDemo']:
            raise RuntimeError('An actual completed tagged Demo job is required.')
        source=Path(job['files'][0]['output'])
        result={'actualNative':True,'phase':args.phase,'jobId':job['id'],'jobOutputSHA256':digest(source),'jobOutputBytes':source.stat().st_size}
        if args.phase=='open':
            await page.get_by_test_id('export-result').click()
            responsive=await asyncio.wait_for(page.evaluate("window.__TAURI__.core.invoke('get_state')"),5)
            result.update(backendResponsive=responsive['job']['status']=='done',uiBusy=await page.get_by_test_id('main-content').get_attribute('aria-busy'))
        else:
            await page.wait_for_function("() => document.querySelector('[data-testid=main-content]')?.getAttribute('aria-busy')==='false'",timeout=30000)
            toast=page.get_by_test_id('toast')
            result['uiBusyCleared']=True
            result['toastTitle']=await toast.get_attribute('title')
            if args.phase=='cancelled':
                result['noSuccessToast']=await toast.is_hidden()
                if not result['noSuccessToast']:
                    raise RuntimeError('Cancel reported a success toast.')
            else:
                actual=Path(result['toastTitle'])
                allowed=Path(args.allowed_dir).resolve()
                if actual.parent.resolve()!=allowed or not actual.is_file():
                    raise RuntimeError(f'Native returned path outside verified synthetic folder or missing: {actual}')
                result.update(returnedPath=str(actual),bytes=actual.stat().st_size,sha256=digest(actual),unicodePath=any(ord(c)>127 for c in str(actual)))
                if result['sha256']!=result['jobOutputSHA256']:
                    raise RuntimeError('Exported bytes differ from actual job output.')
                if args.preexisting:
                    existing=Path(args.preexisting)
                    result.update(preexistingPath=str(existing),preexistingSHA256=digest(existing),collisionReturnedDifferentPath=actual.resolve()!=existing.resolve())
                    if result['preexistingSHA256']!=args.preexisting_sha256.lower() or not result['collisionReturnedDifferentPath'] or not actual.name.startswith(existing.stem):
                        raise RuntimeError('Preexisting synthetic file changed or export silently reused its path.')
                await page.screenshot(path=str(evidence.with_suffix('.png')))
            result['status']='PASS'
        evidence.parent.mkdir(parents=True,exist_ok=True)
        evidence.write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
        await browser.close()
        return result


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--cdp',required=True)
    parser.add_argument('--data-dir',required=True)
    parser.add_argument('--evidence',required=True)
    parser.add_argument('--phase',choices=['open','cancelled','verify'],required=True)
    parser.add_argument('--allowed-dir')
    parser.add_argument('--preexisting')
    parser.add_argument('--preexisting-sha256')
    args=parser.parse_args()
    try:
        print(json.dumps(asyncio.run(run(args)),ensure_ascii=True,indent=2))
    except Exception as error:
        path=Path(args.evidence)
        if not path.exists():
            path.parent.mkdir(parents=True,exist_ok=True)
            path.write_text(json.dumps({'status':'FAIL','phase':args.phase,'error':str(error)},indent=2),encoding='utf-8')
        raise
