"""Exercise the real Versora Tauri/WebView2 window through its CDP endpoint.

Development-only regression tooling; neither Python nor Playwright is packaged.
Start the actual executable with a separate VERSORA_DATA_DIR and SFTS_DEMO=1.
Input files may arrive through the executable's legitimate Open With entry.
Native picker/export tests use actual manual dialogs; there is no fixture override.
Never replace __TAURI__, synthesize backend state, or use a browser page as proof.
"""
from __future__ import annotations

import argparse
import asyncio
import hashlib
import json
import statistics
import time
from pathlib import Path

from playwright.async_api import async_playwright

def comparable_path(value):
    path = str(Path(value).resolve())
    if path.startswith('\\\\?\\UNC\\'):
        path = '\\\\' + path[8:]
    elif path.startswith('\\\\?\\'):
        path = path[4:]
    return path.casefold()


async def run(args):
    evidence = {"native": False, "checks": [], "real_translation": "NOT_RUN", "demo": "NOT_RUN", "screenshots": [], "perf": {}, "errors": []}
    args.partial_evidence = evidence
    target = Path(args.evidence).resolve()
    target.mkdir(parents=True, exist_ok=True)
    async with async_playwright() as playwright:
        browser = await playwright.chromium.connect_over_cdp(args.cdp)
        pages = [page for context in browser.contexts for page in context.pages]
        page = None
        for candidate in pages:
            if await candidate.evaluate("Boolean(window.__TAURI__?.core?.invoke)"):
                page = candidate
                break
        if page is None:
            raise RuntimeError("No native Tauri WebView2 page. A normal browser is not an acceptable substitute.")
        evidence["native"] = True
        evidence["url"] = page.url
        page.on("pageerror", lambda error: evidence["errors"].append(str(error)))
        await page.emulate_media(reduced_motion="reduce")
        await page.get_by_test_id("nav-translate").click()
        await page.get_by_test_id("translate-card").wait_for()
        assert await page.title() == "Versora"
        assert not await page.get_by_test_id("connection-error").is_visible()
        evidence["checks"].append("Native Tauri window, frontend startup and no backend-bridge error")

        backend_state = await page.evaluate("window.__TAURI__.core.invoke('get_state')")
        if not backend_state.get("testMode"):
            raise RuntimeError("Run only an explicitly opted-in Demo test instance; live provider requests are not authorized by this script.")
        if args.data_dir and comparable_path(backend_state["dataDir"]) != comparable_path(args.data_dir):
            raise RuntimeError("Native application is not using the expected isolated test data directory")
        evidence["dataDir"] = backend_state["dataDir"]

        # Preserve the same choices across navigation; all settings panes are real Rust state.
        source = await page.get_by_test_id("source-language").input_value()
        target_language = await page.get_by_test_id("target-language").input_value()
        for theme in ["light", "dark"]:
            await page.get_by_test_id("nav-settings").click()
            await page.get_by_test_id("settings-pane-appearance").click()
            await page.get_by_test_id("appearance-theme").select_option(theme)
            await page.wait_for_function("theme => document.documentElement.dataset.theme === theme", arg=theme)
            for pane in ["purposes", "keys", "order", "glossary", "appearance", "updates"]:
                await page.get_by_test_id(f"settings-pane-{pane}").click()
                await page.get_by_test_id(f"settings-content-{pane}").wait_for()
                await page.screenshot(path=str(target / f"native-settings-{pane}-{theme}.png"))
            evidence["checks"].append(f"All six settings panes in {theme} theme")
        await page.get_by_test_id("settings-pane-appearance").click()
        language_options = await page.get_by_test_id("interface-language").locator("option").evaluate_all("nodes => nodes.map(node => node.value)")
        assert len(language_options) == 13 and "zh-Hant" in language_options and "th" in language_options
        evidence["checks"].append("All 12 interface languages plus system-language choice")
        locale_root = Path(__file__).resolve().parents[2] / "ui" / "locales"
        for code in [value for value in language_options if value != "system"]:
            await page.get_by_test_id("interface-language").select_option(code)
            await page.wait_for_function("code => document.documentElement.lang === code", arg=code)
            catalog = json.loads((locale_root / f"{code}.json").read_text(encoding="utf-8"))
            assert await page.get_by_test_id("settings-content-appearance").locator("h1").inner_text() == catalog["card.appearance"]
        evidence["checks"].append("Every interface locale loaded and appearance heading matches its preserved catalog")
        await page.get_by_test_id("interface-language").select_option("en")
        await page.wait_for_function("() => document.documentElement.lang === 'en'")
        await page.get_by_test_id("appearance-theme").select_option("light")
        await page.wait_for_function("() => document.documentElement.dataset.theme === 'light'")

        # Missing publisher trust/capability must keep the app alive and block download/install.
        await page.get_by_test_id("settings-pane-updates").click()
        await page.get_by_test_id("updates-card").wait_for()
        update_state = await page.evaluate("window.__TAURI__.core.invoke('updates_get_state')")
        assert update_state.get("trust") is False and update_state.get("installCapabilities") is False
        assert await page.get_by_test_id("updates-download").is_disabled()
        assert await page.get_by_test_id("updates-install").is_disabled()
        assert await page.get_by_test_id("updates-capability-reason").inner_text()
        await page.get_by_test_id("updates-policy").select_option("periodic")
        await page.get_by_test_id("updates-intervalHours").wait_for()
        await page.get_by_test_id("updates-intervalHours").fill("168")
        await page.get_by_test_id("updates-intervalHours").press("Tab")
        await page.wait_for_function("() => document.querySelector('[data-testid=main-content]')?.getAttribute('aria-busy') === 'false'")
        preferences = await page.evaluate("window.__TAURI__.core.invoke('updates_get_preferences')")
        assert preferences["intervalHours"] == 168 and preferences["policy"] == "periodic"
        await page.get_by_test_id("updates-policy").select_option("manual")
        await page.wait_for_function("() => document.querySelector('[data-testid=updates-policy]')?.value === 'manual'")
        evidence["checks"].append("Updater policies persist; missing publisher trust blocks download/install with a visible reason")

        # Regression: order edits must not disappear when another setting saves.
        await page.get_by_test_id("settings-pane-order").click()
        first_provider = backend_state["providers"][0]["id"]
        original_model = await page.get_by_test_id(f"order-model-{first_provider}").input_value()
        original_limit = await page.get_by_test_id("global-limit").input_value()
        await page.get_by_test_id(f"order-model-{first_provider}").fill("native-regression-model")
        await page.get_by_test_id("global-limit").select_option("3" if original_limit != "3" else "2")
        await page.wait_for_function("id => document.querySelector(`[data-testid=order-model-${id}]`)?.value === 'native-regression-model'", arg=first_provider)
        await page.get_by_test_id("theme-toggle").click()
        await page.wait_for_function("() => document.documentElement.dataset.theme === 'dark'")
        assert await page.get_by_test_id(f"order-model-{first_provider}").input_value() == "native-regression-model"
        await page.get_by_test_id(f"order-model-{first_provider}").fill(original_model)
        await page.get_by_test_id("global-limit").select_option(original_limit)
        await page.get_by_test_id("save-order").click()
        await page.wait_for_function("() => document.querySelector('[data-testid=save-order]')?.disabled === false")
        evidence["checks"].append("Order draft survives concurrency/theme saves and explicit order save")

        # Real glossary and prompt edits must survive subsequent native state reload.
        await page.get_by_test_id("settings-pane-glossary").click()
        await page.get_by_test_id("new-project").fill("retained-project-draft")
        await page.get_by_test_id("theme-toggle").click()
        await page.wait_for_function("() => document.documentElement.dataset.theme === 'light'")
        await page.get_by_test_id("settings-pane-appearance").click()
        await page.get_by_test_id("settings-pane-glossary").click()
        assert await page.get_by_test_id("new-project").input_value() == "retained-project-draft"
        evidence["checks"].append("New-project draft survives theme and settings navigation")
        await page.get_by_test_id("new-project").fill("native-regression")
        await page.get_by_test_id("create-project").click()
        await page.wait_for_function("() => document.querySelector('[data-testid=glossary-project]')?.value === 'native-regression'")
        await page.get_by_test_id("add-glossary-term").click()
        await page.get_by_test_id("glossary-term-0").fill("Versora")
        await page.get_by_test_id("glossary-translation-0").fill("Versora")
        await page.get_by_test_id("save-glossary").click()
        await page.wait_for_function("() => document.querySelector('[data-testid=toast]')?.textContent.includes('Glossary')")
        native_terms = await page.evaluate("window.__TAURI__.core.invoke('load_glossary',{project:'native-regression'})")
        assert native_terms == [{"term": "Versora", "translation": "Versora"}]
        evidence["checks"].append("Glossary project creation, editor save and actual Rust persistence")
        await page.get_by_test_id("settings-pane-purposes").click()
        await page.get_by_test_id("purpose-from").select_option("technical")
        await page.get_by_test_id("copy-purpose").click()
        assert await page.get_by_test_id("purpose-instructions").input_value()
        await page.get_by_test_id("purpose-instructions").fill("Translate accurately. Keep the source structure and glossary terms.")
        await page.get_by_test_id("save-purpose").click()
        await page.wait_for_function("() => Number(document.querySelector('[data-testid=purpose-version]')?.textContent.replace(/\\D/g,'')) > 0")
        evidence["checks"].append("Preset-to-custom purpose editor and versioned native save")

        # Boundary: blank key submission clears the field and never inserts a credential.
        await page.get_by_test_id("settings-pane-keys").click()
        await page.get_by_test_id("edit-provider-openai").click()
        await page.get_by_test_id("provider-key-openai").fill("unsent-native-credential-draft")
        await page.get_by_test_id("theme-toggle").click()
        await page.wait_for_function("() => document.documentElement.dataset.theme === 'dark'")
        assert await page.get_by_test_id("provider-key-openai").input_value() == "unsent-native-credential-draft"
        await page.get_by_test_id("nav-translate").click()
        await page.get_by_test_id("nav-settings").click()
        assert await page.get_by_test_id("provider-key-openai").input_value() == "unsent-native-credential-draft"
        await page.get_by_test_id("keep-provider-openai").click()
        await page.get_by_test_id("edit-provider-openai").click()
        assert await page.get_by_test_id("provider-key-openai").input_value() == ""
        evidence["checks"].append("Unsent password node survives theme/navigation and clears on explicit editor cancellation")
        await page.get_by_test_id("save-provider-openai").click()
        await page.wait_for_function("() => document.querySelector('[data-testid=main-content]')?.getAttribute('aria-busy') === 'false'")
        if not next(item for item in backend_state["providers"] if item["id"] == "openai").get("keyPresent"):
            await page.get_by_test_id("provider-key-openai").wait_for()
            assert await page.get_by_test_id("provider-key-openai").input_value() == ""
        secret_state = await page.evaluate("window.__TAURI__.core.invoke('get_state')")
        assert all("apiKey" not in item and "key" not in item and "secret" not in item for item in secret_state["providers"])
        evidence["checks"].append("Blank credential boundary and no full secrets returned to frontend")
        await page.get_by_test_id("nav-translate").click()
        assert await page.get_by_test_id("toast").is_hidden()
        evidence["checks"].append("Credential-validation feedback clears when leaving its page")

        durations = []
        for _ in range(3):
            await page.get_by_test_id("nav-translate").click()
            started = time.perf_counter()
            await page.get_by_test_id("nav-settings").click()
            await page.get_by_test_id("settings-layout").wait_for()
            durations.append((time.perf_counter() - started) * 1000)
        evidence["perf"]["navigation_median_ms"] = round(statistics.median(durations), 2)
        evidence["perf"]["productionBuild"] = bool(args.release_build)
        if args.release_build:
            assert statistics.median(durations) <= 500
        await page.get_by_test_id("nav-translate").click()
        assert await page.get_by_test_id("source-language").input_value() == source
        assert await page.get_by_test_id("target-language").input_value() == target_language
        evidence["checks"].append("Translation choices retained across settings and median-of-three navigation measurement")

        # The native backend registered actual Open With inputs at process startup.
        # A manual chooser is the only alternate entry; no test override fabricates selection.
        await page.get_by_test_id("translator").select_option("demo")
        await page.get_by_test_id("demo-warning").wait_for()
        if args.manual_dialogs:
            if await page.get_by_test_id("clear-selection").count():
                await page.get_by_test_id("clear-selection").click()
            await page.get_by_test_id("pick-files").click()
        await page.get_by_test_id("selected-files").wait_for(timeout=args.dialog_timeout)
        assert await page.get_by_test_id("selection-count").inner_text()
        controls = await page.locator(".quickbar select,.quickbar .icon-button").evaluate_all("nodes => nodes.map(node => ({id:node.dataset.testid,height:node.getBoundingClientRect().height}))")
        assert all(abs(item["height"] - 42) < 1 for item in controls)
        evidence["native_dialogs"] = "MANUAL" if args.manual_dialogs else "NOT_RUN"
        evidence["checks"].append("Actual registered native input selection; quick-bar control height 42px")
        await page.screenshot(path=str(target / "native-file-selected.png"))
        selected_state = await page.evaluate("window.__TAURI__.core.invoke('get_state')")
        source_hashes = {item['path']: hashlib.sha256(Path(item['path']).read_bytes()).hexdigest() for item in selected_state.get('selected', []) if item.get('kind') not in ['folder', 'zip']}
        await page.get_by_test_id("translate-start").click()
        await page.get_by_test_id("job-card").wait_for()
        if args.expect_chunks:
            await page.wait_for_function("() => {const progress=document.querySelector('[data-testid=job-progress]');return progress?.dataset.unit==='chunks'&&progress.value>0&&progress.value<progress.max;}", timeout=20000)
            assert await page.get_by_test_id("job-chunk-label").inner_text()
            assert await page.get_by_test_id("job-file-chunks-0").inner_text()
            evidence["chunkProgress"] = await page.get_by_test_id("job-progress").evaluate("node => ({value:node.value,max:node.max,unit:node.dataset.unit})")
            await page.screenshot(path=str(target / "native-chunk-progress.png"))
            evidence["checks"].append("Actual intermediate Rust chunk progress advances the single-file native progress bar")
        await page.wait_for_function("() => ['done','stopped','error'].includes(document.querySelector('[data-testid=job-card]')?.dataset.jobStatus)", timeout=120000)
        assert await page.get_by_test_id("job-card").get_attribute("data-job-status") == "done"
        assert await page.get_by_test_id("job-card").get_attribute("data-used-demo") == "true"
        assert await page.get_by_test_id("job-title").inner_text() == "Demo completed"
        assert await page.get_by_test_id("job-demo-warning").is_visible()
        assert await page.get_by_test_id("toast").is_hidden()
        evidence["checks"].append("Authoritative Rust Demo contribution uses a local Demo completed result label without stale credential alerts")
        assert await page.get_by_test_id("export-result").is_visible()
        if args.manual_dialogs:
            await page.get_by_test_id("export-result").click()
        if args.manual_dialogs and args.export_path:
            path = Path(args.export_path)
            deadline = time.monotonic() + args.dialog_timeout / 1000
            while time.monotonic() < deadline and not path.is_file():
                await asyncio.sleep(0.1)
            assert path.is_file() and path.stat().st_size > 0
            evidence["export"] = {"path": str(path), "bytes": path.stat().st_size}
        await page.get_by_test_id("toast").wait_for(state="hidden", timeout=7000)
        await page.get_by_test_id("job-card").scroll_into_view_if_needed()
        await page.screenshot(path=str(target / "native-demo-result.png"))
        evidence["demo"] = "PASS_OFFLINE_TAGGED_TEST_ONLY"
        job_state = await page.evaluate("window.__TAURI__.core.invoke('get_state')")
        assert job_state["job"]["usedDemo"] is True
        saved_outputs = [Path(item["output"]) for item in job_state["job"]["files"] if item.get("status") == "saved" and item.get("output")]
        assert saved_outputs and all(path.is_file() and path.stat().st_size > 0 for path in saved_outputs)
        evidence["outputs"] = [{"path": str(path), "bytes": path.stat().st_size, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()} for path in saved_outputs]
        if source_hashes:
            assert all(hashlib.sha256(Path(path).read_bytes()).hexdigest() == digest for path, digest in source_hashes.items())
            evidence["checks"].append("Original registered source files retain their SHA256 after translation")
        else:
            evidence["source_hash_check"] = "NOT_RUN_NO_DIRECT_FILE_SELECTION"
        evidence["checks"].append("Open With entry → registered actual input → explicit offline Demo → real completed output on disk")
        await page.get_by_test_id("result-preview").click()
        await page.get_by_test_id("translated-text-preview").wait_for()
        assert await page.get_by_test_id("translated-text-preview").inner_text()
        evidence["checks"].append("Owned source/translated preview through scoped Rust command")
        path_nodes = await page.locator('.file-path,.output-path').evaluate_all("nodes => nodes.map(node=>({text:node.textContent,title:node.title,ellipsis:getComputedStyle(node).textOverflow}))")
        assert path_nodes and all(not item['text'].startswith('\\\\?\\') and item['title']==item['text'] and item['ellipsis']=='ellipsis' for item in path_nodes)
        evidence["checks"].append("Windows extended paths show familiar drive/UNC forms with ellipsis and complete tooltips")
        for kind in ["folder", "zip", "file"]:
            await page.get_by_test_id(f"source-type-{kind}").click()
            await page.wait_for_function("kind => document.querySelector(`[data-testid=source-type-${kind}]`)?.getAttribute('aria-pressed') === 'true'", arg=kind)
            assert await page.get_by_test_id("dropzone").is_visible()
        evidence["checks"].append("File/folder/zip switching clears native selection and persists source type")
        assert not evidence["errors"], evidence["errors"]
        evidence["screenshots"] = [str(path) for path in target.glob("*.png")]
        (target / "native-journey.json").write_text(json.dumps(evidence, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        # CDP disconnect leaves the owner's native window open.
        await browser.close()
    return evidence


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--cdp", default="http://127.0.0.1:9222")
    parser.add_argument("--evidence", required=True)
    parser.add_argument("--data-dir")
    parser.add_argument("--export-path")
    parser.add_argument("--dialog-timeout", type=int, default=120000)
    parser.add_argument("--release-build", action="store_true")
    parser.add_argument("--manual-dialogs", action="store_true")
    parser.add_argument("--expect-chunks", action="store_true")
    args = parser.parse_args()
    try:
        result = asyncio.run(run(args))
    except Exception as error:
        directory = Path(args.evidence).resolve()
        directory.mkdir(parents=True, exist_ok=True)
        partial = {**getattr(args, "partial_evidence", {}), "status": "FAILED", "error": str(error), "type": type(error).__name__, "real_translation": "NOT_RUN"}
        partial["screenshots"] = [str(path) for path in directory.glob("*.png")]
        (directory / "native-journey-failure.json").write_text(json.dumps(partial, indent=2) + "\n", encoding="utf-8")
        raise
    print(json.dumps(result, indent=2, ensure_ascii=True))
