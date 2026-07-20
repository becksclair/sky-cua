{
  const { createCanvas } = await import("@napi-rs/canvas");
  const { default: pixelmatch } = await import("pixelmatch");
  const left = createCanvas(32, 32);
  const right = createCanvas(32, 32);
  left.getContext("2d").fillRect(0, 0, 16, 16);
  right.getContext("2d").fillRect(1, 0, 16, 16);
  const a = left.getContext("2d").getImageData(0, 0, 32, 32);
  const b = right.getContext("2d").getImageData(0, 0, 32, 32);
  const diff = createCanvas(32, 32);
  const output = diff.getContext("2d").createImageData(32, 32);
  const pixels = pixelmatch(a.data, b.data, output.data, 32, 32);
  diff.getContext("2d").putImageData(output, 0, 0);
  nodeRepl.write({ pixels });
  nodeRepl.emitImage(await diff.encode("webp"));
}
