{
  const { readFile, writeFile } = await import("node:fs/promises");
  const { dirname, join } = await import("node:path");
  const { fileURLToPath } = await import("node:url");
  const input = fileURLToPath(new URL(nodeRepl.env.SKY_CUA_EXAMPLE_INPUT_FILE));
  const bytes = await readFile(input);
  const output = join(dirname(input), "sky-cua-copy.bin");
  await writeFile(output, Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength));
  nodeRepl.write({ output, size: bytes.byteLength, dataUrl: `data:application/octet-stream;base64,${bytes.toString("base64")}` });
}
