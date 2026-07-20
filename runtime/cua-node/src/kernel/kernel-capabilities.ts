import capabilityContract from "../../contracts/runtime-capabilities.contract.json";

type CapabilityContract = typeof capabilityContract;

export const RUNTIME_CAPABILITY_CONTRACT: CapabilityContract = capabilityContract;

const serializedContract = JSON.stringify(capabilityContract);

export const KERNEL_CAPABILITY_DECLARATIONS_SOURCE = String.raw`
const RUNTIME_CAPABILITIES = Object.freeze(${serializedContract});
const NODE_WEB_GLOBAL_NAMES = Object.freeze(Object.values(RUNTIME_CAPABILITIES.node_global_descriptors).flat());
const CANVAS_GLOBAL_NAMES = Object.freeze([...RUNTIME_CAPABILITIES.canvas_lazy_globals]);
`;

export const KERNEL_PROCESS_FACADES_SOURCE = String.raw`
const compatibleHrtime = (...args) => process.hrtime(...args);
compatibleHrtime.bigint = () => process.hrtime.bigint();
Object.freeze(compatibleHrtime);
const compatibleMemoryUsage = () => process.memoryUsage();
compatibleMemoryUsage.rss = () => process.memoryUsage.rss();
Object.freeze(compatibleMemoryUsage);
const compatibleStdioMetadata = (stream) => Object.freeze({ fd: stream.fd, isTTY: stream.isTTY === true });
const compatibleProcessFields = Object.freeze({
  arch: process.arch, argv: cloneFreeze([...process.argv]), cwd: () => cwd,
  execPath: process.execPath, getuid: typeof process.getuid === 'function' ? () => process.getuid() : undefined,
  hrtime: compatibleHrtime, memoryUsage: compatibleMemoryUsage,
  nextTick: (callback, ...args) => process.nextTick(callback, ...args), pid: process.pid,
  platform: process.platform, release: cloneFreeze(process.release),
  resourceUsage: typeof process.resourceUsage === 'function' ? () => process.resourceUsage() : undefined,
  stderr: compatibleStdioMetadata(process.stderr), stdin: compatibleStdioMetadata(process.stdin),
  stdout: compatibleStdioMetadata(process.stdout), cpuUsage: (previous) => process.cpuUsage(previous),
  uptime: () => process.uptime(), version: process.version, versions: cloneFreeze(process.versions),
});

const contextBreakingBuiltinNames = new Set(RUNTIME_CAPABILITIES.builtin_modules.context_breaking_denied);

function normalizeBuiltinModuleName(name) {
  return name.startsWith('node:') ? name.slice(5) : name;
}

function assertBuiltinModuleAllowed(name) {
  const normalized = normalizeBuiltinModuleName(name);
  if (contextBreakingBuiltinNames.has(normalized)) throw new Error('node_repl builtin module is unavailable: ' + normalized);
  return normalized;
}

function compatibleBuiltinModule(name, processFacade, trusted = false) {
  if (typeof name !== 'string' || name.length === 0) throw new TypeError('process.getBuiltinModule requires a non-empty module name');
  const normalized = normalizeBuiltinModuleName(name);
  if (normalized === 'process') return processFacade;
  if (normalized === 'module') return createProcessModuleFacade(trusted);
  if (contextBreakingBuiltinNames.has(normalized)) return undefined;
  if (!builtinModuleNames.has(normalized) && !builtinModuleNames.has('node:' + normalized)) return undefined;
  if (typeof process.getBuiltinModule === 'function') return process.getBuiltinModule(normalized);
  return createRequire(import.meta.url)(normalized);
}

function createProcessModuleFacade(trusted) {
  return Object.freeze({
    builtinModules: Object.freeze([...builtinModules]),
    createRequire(referrer) {
      const file = normalizeCreateRequireReferrer(referrer);
      const packageRoot = findPackageRoot(file);
      if (!trusted && (packageRoot === null || !isConfiguredPackageRoot(packageRoot.root))) throw new Error('package createRequire() must be rooted in a configured package');
      const context = trusted ? createVmContext(trustedSurface, 'trusted') : packageContext;
      return createCommonJsRequire(file, context, trusted, !trusted, createSyntheticCommonJsParent(file));
    },
  });
}

function isConfiguredPackageRoot(packageRoot) {
  return [...nodeModuleDirs, ...addedModuleDirs, cwd].some((root) => trustedPathContains(packageRoot, join(root, 'node_modules')));
}

function normalizeCreateRequireReferrer(referrer) {
  const file = referrer instanceof URL ? fileURLToPath(referrer) : typeof referrer === 'string' && referrer.startsWith('file:') ? fileURLToPath(referrer) : referrer;
  if (typeof file !== 'string' || !isAbsolute(file)) throw new TypeError('createRequire() requires an absolute path or file URL');
  return file;
}

function createSyntheticCommonJsParent(file) {
  return { children: [], exports: {}, filename: file, id: file, loaded: true, parent: null, path: dirname(file), paths: [] };
}

function freezeProcessFacade(value) {
  Object.defineProperty(value, Symbol.toStringTag, { configurable: false, enumerable: false, value: 'process', writable: false });
  return Object.freeze(value);
}

const packageProcess = freezeProcessFacade({ ...compatibleProcessFields, env: cloneFreeze(packageEnv), getBuiltinModule: (name) => compatibleBuiltinModule(name, packageProcess, false) });
const trustedProcess = freezeProcessFacade({
  ...compatibleProcessFields, env: trustedEnv,
  getBuiltinModule: (name) => compatibleBuiltinModule(name, trustedProcess, true),
  once(event, listener) {
    if (event !== 'exit' || typeof listener !== 'function') throw new TypeError('trusted process only supports once("exit", listener)');
    process.once('exit', listener); return trustedProcess;
  },
  off(event, listener) {
    if (event !== 'exit' || typeof listener !== 'function') throw new TypeError('trusted process only supports off("exit", listener)');
    process.off('exit', listener); return trustedProcess;
  },
});

function mutableCommonJsProcess(env, trusted) {
  const listenerWrappers = new Map();
  function rememberListener(event, listener, wrapped) {
    let entry = listenerWrappers.get(event);
    if (entry === undefined) { entry = { listeners: new WeakMap(), count: 0 }; listenerWrappers.set(event, entry); }
    const wrappers = entry.listeners.get(listener) ?? [];
    wrappers.push(wrapped); entry.listeners.set(listener, wrappers); entry.count += 1;
  }
  function forgetListener(event, listener, wrapped) {
    const entry = listenerWrappers.get(event);
    const wrappers = entry?.listeners.get(listener);
    if (wrappers === undefined) return;
    const index = wrappers.lastIndexOf(wrapped);
    if (index === -1) return;
    wrappers.splice(index, 1); entry.count -= 1;
    if (wrappers.length === 0) entry.listeners.delete(listener);
    if (entry.count === 0) listenerWrappers.delete(event);
  }
  function addProcessListener(method, event, listener, once) {
    if (typeof listener !== 'function') throw new TypeError('process listener must be a function');
    const wrapped = (...args) => { if (once) forgetListener(event, listener, wrapped); return Reflect.apply(listener, facade, args); };
    rememberListener(event, listener, wrapped);
    try { process[method](event, wrapped); } catch (error) { forgetListener(event, listener, wrapped); throw error; }
    return facade;
  }
  function removeProcessListener(event, listener) {
    if (typeof listener !== 'function') throw new TypeError('process listener must be a function');
    const entry = listenerWrappers.get(event);
    const wrappers = entry?.listeners.get(listener);
    const wrapped = wrappers?.pop();
    if (wrapped !== undefined) entry.count -= 1;
    if (wrappers?.length === 0) entry.listeners.delete(listener);
    if (entry?.count === 0) listenerWrappers.delete(event);
    if (wrapped !== undefined) process.off(event, wrapped);
    return facade;
  }
  const facade = {
    ...compatibleProcessFields, env: { ...env }, getBuiltinModule: (name) => compatibleBuiltinModule(name, facade, trusted),
    addListener(event, listener) { return addProcessListener('addListener', event, listener, false); },
    on(event, listener) { return addProcessListener('on', event, listener, false); },
    once(event, listener) { return addProcessListener('once', event, listener, true); },
    off(event, listener) { return removeProcessListener(event, listener); },
    removeListener(event, listener) { return removeProcessListener(event, listener); },
  };
  for (const name of RUNTIME_CAPABILITIES.process_facades.package.forbidden) delete facade[name];
  Object.defineProperty(facade, Symbol.toStringTag, { configurable: false, enumerable: false, value: 'process', writable: false });
  return facade;
}

const commonJsPackageProcess = mutableCommonJsProcess(packageEnv, false);
const commonJsTrustedProcess = mutableCommonJsProcess(process.env, true);
`;
