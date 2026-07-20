export const KERNEL_MODULE_RESOLUTION_SOURCE = String.raw`function withinPath(file, root) {
  const rel = relative(root, file);
  return rel === '' || (rel !== '..' && !rel.startsWith('..' + (process.platform === 'win32' ? '\\' : '/')) && !isAbsolute(rel));
}

function resolveSpecifier(specifier, referrer, parentTrusted, parentPackage) {
  if (specifier === 'process' || specifier === 'node:process') {
    if (!parentTrusted && !parentPackage) throw new Error('Cannot import process from untrusted node_repl code');
    return { builtin: 'node:process', trustedProcess: parentTrusted, packageProcess: parentPackage };
  }
  if (specifier.startsWith('node:') || builtinModuleNames.has(specifier)) return { builtin: specifier };
  const referrerPath = fileURLToPath(referrer);
  const isPackage = !specifier.startsWith('.') && !specifier.startsWith('/') && !specifier.startsWith('file:');
  const file = resolveWithNode(specifier, referrerPath, isPackage);
  return {
    file,
    packageEntrypoint: isPackage && !parentTrusted,
    trusted: parentTrusted || trustedModulePolicy.isTrustedDirectoryPath(file),
  };
}

function resolveWithNode(specifier, referrerPath, isPackage) {
  const target = specifier.startsWith('file:') ? fileURLToPath(specifier) : specifier;
  if (!isPackage) return createRequire(referrerPath).resolve(target);
  const packageRoot = findPackageRoot(referrerPath);
  const roots = [];
  if (packageRoot !== null) roots.push(packageRoot.root);
  roots.push(...nodeModuleDirs, ...addedModuleDirs, cwd);
  let firstError = null;
  for (const root of roots) {
    try {
      return resolvePackageSpecifier(specifier, root, packageRoot);
    } catch (error) {
      if (firstError === null) firstError = error;
    }
  }
  throw firstError instanceof Error ? firstError : new Error('Cannot find module ' + specifier);
}

function packageParts(specifier) {
  const parts = specifier.split('/');
  const name = specifier.startsWith('@') ? parts.slice(0, 2).join('/') : parts[0];
  return [name, parts.slice(name.startsWith('@') ? 2 : 1).join('/')];
}

function findPackageRoot(file) {
  let current = dirname(file);
  while (current !== dirname(current)) {
    const packageJsonPath = join(current, 'package.json');
    if (existsSync(packageJsonPath)) {
      try {
        const json = JSON.parse(readFileSync(packageJsonPath, 'utf8'));
        return { root: current, packageName: typeof json.name === 'string' ? json.name : null };
      } catch {
        return null;
      }
    }
    current = dirname(current);
  }
  return null;
}

function resolvePackageSpecifier(specifier, root, referrerPackage) {
  const [name, subpath] = packageParts(specifier);
  const packageRoot = referrerPackage?.packageName === name
    ? referrerPackage.root
    : resolve(root, 'node_modules', name);
  if (!existsSync(packageRoot)) throw new Error('Cannot find package ' + specifier);
  const packageJsonPath = join(packageRoot, 'package.json');
  let packageJson = {};
  if (existsSync(packageJsonPath)) {
    try {
      packageJson = JSON.parse(readFileSync(packageJsonPath, 'utf8'));
    } catch {
      throw new Error('invalid package.json: ' + packageJsonPath);
    }
  }
  let target;
  let packageExport = false;
  if (packageJson.exports !== undefined) {
    packageExport = true;
    target = resolvePackageExports(packageJson.exports, subpath.length === 0 ? '.' : './' + subpath);
  } else if (subpath.length > 0) {
    target = './' + subpath;
  } else {
    target = typeof packageJson.main === 'string' ? packageJson.main : typeof packageJson.module === 'string' ? packageJson.module : './index.js';
  }
  if (typeof target !== 'string' || target.length === 0) throw new Error('package entrypoint must resolve to a relative file');
  if (packageExport && !target.startsWith('./')) throw new Error('package exports must resolve to a relative file');
  const relativeTarget = target.startsWith('./') ? target.slice(2) : target;
  const file = resolve(packageRoot, relativeTarget);
  if (!trustedPathContains(file, packageRoot)) throw new Error('package import escapes its configured root');
  return resolveFileCandidate(file);
}

function resolvePackageExports(exportsValue, subpath) {
  const keys = exportsValue && typeof exportsValue === 'object' && !Array.isArray(exportsValue)
    ? Object.keys(exportsValue)
    : [];
  if (keys.some((key) => key.startsWith('.'))) {
    const exact = exportsValue[subpath];
    if (exact !== undefined) return resolveConditionalExport(exact);
    for (const key of keys.filter((entry) => entry.includes('*')).sort((left, right) => right.length - left.length)) {
      const [prefix, suffix] = key.split('*');
      if (!subpath.startsWith(prefix) || !subpath.endsWith(suffix)) continue;
      const target = resolveConditionalExport(exportsValue[key]);
      return target?.replaceAll('*', subpath.slice(prefix.length, subpath.length - suffix.length));
    }
    throw new Error('package subpath is not exported: ' + subpath);
  }
  if (subpath !== '.') throw new Error('package subpath is not exported: ' + subpath);
  return resolveConditionalExport(exportsValue);
}

function resolveConditionalExport(value) {
  if (typeof value === 'string') return value;
  if (Array.isArray(value)) {
    for (const candidate of value) {
      try { return resolveConditionalExport(candidate); } catch { /* try the next fallback */ }
    }
    throw new Error('package export conditions did not resolve');
  }
  if (!value || typeof value !== 'object') throw new Error('package export target is invalid');
  for (const [condition, candidate] of Object.entries(value)) {
    if (condition !== 'import' && condition !== 'node' && condition !== 'default') continue;
    try { return resolveConditionalExport(candidate); } catch { /* try the next condition */ }
  }
  throw new Error('package export conditions did not resolve');
}

function resolveFileCandidate(file) {
  const directoryPackageJson = join(file, 'package.json');
  if (existsSync(directoryPackageJson)) {
    try {
      const packageJson = JSON.parse(readFileSync(directoryPackageJson, 'utf8'));
      const target = typeof packageJson.main === 'string' ? packageJson.main : typeof packageJson.module === 'string' ? packageJson.module : './index.js';
      return resolveFileCandidate(resolve(file, target));
    } catch (error) {
      if (error instanceof Error && error.message.startsWith('module not found:')) throw error;
      throw new Error('invalid package.json: ' + directoryPackageJson);
    }
  }
  if (existsSync(file) && statSync(file).isFile()) return file;
  for (const extension of ['.js', '.mjs', '.cjs']) if (existsSync(file + extension)) return file + extension;
  throw new Error('module not found: ' + file);
}

function packageTypeFor(file) {
  const extension = extname(file);
  if (extension === '.mjs') return 'module';
  if (extension === '.cjs') return 'commonjs';
  let current = dirname(file);
  while (current !== dirname(current)) {
    const packageJsonPath = join(current, 'package.json');
    if (existsSync(packageJsonPath)) {
      try {
        const json = JSON.parse(readFileSync(packageJsonPath, 'utf8'));
        return json.type === 'module' ? 'module' : 'commonjs';
      } catch {
        throw new Error('invalid package.json: ' + packageJsonPath);
      }
    }
    current = dirname(current);
  }
  return 'commonjs';
}

async function builtinModule(specifier, context, processFacade, referrer, parentTrusted, parentPackage) {
  const usesProcessFacade = processFacade !== null && specifier === 'node:process';
  const normalized = assertBuiltinModuleAllowed(specifier);
  const namespace = usesProcessFacade
    ? processFacade
    : normalized === 'module'
      ? createPolicyModuleFacade(referrer, context, parentTrusted, parentPackage)
      : await import(specifier);
  const names = Object.keys(namespace);
  if (usesProcessFacade && !names.includes('default')) names.push('default');
  const module = new vm.SyntheticModule(names, function () {
    for (const name of names) this.setExport(name, name === 'default' && usesProcessFacade ? processFacade : namespace[name]);
  }, { context });
  await module.link(() => { throw new Error('builtin modules cannot have VM imports'); });
  await module.evaluate();
  return module;
}

function createPolicyModuleFacade(referrer, context, parentTrusted, parentPackage) {
  return Object.freeze({
    builtinModules: Object.freeze([...builtinModules]),
    createRequire(requestedReferrer) {
      const parentFile = fileURLToPath(referrer);
      const file = normalizeCreateRequireReferrer(requestedReferrer);
      if (parentPackage) {
        const packageRoot = findPackageRoot(parentFile);
        if (packageRoot === null || !trustedPathContains(file, packageRoot.root)) {
          throw new Error('package createRequire() cannot escape its package root');
        }
      } else if (!parentTrusted && file !== parentFile) {
        throw new Error('model createRequire() must remain rooted at its importing module');
      }
      return createCommonJsRequire(file, context, parentTrusted, parentPackage, createSyntheticCommonJsParent(file));
    },
  });
}`;

export const KERNEL_MODULE_LOADING_SOURCE = String.raw`function moduleMeta(meta, module, isRoot) {
  const path = fileURLToPath(module.identifier);
  meta.url = module.identifier;
  meta.filename = path;
  meta.dirname = dirname(path);
  meta.main = isRoot;
  meta.resolve = (specifier) => {
    const resolved = resolveSpecifier(specifier, module.identifier, module.__trusted === true, module.__package === true);
    return resolved.builtin || pathToFileURL(resolved.file).href;
  };
}

async function loadModule(specifier, referrer, context, parentTrusted, parentPackage) {
  const resolved = resolveSpecifier(specifier, referrer, parentTrusted, parentPackage);
  if (resolved.builtin) {
    const processFacade = resolved.trustedProcess || parentTrusted
      ? trustedProcess
      : resolved.packageProcess || parentPackage
        ? packageProcess
        : null;
    return builtinModule(resolved.builtin, context, processFacade, referrer, parentTrusted, parentPackage);
  }
  const cacheKey = pathToFileURL(resolved.file).href;
  if (cachedModules.has(cacheKey)) return cachedModules.get(cacheKey);
  const sourceBytes = Buffer.from(readFileSync(resolved.file));
  const policyResult = trustedModulePolicy.evaluate(
    resolved.file,
    sourceBytes,
    resolved.packageEntrypoint === true,
    resolved.trusted === true,
  );
  const packageModule = policyResult.trusted !== true && (parentPackage || resolved.packageEntrypoint === true);
  const shouldCache = policyResult.trusted === true || packageModule;
  const moduleContext = policyResult.trusted === true
    ? (parentTrusted ? context : createVmContext(trustedSurface, 'trusted'))
    : packageModule
      ? packageContext
      : context;
  if (extname(resolved.file) === '.json') {
    return loadJsonModule(resolved.file, moduleContext, cacheKey, shouldCache);
  }
  if (packageTypeFor(resolved.file) === 'commonjs') {
    return loadCommonJsModule(resolved, sourceBytes, policyResult, moduleContext, cacheKey, packageModule, shouldCache);
  }
  const source = sourceBytes.toString('utf8');
  const module = new vm.SourceTextModule(source, {
    context: moduleContext,
    identifier: cacheKey,
    initializeImportMeta(meta, mod) { moduleMeta(meta, mod, false); },
    importModuleDynamically(spec, mod) { return loadModule(spec, mod.identifier, mod.context, policyResult.trusted === true, module.__package === true); },
  });
  module.__trusted = policyResult.trusted === true;
  module.__package = packageModule;
  if (shouldCache) cachedModules.set(cacheKey, module);
  try {
    await module.link((childSpecifier, referencingModule) => loadModule(childSpecifier, referencingModule.identifier, moduleContext, policyResult.trusted === true, module.__package === true));
    await module.evaluate();
    return module;
  } catch (error) {
    if (shouldCache) cachedModules.delete(cacheKey);
    throw error;
  }
}

async function loadJsonModule(file, context, cacheKey, shouldCache) {
  if (cachedModules.has(cacheKey)) return cachedModules.get(cacheKey);
  const value = JSON.parse(readFileSync(file, 'utf8'));
  const module = new vm.SyntheticModule(['default'], function () {
    this.setExport('default', value);
  }, { context });
  if (shouldCache) cachedModules.set(cacheKey, module);
  try {
    await module.evaluate();
    return module;
  } catch (error) {
    if (shouldCache) cachedModules.delete(cacheKey);
    throw error;
  }
}

async function loadCommonJsModule(resolved, sourceBytes, policyResult, context, cacheKey, packageModule, shouldCache) {
  if (!policyResult.trusted && !packageModule) throw new Error('CommonJS modules must be loaded from a package context or trusted code');
  if (cachedModules.has(cacheKey)) return cachedModules.get(cacheKey);
  const value = loadCommonJsValue(resolved.file, sourceBytes, context, policyResult, packageModule);
  const names = new Set(['default']);
  if (value !== null && (typeof value === 'object' || typeof value === 'function')) {
    for (const name of Object.keys(value)) names.add(name);
  }
  const module = new vm.SyntheticModule([...names], function () {
    this.setExport('default', value);
    if (value !== null && (typeof value === 'object' || typeof value === 'function')) {
      for (const name of names) if (name !== 'default') this.setExport(name, value[name]);
    }
  }, { context });
  if (shouldCache) cachedModules.set(cacheKey, module);
  try {
    await module.evaluate();
    return module;
  } catch (error) {
    if (shouldCache) cachedModules.delete(cacheKey);
    throw error;
  }
}

function loadCommonJsValue(file, sourceBytes, context, policyResult, packageModule) {
  const existing = commonJsModules.get(file);
  if (existing !== undefined) return existing.exports;
  const extension = extname(file);
  if (extension === '.node') return createRequire(file)(file);
  const module = {
    children: [],
    exports: {},
    filename: file,
    id: file,
    loaded: false,
    parent: null,
    path: dirname(file),
    paths: createRequire(file).resolve.paths('.') || [],
  };
  commonJsModules.set(file, module);
  commonJsCache[file] = module;
  try {
    if (extension === '.json') {
      module.exports = JSON.parse(readFileSync(file, 'utf8'));
    } else {
      let source = sourceBytes.toString('utf8');
      if (source.codePointAt(0) === 0xfeff) source = source.slice(1);
      if (source.startsWith('#!')) {
        const lineEnd = source.indexOf('\n');
        source = lineEnd === -1 ? '' : source.slice(lineEnd);
      }
      const trusted = policyResult.trusted === true;
      const commonJsProcess = trusted ? commonJsTrustedProcess : commonJsPackageProcess;
      const localRequire = createCommonJsRequire(file, context, trusted, packageModule, module);
      module.require = localRequire;
      const wrapper = new vm.Script('(function (exports, require, module, __filename, __dirname, process) { ' + source + '\n})', {
        filename: file,
        importModuleDynamically(specifier) {
          return loadModule(specifier, pathToFileURL(file).href, context, trusted, packageModule);
        },
      }).runInContext(context);
      wrapper.call(module.exports, module.exports, localRequire, module, file, dirname(file), commonJsProcess);
    }
    module.loaded = true;
    return module.exports;
  } catch (error) {
    commonJsModules.delete(file);
    delete commonJsCache[file];
    throw error;
  }
}

function createCommonJsRequire(parentFile, context, parentTrusted, parentPackage, parentModule) {
  const hostRequire = createRequire(parentFile);
  function localRequire(specifier) {
    if (typeof specifier !== 'string') throw new TypeError('require specifier must be a string');
    if (specifier === 'process' || specifier === 'node:process') {
      if (!parentTrusted && !parentPackage) throw new Error('Cannot require process from untrusted node_repl code');
      return parentTrusted ? commonJsTrustedProcess : commonJsPackageProcess;
    }
    if (specifier.startsWith('node:') || builtinModuleNames.has(specifier)) {
      const processFacade = parentTrusted ? commonJsTrustedProcess : parentPackage ? commonJsPackageProcess : null;
      if (specifier === 'module' || specifier === 'node:module') {
        return createPolicyModuleFacade(pathToFileURL(parentFile).href, context, parentTrusted, parentPackage);
      }
      assertBuiltinModuleAllowed(specifier);
      return compatibleBuiltinModule(specifier, processFacade);
    }
    const childFile = hostRequire.resolve(specifier);
    const extension = extname(childFile);
    if (extension === '.node') return hostRequire(childFile);
    const existing = commonJsModules.get(childFile);
    if (existing !== undefined) {
      addCommonJsChild(parentModule, existing);
      return existing.exports;
    }
    const sourceBytes = Buffer.from(readFileSync(childFile));
    const childPolicy = trustedModulePolicy.evaluate(
      childFile,
      sourceBytes,
      false,
      parentTrusted || trustedModulePolicy.isTrustedDirectoryPath(childFile),
    );
    const childPackage = childPolicy.trusted !== true && parentPackage;
    if (!childPolicy.trusted && !childPackage) throw new Error('CommonJS modules must be loaded from a package context or trusted code');
    if (extension !== '.json' && packageTypeFor(childFile) === 'module') {
      throw new Error('require() of ES Module ' + childFile + ' is not supported in the node_repl VM loader');
    }
    const childContext = childPolicy.trusted === true
      ? (parentTrusted ? context : createVmContext(trustedSurface, 'trusted'))
      : childPackage
        ? packageContext
        : context;
    const value = loadCommonJsValue(childFile, sourceBytes, childContext, childPolicy, childPackage);
    const childModule = commonJsModules.get(childFile);
    if (childModule !== undefined) {
      childModule.parent = parentModule;
      addCommonJsChild(parentModule, childModule);
    }
    return value;
  }
  localRequire.cache = commonJsCache;
  localRequire.main = undefined;
  localRequire.resolve = (specifier) => hostRequire.resolve(specifier);
  localRequire.resolve.paths = (specifier) => hostRequire.resolve.paths(specifier);
  return localRequire;
}

function addCommonJsChild(parentModule, childModule) {
  let children = commonJsModuleChildren.get(parentModule);
  if (children === undefined) {
    children = new Set();
    commonJsModuleChildren.set(parentModule, children);
  }
  if (children.has(childModule)) return;
  children.add(childModule);
  parentModule.children.push(childModule);
}`;
