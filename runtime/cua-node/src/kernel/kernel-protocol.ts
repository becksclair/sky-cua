export const KERNEL_PROTOCOL_SOURCE = String.raw`
async function handleExec(message) {
  if (runningExecution !== null) { send({ type: 'exec_result', id: message.id, ok: false, output: '', error: 'node_repl kernel is busy' }); return; }
  const exec = { id: message.id, requestMeta: message.request_meta === undefined || message.request_meta === null ? null : cloneFreeze(message.request_meta), events: [], images: [], imageCounter: 0, responseMeta: {}, hooks: [], closed: false };
  runningExecution = exec;
  await executionStorage.run(exec, async () => {
    try {
      const result = await executeCell(message);
      send({ type: 'exec_result', id: message.id, ok: true, output: result.output, images: result.images, response_meta: result.response_meta });
    } catch (error) {
      const text = error instanceof Error ? error.message : String(error);
      send({ type: 'exec_result', id: message.id, ok: false, output: '', error: text, response_meta: Object.keys(exec.responseMeta).length ? exec.responseMeta : null });
    } finally { exec.closed = true; if (runningExecution === exec) runningExecution = null; }
  });
}

function receiveBridge(message) {
  if (message.type === 'native_pipe_data' || message.type === 'native_pipe_closed') nativePipe.receive(message);
  const waiter = bridgeWaiters.get(message.id);
  if (!waiter) return;
  bridgeWaiters.delete(message.id);
  if (message.ok === false) waiter.reject(new Error(message.error || 'privileged operation failed'));
  else waiter.resolve(message.result);
}

async function main() {
  send({ type: 'privileged_bridge_handshake', token: bridgeToken });
  process.on('message', (message) => {
    if (!message || typeof message !== 'object' || Array.isArray(message)) { send({ type: 'protocol_error', error: 'kernel control message must be an object' }); return; }
    if (message.version !== PROTOCOL) { send({ type: 'protocol_error', error: 'unsupported kernel control version' }); return; }
    if (message.type === 'exec') void handleExec(message);
    else if (message.type === 'add_node_module_dir' && typeof message.id === 'string' && typeof message.path === 'string') {
      const normalized = resolve(cwd, message.path).endsWith('node_modules') ? dirname(resolve(cwd, message.path)) : resolve(cwd, message.path);
      const added = !addedModuleDirSet.has(normalized);
      if (added) { addedModuleDirSet.add(normalized); addedModuleDirs.push(normalized); }
      send({ type: 'module_dir_result', id: message.id, added });
    }
    else if (message.type === 'bridge_response' || message.type === 'native_pipe_data' || message.type === 'native_pipe_closed') receiveBridge(message);
    else if (message.type === 'shutdown') { process.exitCode = 0; if (process.connected) process.disconnect(); }
  });
}

void main().catch((error) => { process.stderr.write((error instanceof Error ? error.stack || error.message : String(error)) + '\n'); process.exitCode = 1; });
`;
