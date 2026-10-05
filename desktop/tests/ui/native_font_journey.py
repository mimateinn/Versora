"""Verify real bundled font shaping and 12-locale native layout without emulation."""
import argparse
import asyncio
import json
from pathlib import Path
from playwright.async_api import async_playwright
from native_journey import comparable_path


async def run(args):
    directory=Path(args.evidence).resolve()
    if (directory/'native-font-journey.json').exists():
        raise RuntimeError('Preserve prior native font evidence.')
    directory.mkdir(parents=True,exist_ok=True)
    result={'native':False,'browserViewportEmulation':False,'fontFaces':[],'renderedFonts':{},'layouts':[],'errors':[],'realTranslation':'NOT_RUN'}
    async with async_playwright() as p:
        browser=await p.chromium.connect_over_cdp(args.cdp)
        page=None
        for context in browser.contexts:
            for candidate in context.pages:
                if await candidate.evaluate('Boolean(window.__TAURI__?.core?.invoke)'):
                    page=candidate
        if page is None or await page.title()!='Versora':
            raise RuntimeError('Actual native Versora Tauri context required.')
        result['native']=True
        result['url']=page.url
        page.on('pageerror',lambda error:result['errors'].append(str(error)))
        state=await page.evaluate("window.__TAURI__.core.invoke('get_state')")
        if not state.get('testMode') or comparable_path(state['dataDir'])!=comparable_path(args.data_dir):
            raise RuntimeError('Expected isolated native Demo profile required.')
        result['dataDir']=state['dataDir']
        result['configuredProviders']=[item['id'] for item in state['providers'] if item.get('configured') and item['id']!='demo']
        try:
            size=await page.evaluate('() => ({width:innerWidth,height:innerHeight,devicePixelRatio,visualScale:visualViewport.scale})')
            result['nativeViewport']=size
            if size['width']!=800:
                raise RuntimeError('Resize the actual native HWND to 800 logical pixels first.')
            await page.evaluate('async () => {for(const style of ["normal","italic"]){for(const weight of [400,700]){await document.fonts.load(`${style} ${weight} 14px "Libron"`,"Translate Settings 0123456789 Tiếng Việt");}}await document.fonts.ready;}')
            result['fontFaces']=await page.evaluate('() => [...document.fonts].filter(face=>face.family.replaceAll(String.fromCharCode(34),"")==="Libron").map(face=>({family:face.family,weight:face.weight,style:face.style,status:face.status}))')
            mapped={(face['style'],face['weight']) for face in result['fontFaces'] if face['status']=='loaded'}
            if mapped!={('normal','400'),('italic','400'),('normal','700'),('italic','700')}:
                raise RuntimeError('Four actual static Libron faces did not load.')
            cdp=await page.context.new_cdp_session(page)
            await cdp.send('DOM.enable')
            await cdp.send('CSS.enable')
            async def fonts(selector):
                root=await cdp.send('DOM.getDocument',{'depth':0})
                node=await cdp.send('DOM.querySelector',{'nodeId':root['root']['nodeId'],'selector':selector})
                if not node['nodeId']:
                    raise RuntimeError(f'Missing native font target {selector}')
                return (await cdp.send('CSS.getPlatformFontsForNode',{'nodeId':node['nodeId']}))['fonts']
            # Temporary offscreen glyph probes exercise four real native faces and
            # fallback shaping. They are removed before any product screenshot.
            await page.evaluate('() => {const host=document.createElement("div");host.id="native-font-probes";host.setAttribute("aria-hidden","true");host.style.cssText="position:absolute;left:-10000px;top:0";for(const style of ["normal","italic"]){for(const weight of [400,700]){const node=document.createElement("span");node.id=`font-probe-${style}-${weight}`;node.textContent="Translate Settings 0123456789 Tiếng Việt";node.style.cssText=`font-family:Libron;font-size:14px;font-weight:${weight};font-style:${style};font-synthesis:none`;host.append(node);}}document.body.append(host);}')
            for style in ('normal','italic'):
                for weight in (400,700):
                    key=f'{style}-{weight}'
                    used=await fonts('#font-probe-'+key)
                    result['renderedFonts'][key]=used
                    if not any(font.get('isCustomFont') and font['familyName']=='Libron' and font['glyphCount']>0 for font in used):
                        raise RuntimeError(f'Actual Libron glyph shaping absent for {key}: {used}')
            await page.evaluate('() => document.querySelector("#native-font-probes").remove()')
            await page.get_by_test_id('nav-settings').click()
            await page.get_by_test_id('settings-pane-appearance').click()
            await page.get_by_test_id('interface-language').select_option('en')
            await page.wait_for_function("() => document.documentElement.lang==='en'")
            result['renderedFonts']['brand-bold']=await fonts('.brand b')
            result['renderedFonts']['navigation-regular']=await fonts('[data-testid="nav-translate"]')
            result['renderedFonts']['path-monospace']=await fonts('[data-testid="data-directory"]')
            if any(font.get('isCustomFont') and font['familyName']=='Libron' for font in result['renderedFonts']['path-monospace']):
                raise RuntimeError('Monospace path unexpectedly uses Libron.')
            result['computedContract']=await page.evaluate('() => ({body:getComputedStyle(document.body).fontFamily,bodyWeight:getComputedStyle(document.body).fontWeight,buttonWeight:getComputedStyle(document.querySelector("[data-testid=nav-translate]")).fontWeight,strongWeight:getComputedStyle(document.querySelector(".brand b")).fontWeight,synthesis:getComputedStyle(document.body).fontSynthesis,path:getComputedStyle(document.querySelector("[data-testid=data-directory]")).fontFamily})')
            if result['computedContract']['buttonWeight']!='400' or result['computedContract']['strongWeight']!='700' or result['computedContract']['synthesis']!='none':
                raise RuntimeError('Native computed typography contract differs from true 400/700 without synthesis.')
            locales=['en','zh-Hant','zh-Hans','ja','ko','fr','de','es','pt','vi','th','id']
            panes=['purposes','keys','order','glossary','appearance','updates']
            measure='''() => {
              const issues=[];
              for(const button of document.querySelectorAll('button')){
                const bounds=button.getBoundingClientRect();
                if(!bounds.width||!bounds.height||getComputedStyle(button).visibility==='hidden')continue;
                const nodes=document.createTreeWalker(button,NodeFilter.SHOW_TEXT);
                for(let node=nodes.nextNode();node;node=nodes.nextNode()){
                  if(!node.textContent.trim()||node.parentElement.closest('svg'))continue;
                  const range=document.createRange();range.selectNodeContents(node);
                  for(const text of range.getClientRects()){
                    if(text.width&&text.height&&(text.left<bounds.left-1||text.right>bounds.right+1||text.top<bounds.top-1||text.bottom>bounds.bottom+1))issues.push({id:button.dataset.testid,text:node.textContent,reason:'button text escapes bounds'});
                  }
                }
              }
              return {width:innerWidth,scrollWidth:document.documentElement.scrollWidth,issues};
            }'''
            for theme in ('light','dark'):
                await page.get_by_test_id('settings-pane-appearance').click()
                await page.get_by_test_id('appearance-theme').select_option(theme)
                await page.wait_for_function('theme=>document.documentElement.dataset.theme===theme',arg=theme)
                for locale in locales:
                    await page.get_by_test_id('settings-pane-appearance').click()
                    await page.get_by_test_id('interface-language').select_option(locale)
                    await page.wait_for_function('locale=>document.documentElement.lang===locale',arg=locale)
                    await page.evaluate('async () => await document.fonts.ready')
                    heading=await fonts('[data-testid="settings-content-appearance"] h1')
                    result['renderedFonts'][f'heading-{locale}-{theme}']=heading
                    for pane in panes:
                        await page.get_by_test_id('settings-pane-'+pane).click()
                        await page.get_by_test_id('settings-content-'+pane).wait_for()
                        dimensions=await page.evaluate(measure)
                        record={'theme':theme,'locale':locale,'pane':pane,**dimensions}
                        result['layouts'].append(record)
                        if dimensions['scrollWidth']>dimensions['width'] or dimensions['issues']:
                            await page.screenshot(path=str(directory/f'failure-{theme}-{locale}-{pane}.png'))
                            raise RuntimeError(f'Native font layout failed: {record}')
                    await page.get_by_test_id('nav-translate').click()
                    dimensions=await page.evaluate(measure)
                    result['layouts'].append({'theme':theme,'locale':locale,'pane':'translate',**dimensions})
                    if dimensions['scrollWidth']>dimensions['width'] or dimensions['issues']:
                        raise RuntimeError(f'Translation controls overflow for {theme}/{locale}: {dimensions}')
                    await page.get_by_test_id('nav-settings').click()
                    if locale in ('en','zh-Hant','vi','th'):
                        await page.get_by_test_id('settings-pane-appearance').click()
                        await page.screenshot(path=str(directory/f'native-libron-{theme}-{locale}-client.png'))
                    print(json.dumps({'nativeFontProgress':f'{theme}/{locale}','layouts':len(result['layouts'])}),flush=True)
            await page.get_by_test_id('settings-pane-appearance').click()
            await page.get_by_test_id('interface-language').select_option('en')
            await page.get_by_test_id('appearance-theme').select_option('light')
            await page.get_by_test_id('nav-translate').click()
            result['status']='PASS'
        except Exception as error:
            result.update(status='FAIL',error=str(error))
            await page.evaluate('() => document.querySelector("#native-font-probes")?.remove()')
            await page.screenshot(path=str(directory/'native-font-failure-client.png'))
            raise
        finally:
            (directory/'native-font-journey.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
            await browser.close()
    return {'status':result['status'],'native':True,'layouts':len(result['layouts']),'fonts':len(result['fontFaces']),'errors':result['errors']}


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--cdp',required=True)
    parser.add_argument('--data-dir',required=True)
    parser.add_argument('--evidence',required=True)
    print(json.dumps(asyncio.run(run(parser.parse_args())),indent=2))
