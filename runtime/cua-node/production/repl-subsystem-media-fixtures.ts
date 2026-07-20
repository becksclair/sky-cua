const { mkdirSync, writeFileSync } = require("node:fs");
const { join } = require("node:path");

type MediaFixturePaths = {
  root: string;
  pdf: string;
  malformedImage: string;
  malformedPdf: string;
};

function pdfStream(dictionary: string, bytes: Buffer): Buffer {
  return Buffer.concat([
    Buffer.from(`<< ${dictionary} /Length ${bytes.length} >>\nstream\n`, "ascii"),
    bytes,
    Buffer.from("\nendstream", "ascii"),
  ]);
}

function deterministicPdf(): Buffer {
  const text = Buffer.from("BT /F1 24 Tf 40 100 Td (ZERO PREAMBLE PDF) Tj ET", "ascii");
  const image = Buffer.from([255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 220, 0]);
  const objects = [
    Buffer.from("<< /Type /Catalog /Pages 2 0 R >>", "ascii"),
    Buffer.from("<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>", "ascii"),
    Buffer.from(
      "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 320 180] /Resources << /Font << /F1 8 0 R >> >> /Contents 5 0 R >>",
      "ascii",
    ),
    Buffer.from(
      "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 320 180] /Resources << /XObject << /Im0 7 0 R >> >> /Contents 6 0 R >>",
      "ascii",
    ),
    pdfStream("", text),
    pdfStream("", Buffer.from("q 160 0 0 120 80 30 cm /Im0 Do Q", "ascii")),
    pdfStream(
      "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8",
      image,
    ),
    Buffer.from("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>", "ascii"),
  ];
  const chunks: Buffer[] = [Buffer.from("%PDF-1.4\n%\xe2\xe3\xcf\xd3\n", "binary")];
  const offsets = [0];
  let length = chunks[0].length;
  objects.forEach((object, index) => {
    offsets.push(length);
    const chunk = Buffer.concat([
      Buffer.from(`${index + 1} 0 obj\n`, "ascii"),
      object,
      Buffer.from("\nendobj\n", "ascii"),
    ]);
    chunks.push(chunk);
    length += chunk.length;
  });
  const xref = length;
  const xrefLines = offsets
    .slice(1)
    .map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`)
    .join("");
  chunks.push(
    Buffer.from(
      `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${xrefLines}trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`,
      "ascii",
    ),
  );
  return Buffer.concat(chunks);
}

function prepareMediaFixtures(tempRoot: string): MediaFixturePaths {
  const root = join(tempRoot, "media fixtures - 日本語");
  mkdirSync(root, { recursive: true });
  const paths = {
    root,
    pdf: join(root, "input vector + image.pdf"),
    malformedImage: join(root, "malformed image.png"),
    malformedPdf: join(root, "malformed document.pdf"),
  };
  writeFileSync(paths.pdf, deterministicPdf());
  writeFileSync(paths.malformedImage, "not an image", "utf8");
  writeFileSync(paths.malformedPdf, "%PDF-not-valid", "ascii");
  return paths;
}

exports.deterministicPdf = deterministicPdf;
exports.prepareMediaFixtures = prepareMediaFixtures;
