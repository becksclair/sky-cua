import { createHash } from "node:crypto";
import { TRUSTED_MODULE_POLICY_KERNEL_SOURCE } from "../host/trusted-module-policy.ts";
import {
  KERNEL_CAPABILITY_DECLARATIONS_SOURCE,
  KERNEL_PROCESS_FACADES_SOURCE,
} from "./kernel-capabilities.ts";
import {
  KERNEL_MODULE_LOADING_SOURCE,
  KERNEL_MODULE_RESOLUTION_SOURCE,
} from "./kernel-module-loader.ts";
import { KERNEL_PROTOCOL_SOURCE } from "./kernel-protocol.ts";

/**
 * The kernel ships as source text and starts with the selected bundled Node executable, reserving
 * a private Node IPC channel. Child stdout is separate, so direct fd 1 writes cannot corrupt
 * MCP or kernel control. Keeping this source self contained also makes the
 * development and packaged launch paths identical.
 */
export const KERNEL_PROTOCOL_VERSION = "cua-kernel-control-v2" as const;

export const KERNEL_SOURCE = String.raw`
import { createHash, randomUUID, webcrypto } from 'node:crypto';
import { builtinModules, createRequire } from 'node:module';
import { AsyncLocalStorage } from 'node:async_hooks';
import { dirname, extname, isAbsolute, join, normalize, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { existsSync, readFileSync, statSync } from 'node:fs';
import * as vm from 'node:vm';
import * as util from 'node:util';
import { Buffer } from 'node:buffer';
import { performance } from 'node:perf_hooks';

const PROTOCOL = 'cua-kernel-control-v2';
${TRUSTED_MODULE_POLICY_KERNEL_SOURCE}
const builtinModuleNames = new Set([...builtinModules, ...builtinModules.map((name) => name.startsWith('node:') ? name.slice(5) : 'node:' + name)]);
const cwd = process.cwd();
const homeDir = typeof process.env.HOME === 'string' && process.env.HOME.length > 0 ? process.env.HOME : null;
const tmpDir = (await import('node:os')).tmpdir();
const executionStorage = new AsyncLocalStorage();
const nodeModuleDirs = normalizeRoots(process.env.NODE_REPL_NODE_MODULE_DIRS || '');
const trustedModulePolicy = createTrustedModulePolicy();
const runtimeMetadata = parseRuntimeMetadata(process.env.NODE_REPL_RUNTIME_METADATA);
const addedModuleDirs = [];
const addedModuleDirSet = new Set();
const cachedModules = new Map();
const commonJsModules = new Map();
const commonJsCache = Object.create(null);
const commonJsModuleChildren = new WeakMap();
const bindings = new Map();
let untrustedContext = null;
let cellCounter = 0;
let nativeRequestCounter = 0;
const bridgeWaiters = new Map();
const bridgeToken = randomUUID();
let runningExecution = null;
let canvasGlobalValues = null;

${KERNEL_CAPABILITY_DECLARATIONS_SOURCE}

function send(message) {
  if (typeof process.send !== 'function') throw new Error('kernel private IPC channel is unavailable');
  process.send({ version: PROTOCOL, ...message });
}

function normalizeRoots(value) {
  const delimiter = process.platform === 'win32' ? ';' : ':';
  const roots = [];
  const seen = new Set();
  for (const entry of value.split(delimiter)) {
    const trimmed = entry.trim();
    if (!trimmed) continue;
    const absolute = resolve(cwd, trimmed);
    const base = absolute.endsWith('node_modules') ? dirname(absolute) : absolute;
    if (!seen.has(base)) { seen.add(base); roots.push(base); }
  }
  return roots;
}

function cloneFreeze(value, seen = new WeakMap()) {
  if (value === null || typeof value !== 'object') return value;
  if (seen.has(value)) return seen.get(value);
  const clone = Array.isArray(value) ? [] : {};
  seen.set(value, clone);
  for (const [key, item] of Object.entries(value)) clone[key] = cloneFreeze(item, seen);
  return Object.freeze(clone);
}

function cloneJsonValue(value, label, ancestors = new WeakSet()) {
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) throw new TypeError(label + ' must be JSON-safe');
    return value;
  }
  if (typeof value !== 'object' || ancestors.has(value)) throw new TypeError(label + ' must be JSON-safe');
  ancestors.add(value);
  let result;
  if (Array.isArray(value)) result = value.map((item) => cloneJsonValue(item, label, ancestors));
  else {
    const prototype = Object.getPrototypeOf(value);
    if (prototype !== Object.prototype && prototype !== null && prototype?.constructor?.name !== 'Object') throw new TypeError(label + ' must contain only JSON values');
    result = {};
    for (const [key, item] of Object.entries(value)) result[key] = cloneJsonValue(item, label, ancestors);
  }
  ancestors.delete(value);
  return result;
}

function parseRuntimeMetadata(value) {
  if (typeof value !== 'string' || value.length === 0) return null;
  let parsed;
  try { parsed = JSON.parse(value); }
  catch { throw new Error('NODE_REPL_RUNTIME_METADATA must be valid JSON'); }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed) || parsed.version !== 1) {
    throw new Error('NODE_REPL_RUNTIME_METADATA must be a runtime metadata v1 object');
  }
  return cloneFreeze(parsed);
}

function inspect(value) {
  return typeof value === 'string' ? value : util.inspect(value, { depth: 4, colors: false, breakLength: Infinity });
}

function isPlainObject(value) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === null || prototype === Object.prototype || prototype?.constructor?.name === 'Object';
}

function activeExecution() {
  const exec = executionStorage.getStore();
  return exec && !exec.closed ? exec : null;
}

function outputWrite(value) {
  const exec = activeExecution();
  if (!exec) return;
  exec.events.push({ kind: 'write', text: inspect(value) });
}

function outputLine(...values) {
  const exec = activeExecution();
  if (!exec) return;
  exec.events.push({ kind: 'line', text: values.map(inspect).join(' ') + '\n' });
}

function outputText(events) {
  let value = events.map((event) => event.text).join('');
  if (value.endsWith('\n')) value = value.slice(0, -1);
  return value;
}

function dataUrlFromBytes(bytes, mimeType) {
  if (!bytes || bytes.byteLength === 0) throw new Error('nodeRepl.emitImage expected non-empty bytes');
  let mime = mimeType;
  if (!mime) {
    const view = Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    if (view.length >= 8 && view.subarray(0, 8).equals(Buffer.from('89504e470d0a1a0a', 'hex'))) mime = 'image/png';
    else if (view.length >= 3 && view.subarray(0, 3).equals(Buffer.from('ffd8ff', 'hex'))) mime = 'image/jpeg';
    else if (view.length >= 12 && view.subarray(0, 4).toString() === 'RIFF' && view.subarray(8, 12).toString() === 'WEBP') mime = 'image/webp';
    else throw new Error('nodeRepl.emitImage could not infer an image MIME type');
  }
  if (typeof mime !== 'string' || mime.trim().length === 0) throw new Error('nodeRepl.emitImage requires a non-empty MIME type');
  return 'data:' + mime + ';base64,' + Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength).toString('base64');
}

async function emitImage(value) {
  const exec = activeExecution();
  if (!exec) return;
  let imageUrl;
  if (typeof value === 'string') {
    if (!value.startsWith('data:') || value.length <= 5) throw new Error('nodeRepl.emitImage requires a non-empty data URL');
    imageUrl = value;
  } else if (value && typeof value === 'object' && 'bytes' in value) {
    const bytes = toBytes(value.bytes);
    imageUrl = dataUrlFromBytes(bytes, value.mimeType);
  } else {
    imageUrl = dataUrlFromBytes(toBytes(value));
  }
  const id = exec.id + '-emit-image-' + exec.imageCounter++;
  await bridgeRequest({ type: 'privileged_request', op: 'emit_image', id, image_url: imageUrl });
  exec.images.push(imageUrl);
}

function toBytes(value) {
  if (value instanceof ArrayBuffer || Object.prototype.toString.call(value) === '[object ArrayBuffer]') return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  throw new Error('nodeRepl.emitImage expected bytes or {bytes, mimeType}');
}

function responseMeta(value) {
  const exec = activeExecution();
  if (!exec) return;
  if (!isPlainObject(value)) throw new TypeError('nodeRepl.setResponseMeta expects a plain object');
  const safe = cloneJsonValue(value, 'nodeRepl.setResponseMeta');
  exec.responseMeta = { ...exec.responseMeta, ...safe };
}

function bridgeRequest(message) {
  const exec = activeExecution();
  if (!exec) return Promise.reject(new Error('nodeRepl operation is unavailable because no JavaScript call is active'));
  return bridgeRequestForExecution(message, exec);
}

function bridgeRequestForExecution(message, exec) {
  if (!exec) return Promise.reject(new Error('nodeRepl operation is unavailable because no JavaScript execution context is available'));
  const id = message.id || 'native-pipe-' + nativeRequestCounter++;
  const request = { ...message, id, token: bridgeToken, generation: bridgeToken, exec_id: exec.id };
  return new Promise((resolvePromise, rejectPromise) => {
    bridgeWaiters.set(id, { resolve: resolvePromise, reject: rejectPromise });
    send(request);
  });
}

function createNativePipe() {
  const connections = new Map();
  let connectionCounter = 0;
  function createConnection(path) {
    if (typeof path !== 'string' || path.length === 0) return Promise.reject(new TypeError('native pipe path must be a non-empty string'));
    const id = 'connection-' + connectionCounter++;
    const listeners = { data: new Set(), error: new Set(), close: new Set() };
    const state = { id, closed: false, error: null, listeners, pendingData: [], exec: activeExecution() };
    connections.set(id, state);
    return bridgeRequest({ type: 'privileged_request', op: 'native_pipe', native_op: 'connect', connection_id: id, path })
      .catch((error) => {
        state.closed = true;
        state.error = error instanceof Error ? error : new Error(String(error));
        connections.delete(id);
        throw state.error;
      })
      .then(() => Object.freeze({
        write(data) {
          if (state.closed) return;
          const execution = activeExecution() ?? executionStorage.getStore() ?? state.exec;
          if (execution !== null && execution !== undefined) state.exec = execution;
          const bytes = toBytes(data);
          void bridgeRequestForExecution({ type: 'privileged_request', op: 'native_pipe', native_op: 'write', connection_id: id, data_base64: Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength).toString('base64') }, execution).catch((error) => dispatch('error', error));
        },
        on(event, listener) {
          if (!(event in listeners) || typeof listener !== 'function') throw new TypeError('invalid native pipe listener');
          listeners[event].add(listener);
          if (event === 'data' && state.pendingData.length > 0) {
            const pendingData = state.pendingData.splice(0);
            for (const value of pendingData) invokeListener(state, listener, value);
          }
          if (state.closed && (event === 'close' || (event === 'error' && state.error !== null))) queueMicrotask(() => dispatch(event, state.error));
          return this;
        },
        off(event, listener) {
          if (event in listeners) listeners[event].delete(listener);
          return this;
        },
        end() {
          if (state.closed) return;
          state.closed = true;
          const execution = activeExecution() ?? executionStorage.getStore() ?? state.exec;
          void bridgeRequestForExecution({ type: 'privileged_request', op: 'native_pipe', native_op: 'close', connection_id: id }, execution).catch(() => undefined);
          dispatch('close', null);
        },
      }));
    function dispatch(event, value) {
      const stateListeners = listeners[event];
      if (!stateListeners) return;
      for (const listener of stateListeners) invokeListener(state, listener, value);
    }
  }
  function invokeListener(state, listener, value) {
    queueMicrotask(() => {
      const invoke = () => Promise.resolve(listener(value)).catch(() => undefined);
      return state.exec ? executionStorage.run(state.exec, invoke) : invoke();
    });
  }
  function receive(message) {
    const state = connections.get(message.connection_id);
    if (!state) return;
    if (runningExecution !== null && message.exec_id === runningExecution.id) {
      state.exec = runningExecution;
    }
    if (message.type === 'native_pipe_data') {
      const value = Buffer.from(message.data_base64, 'base64');
      if (state.listeners.data.size === 0) state.pendingData.push(value);
      else for (const listener of state.listeners.data) invokeListener(state, listener, value);
      return;
    }
    state.closed = true;
    state.error = message.error ? new Error(message.error) : null;
    if (state.error) for (const listener of state.listeners.error) invokeListener(state, listener, state.error);
    for (const listener of state.listeners.close) invokeListener(state, listener, state.error);
  }
  return Object.freeze({ createConnection, receive, closeAll() { for (const state of connections.values()) { state.closed = true; } connections.clear(); } });
}

const nativePipe = createNativePipe();
const publicEnv = {};
const publicEnvNames = new Set([
  ...(process.env.NODE_REPL_PUBLIC_ENV || '').split(',').filter(Boolean),
  'SKY_CUA_CODEX_BROWSER_SOCKET_PATH',
  'SKY_CUA_MCP_CALLER_PROVENANCE',
]);
for (const key of publicEnvNames) if (typeof process.env[key] === 'string') publicEnv[key] = process.env[key];
const trustedEnv = cloneFreeze({ ...process.env });
const packageEnv = {};
const packageEnvNames = new Set([
  ...Object.keys(publicEnv),
  'HOME',
  'TMPDIR',
  'XDG_CACHE_HOME',
  'XDG_RUNTIME_DIR',
  'OAI_SKY_CONFIG_PATH',
  'SKY_CUA_JS_CONFIG_PATH',
  'SKY_CUA_SERVICE_SOCKET_PATH',
  'NODE_REPL_REQUEST_META',
]);
for (const key of packageEnvNames) if (typeof process.env[key] === 'string') packageEnv[key] = process.env[key];

${KERNEL_PROCESS_FACADES_SOURCE}

function createUntrustedSurface() {
  const surface = {
    cwd,
    env: cloneFreeze(publicEnv),
    homeDir,
    tmpDir,
    get requestMeta() { return activeExecution()?.requestMeta ?? null; },
    write: outputWrite,
    setResponseMeta: responseMeta,
    emitImage,
    runtime: runtimeMetadata,
    loaders: createConvenienceLoaders(),
  };
  return Object.freeze(surface);
}

function createConvenienceLoaders() {
  const specifiers = Object.freeze({
    canvas: '@napi-rs/canvas',
    pdfjs: 'pdfjs-dist/legacy/build/pdf.mjs',
    pixelmatch: 'pixelmatch',
    playwright: 'playwright',
    sharp: 'sharp',
    tesseract: 'tesseract.js',
  });
  const loaders = {};
  for (const name of RUNTIME_CAPABILITIES.loader_promotion.fixed_names) {
    const specifier = specifiers[name];
    if (typeof specifier !== 'string') throw new Error('runtime capability contract names an unsupported fixed loader: ' + name);
    Object.defineProperty(loaders, name, {
      enumerable: true,
      value: async () => {
        if (untrustedContext === null) throw new Error('nodeRepl loader requires an active VM context');
        const referrer = pathToFileURL(join(cwd, '.node_repl_loader.mjs')).href;
        const module = await loadModule(specifier, referrer, untrustedContext, false, false);
        return module.namespace;
      },
    });
  }
  return Object.freeze(loaders);
}

async function withSuspendedTimeout(fn) {
  const exec = activeExecution();
  if (!exec) return await fn();
  send({ type: 'suspend_timeout', exec_id: exec.id });
  try { return await fn(); }
  finally { if (!exec.closed) send({ type: 'resume_timeout', exec_id: exec.id }); }
}

function createPackageSurface(untrusted) {
  const packageSurface = Object.create(untrusted);
  Object.defineProperty(packageSurface, 'withSuspendedTimeout', {
    value: withSuspendedTimeout,
    enumerable: true,
  });
  return Object.freeze(packageSurface);
}

function createTrustedSurface(untrusted) {
  const config = Object.freeze({
    readToml: (path) => bridgeRequest({ type: 'privileged_request', op: 'config', config_op: 'readToml', path }),
    writeToml: (path, value) => bridgeRequest({ type: 'privileged_request', op: 'config', config_op: 'writeToml', path, value }),
    read: (options) => bridgeRequest({ type: 'privileged_request', op: 'config', config_op: 'read', options }),
    readRequirements: () => bridgeRequest({ type: 'privileged_request', op: 'config', config_op: 'readRequirements' }),
    writeValue: (request) => bridgeRequest({ type: 'privileged_request', op: 'config', config_op: 'writeValue', request }),
    batchWrite: (request) => bridgeRequest({ type: 'privileged_request', op: 'config', config_op: 'batchWrite', request }),
  });
  const trusted = Object.create(untrusted);
  Object.defineProperties(trusted, {
    addAfterSubmittedCodeHook: { value: (request) => { const exec = activeExecution(); if (!exec) return; if (!request || typeof request.run !== 'function' || !Number.isInteger(request.timeoutMs) || request.timeoutMs < 1) throw new TypeError('invalid submitted-code hook'); exec.hooks.push(request); }, enumerable: true },
    gaasBrowserConfig: { get: () => cloneFreeze(parseConfig(process.env.NODE_REPL_GAAS_BROWSER_CONFIG || '{}')), enumerable: true },
    launchServices: { value: Object.freeze({ openApplication: (target) => { validateLaunchTarget(target); return bridgeRequest({ type: 'privileged_request', op: 'launch_service', target }); } }), enumerable: true },
    config: { value: config, enumerable: true },
    env: { value: trustedEnv, enumerable: true },
    createElicitation: { value: (request) => { validateElicitation(request); return bridgeRequest({ type: 'privileged_request', op: 'elicitation', request }); }, enumerable: true },
    fetch: { value: (input, init) => bridgeRequest({ type: 'privileged_request', op: 'authenticated_fetch', input: serializeRequestInput(input), init: serializeRequestInit(init) }).then((value) => new Response(Buffer.from(value.body_base64 || '', 'base64'), { status: value.status, statusText: value.statusText, headers: value.headers })), enumerable: true },
    nativePipe: { value: nativePipe, enumerable: true },
    withSuspendedTimeout: { value: withSuspendedTimeout, enumerable: true },
  });
  return Object.freeze(trusted);
}

function validateElicitation(request) {
  if (!isPlainObject(request) || typeof request.message !== 'string' || request.message.length === 0) throw new TypeError('createElicitation requires a non-empty message');
  if (request.requestedSchema !== undefined && !isPlainObject(request.requestedSchema)) throw new TypeError('createElicitation requestedSchema must be a plain object');
  if (request.meta !== undefined && !isPlainObject(request.meta)) throw new TypeError('createElicitation meta must be a plain object');
}

function validateLaunchTarget(target) {
  if (!isPlainObject(target)) throw new TypeError('launchServices target must be an object');
  const hasApplication = typeof target.applicationPath === 'string' && target.applicationPath.length > 0;
  const hasBundle = typeof target.bundleIdentifier === 'string' && target.bundleIdentifier.length > 0;
  if (hasApplication === hasBundle) throw new TypeError('launchServices requires exactly one non-empty applicationPath or bundleIdentifier');
}

function parseConfig(value) {
  try { const parsed = JSON.parse(value); return isPlainObject(parsed) ? parsed : {}; } catch { return {}; }
}

function serializeRequestInput(input) {
  if (typeof input === 'string') return input;
  if (input instanceof URL) return input.href;
  if (input && typeof input.url === 'string') return input.url;
  throw new TypeError('authenticated fetch input must be a string or URL');
}

function serializeRequestInit(init) {
  if (!init) return undefined;
  const result = { ...init };
  if (result.body instanceof ArrayBuffer || Object.prototype.toString.call(result.body) === '[object ArrayBuffer]') {
    result.body = 'base64:' + Buffer.from(new Uint8Array(result.body)).toString('base64');
  } else if (ArrayBuffer.isView(result.body)) {
    result.body = 'base64:' + Buffer.from(result.body.buffer, result.body.byteOffset, result.body.byteLength).toString('base64');
  }
  return result;
}

const untrustedSurface = createUntrustedSurface();
const packageSurface = createPackageSurface(untrustedSurface);
const trustedSurface = createTrustedSurface(untrustedSurface);
const packageContext = createVmContext(packageSurface, 'package');

${KERNEL_MODULE_RESOLUTION_SOURCE}

function makeRootSource(source, declarations) {
  const seen = new Set();
  const exports = [];
  for (const declaration of declarations) if (!seen.has(declaration.name)) { seen.add(declaration.name); exports.push(declaration.name); }
  return source + (exports.length ? '\nexport { ' + exports.join(', ') + ' };\n' : '\n');
}

function validateBindings(value) {
  if (!Array.isArray(value)) throw new Error('kernel exec bindings are required');
  const kinds = new Set(['var', 'let', 'const', 'function', 'class']);
  for (const declaration of value) {
    if (!declaration || typeof declaration !== 'object' || !kinds.has(declaration.kind) || typeof declaration.name !== 'string' || declaration.name.length === 0) {
      throw new Error('kernel exec bindings are invalid');
    }
  }
  return value;
}

function checkBindingConflicts(declarations) {
  const local = new Map();
  for (const declaration of declarations) {
    const previous = local.get(declaration.name) || bindings.get(declaration.name);
    if (previous && (previous.kind !== 'var' || declaration.kind !== 'var')) throw new SyntaxError("Identifier '" + declaration.name + "' has already been declared");
    local.set(declaration.name, declaration);
  }
}

function createVmContext(surface, kind) {
  const globals = {
    nodeRepl: surface,
    console: Object.freeze({ log: outputLine, info: outputLine, warn: outputLine, error: outputLine, debug: outputLine }),
    Buffer,
    setTimeout,
    clearTimeout,
    setInterval,
    clearInterval,
    setImmediate,
    clearImmediate,
    queueMicrotask,
  };
  for (const name of NODE_WEB_GLOBAL_NAMES) {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, name);
    if (descriptor === undefined) throw new Error('bundled Node is missing required global ' + name);
    if ('value' in descriptor) Object.defineProperty(globals, name, descriptor);
    else {
      let value = globalThis[name];
      Object.defineProperty(globals, name, {
        configurable: descriptor.configurable,
        enumerable: descriptor.enumerable,
        get: () => value,
        set: typeof descriptor.set === 'function' ? (next) => { value = next; } : undefined,
      });
    }
  }
  for (const name of CANVAS_GLOBAL_NAMES) installLazyCanvasGlobal(globals, name);
  if (kind === 'trusted') globals.process = trustedProcess;
  else if (kind === 'package') globals.process = packageProcess;
  for (const [name, binding] of bindings) globals[name] = binding.value;
  const context = vm.createContext(globals, { name: kind === null ? 'node-repl-untrusted' : 'node-repl-' + kind });
  Object.defineProperty(context, 'global', { configurable: true, enumerable: true, writable: true, value: context });
  return context;
}

function installLazyCanvasGlobal(globals, name) {
  Object.defineProperty(globals, name, {
    configurable: true,
    enumerable: false,
    get() {
      const value = loadCanvasGlobalValues()[name];
      Object.defineProperty(globals, name, { configurable: true, enumerable: false, writable: true, value });
      return value;
    },
    set(value) {
      Object.defineProperty(globals, name, { configurable: true, enumerable: false, writable: true, value });
    },
  });
}

function loadCanvasGlobalValues() {
  if (canvasGlobalValues !== null) return canvasGlobalValues;
  const referrer = join(cwd, '.node_repl_canvas_bootstrap.cjs');
  const entrypoint = resolveWithNode('@napi-rs/canvas', referrer, true);
  const sourceBytes = Buffer.from(readFileSync(entrypoint));
  const policyResult = trustedModulePolicy.evaluate(
    entrypoint,
    sourceBytes,
    true,
    trustedModulePolicy.isTrustedDirectoryPath(entrypoint),
  );
  const packageModule = policyResult.trusted !== true;
  const context = policyResult.trusted === true
    ? createVmContext(trustedSurface, 'trusted')
    : packageContext;
  const canvas = loadCommonJsValue(entrypoint, sourceBytes, context, policyResult, packageModule);
  const values = {};
  for (const name of CANVAS_GLOBAL_NAMES) {
    if (typeof canvas[name] !== 'function') throw new Error('@napi-rs/canvas is missing required global ' + name);
    values[name] = canvas[name];
  }
  canvasGlobalValues = Object.freeze(values);
  return canvasGlobalValues;
}

${KERNEL_MODULE_LOADING_SOURCE}

async function executeCell(message) {
  const source = String(message.code || '');
  const declarations = validateBindings(message.bindings);
  checkBindingConflicts(declarations);
  const context = untrustedContext ??= createVmContext(untrustedSurface, null);
  const identifier = pathToFileURL(join(cwd, '.node_repl_cell_' + cellCounter++ + '.mjs')).href;
  const module = new vm.SourceTextModule(makeRootSource(source, declarations), {
    context,
    identifier,
    initializeImportMeta(meta, mod) { moduleMeta(meta, mod, true); },
    importModuleDynamically(specifier, mod) { return loadModule(specifier, mod.identifier, context, false, false); },
  });
  module.__trusted = false;
  module.__package = false;
  await module.link(() => { throw new Error('Top-level static import is not supported in node_repl. Use await import("...") instead.'); });
  try {
    await module.evaluate();
  } finally {
    for (const declaration of declarations) {
      if (!(declaration.name in module.namespace)) continue;
      try {
        const value = module.namespace[declaration.name];
        bindings.set(declaration.name, { kind: declaration.kind, value });
        context[declaration.name] = value;
      } catch {
        // A lexical binding whose initializer did not run remains in TDZ.
      }
    }
  }
  const exec = activeExecution();
  for (const hook of exec?.hooks.splice(0) || []) {
    let timer = null;
    try {
      await Promise.race([Promise.resolve().then(() => hook.run()), new Promise((resolve) => { timer = setTimeout(resolve, hook.timeoutMs); })]).catch(() => undefined);
    } finally {
      if (timer !== null) clearTimeout(timer);
    }
  }
  return { output: outputText(exec?.events || []), images: exec?.images || [], response_meta: exec && Object.keys(exec.responseMeta).length ? exec.responseMeta : null };
}

${KERNEL_PROTOCOL_SOURCE}
`;

export function sha256Bytes(value: Uint8Array): string {
  return createHash("sha256").update(value).digest("hex");
}
