/** Real Chrome fixture driver. Import through the browser tool and pass its tab.
 * The model sees only read-only rendered DOM observations. Server state is used
 * after the action solely for scoring. This is not Rust resolver acceptance.
 */
import { readFile, writeFile } from 'node:fs/promises';
import { performance } from 'node:perf_hooks';

export async function observe(tab) {
  return await tab.playwright.evaluate(() => ({
    title: document.title,
    candidates: Array.from(document.querySelectorAll('main button')).map(el => {
      const r = el.getBoundingClientRect();
      return {
        id: el.getAttribute('data-ref'), role: 'button', label: el.textContent.trim(),
        context: el.closest('fieldset')?.querySelector('legend')?.textContent.trim() ?? '',
        disabled: el.disabled, visible: r.width > 0 && r.height > 0,
        bounds: { x: r.x, y: r.y, width: r.width, height: r.height },
      };
    }),
  }));
}

export async function validateAndClick(tab, observation, choice) {
  const observed = observation.candidates.find(c => c.id === choice);
  if (!observed || !/^[a-z0-9]+$/.test(choice)) return { status: 'blocked', reason: 'unknown_reference' };
  const current = await observe(tab);
  const matches = current.candidates.filter(c => c.id === choice);
  if (matches.length !== 1) return { status: 'blocked', reason: 'stale_or_ambiguous_reference' };
  const live = matches[0];
  if (live.label !== observed.label || live.context !== observed.context || live.role !== observed.role)
    return { status: 'blocked', reason: 'identity_changed' };
  if (live.disabled || !live.visible) return { status: 'blocked', reason: 'not_actionable' };
  const locator = tab.playwright.locator(`[data-ref="${choice}"]`);
  if (await locator.count() !== 1) return { status: 'blocked', reason: 'locator_not_unique' };
  if (!await locator.isEnabled() || !await locator.isVisible()) return { status: 'blocked', reason: 'not_actionable' };
  await locator.click({ timeoutMs: 3000 });
  await tab.playwright.getByText('Action recorded.', { exact: true }).waitFor({ state: 'visible', timeoutMs: 3000 });
  return { status: 'clicked', reference: choice };
}

async function jsonFetch(url, init) {
  const response = await fetch(url, init);
  if (!response.ok) throw new Error(`Fixture HTTP ${response.status}`);
  return await response.json();
}

export async function loadPilot(out) {
  const server = JSON.parse(await readFile(`${out}/server.json`, 'utf8'));
  const manifest = await jsonFetch(`${server.decision_origin}/manifest`);
  let records = [];
  try { records = JSON.parse(await readFile(`${out}/browser-records.json`, 'utf8')); } catch (error) { if (error.code !== 'ENOENT') throw error; }
  return { out, server, manifest, records, started: new Date().toISOString() };
}

export async function runCase(tab, pilot, index) {
  const c = pilot.manifest.cases[index];
  const routes = index % 2 ? ['hybrid', 'baseline'] : ['baseline', 'hybrid'];
  const pair = [];
  for (const route of routes) {
    if (pilot.records.some(r => r.case_id === c.id && r.route === route)) continue;
    const started = performance.now();
    const run = `${c.id}-${route}-${Date.now()}`;
    const record = { case_id: c.id, family: c.family, kind: c.kind, route, run, observation_count: 0, actions: 0 };
    try {
      let stage = performance.now();
      await tab.goto(`${pilot.server.fixture_origin}/case/${c.id}?run=${run}`);
      record.navigation_ms = performance.now() - stage;
      stage = performance.now();
      const observation = await observe(tab);
      record.observation = observation;
      record.observation_count++;
      record.observe_ms = performance.now() - stage;
      stage = performance.now();
      const decision = await jsonFetch(`${pilot.server.decision_origin}/decide`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ case_id: c.id, route, observation }),
      });
      record.decision = decision;
      record.decision_roundtrip_ms = performance.now() - stage;
      if (decision.status !== 'ok') throw new Error('Model decision failed');
      stage = performance.now();
      if (decision.choice !== 'abstain') {
        record.action = await validateAndClick(tab, observation, decision.choice);
        record.observation_count++;
        if (record.action.status === 'clicked') record.actions++;
        record.post_state = await tab.getAXState({ emit: false });
        record.observation_count++;
      } else record.action = { status: 'abstained' };
      record.validate_action_observe_ms = performance.now() - stage;
      stage = performance.now();
      record.fixture_state = await jsonFetch(`${pilot.server.fixture_origin}/state/${run}`);
      record.readback_ms = performance.now() - stage;
      record.success = ['matched', 'abstained'].includes(record.fixture_state.outcome);
      record.wrong_click = record.fixture_state.outcome === 'mismatched';
      record.status = 'ok';
    } catch (error) {
      record.status = 'error'; record.error = String(error);
    }
    record.total_ms = performance.now() - started;
    pilot.records.push(record); pair.push(record);
    await writeFile(`${pilot.out}/browser-records.json`, JSON.stringify(pilot.records, null, 2));
    if (record.status === 'error') throw new Error(`Stopped on ${c.id}/${route}: ${record.error}`);
  }
  return pair.map(r => ({ case: r.case_id, route: r.route, source: r.decision.source, success: r.success, wrong_click: r.wrong_click, total_ms: Math.round(r.total_ms) }));
}

export async function validationProbes(tab, pilot) {
  const c = pilot.manifest.cases[0];
  await tab.goto(`${pilot.server.fixture_origin}/case/${c.id}?run=validation-old`);
  const old = await observe(tab);
  await tab.goto(`${pilot.server.fixture_origin}/case/${c.id}?run=validation-new`);
  const stale = await validateAndClick(tab, old, old.candidates[0].id);
  await tab.getAXState({ emit: false });
  const currentState = await jsonFetch(`${pilot.server.fixture_origin}/state/validation-new`);
  await tab.goto(`${pilot.server.fixture_origin}/case/notification-05?run=validation-disabled`);
  const current = await observe(tab);
  const disabled = current.candidates.find(c => c.disabled);
  const unavailable = await validateAndClick(tab, current, disabled.id);
  await tab.getAXState({ emit: false });
  const disabledState = await jsonFetch(`${pilot.server.fixture_origin}/state/validation-disabled`);
  const result = { stale, currentState, unavailable, disabledState };
  if (stale.status !== 'blocked' || unavailable.status !== 'blocked' || currentState.clicked_keys.length || disabledState.clicked_keys.length)
    throw new Error('Live validation probe failed');
  await writeFile(`${pilot.out}/validation-probes.json`, JSON.stringify(result, null, 2));
  return result;
}
