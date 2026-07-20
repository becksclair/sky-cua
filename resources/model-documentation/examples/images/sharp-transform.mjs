{
  const { default: sharp } = await import("sharp");
  const { dirname, join } = await import("node:path");
  const input = nodeRepl.env.SKY_CUA_EXAMPLE_IMAGE;
  const output = join(dirname(input), "sky-cua-example.webp");
  await sharp(input).resize({ width: 640, withoutEnlargement: true }).webp({ quality: 82 }).toFile(output);
  nodeRepl.write({ output });
  await nodeRepl.emitImage(await sharp(output).toBuffer());
}
