"""Real custom-control interactions in the owned native WebView2 page."""
async def choose(page, test_id, value):
    carrier=page.get_by_test_id(test_id)
    trigger=page.get_by_test_id(test_id+'-trigger')
    if await trigger.count():
        await trigger.click()
        # Values are compared as DOM strings, so arbitrary project names do not
        # become selectors, JavaScript or markup.
        rows=page.locator('.custom-option')
        values=await rows.evaluate_all('nodes => nodes.map(node=>node.dataset.value)')
        if str(value) not in values:
            raise AssertionError(f'Custom option missing: {test_id}: {value}')
        await rows.nth(values.index(str(value))).click()
    else:
        await carrier.select_option(str(value))
    await page.wait_for_function('args => document.querySelector(`[data-testid="${args.id}"]`)?.value === args.value',arg={'id':test_id,'value':str(value)})
