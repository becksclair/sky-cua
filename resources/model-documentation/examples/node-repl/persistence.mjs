globalThis.counter ??= 0;
counter += 1;
nodeRepl.write({ counter, meta: nodeRepl.requestMeta });
