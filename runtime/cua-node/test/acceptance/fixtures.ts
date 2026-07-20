import { existsSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import {
  isObject,
  requireString,
  sha256,
  stableJson,
  writeBytes,
  writeJson,
  writeText,
} from "./types.ts";

export const FIXTURE_ROOT = join(
  fileURLToPath(new URL("../fixtures/acceptance/", import.meta.url)),
);

const ONE_PIXEL_PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
  "base64",
);
const ONE_PIXEL_JPEG = Buffer.from(
  "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAP//////////////////////////////////////////////////////////////////////////////////////2wBDAf//////////////////////////////////////////////////////////////////////////////////////wAARCAABAAEDASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAX/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/9oADAMBAAIQAxAAAAH/AP/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAT8Af//EABQRAQAAAAAAAAAAAAAAAAAAABD/2gAIAQIBAT8Af//EABQRAQAAAAAAAAAAAAAAAAAAABD/2gAIAQMBAT8Af//Z",
  "base64",
);
const ONE_PIXEL_WEBP = Buffer.from(
  "UklGRiIAAABXRUJQVlA4IBYAAAAwAQCdASoBAAEADsD+JaQAA3AA/vuUAAA=",
  "base64",
);

type FixtureFile = {
  path: string;
  bytes: Uint8Array;
};

export type ExpectedMcpImageContent = {
  type: string;
  data: string;
  mimeType: string;
  _meta: { "codex/imageDetail": string };
};

function record(value: unknown, label: string): Record<string, unknown> {
  if (!isObject(value)) {
    throw new Error(`${label} must be an object`);
  }
  return value;
}

export function expectedMcpImageContent(): ExpectedMcpImageContent {
  const metadataPath = join(FIXTURE_ROOT, "../upstream-5307/output-metadata.json");
  const metadata = record(
    JSON.parse(readFileSync(metadataPath, "utf8")) as unknown,
    "output metadata",
  );
  const images = record(metadata.images, "output metadata images");
  if (!Array.isArray(images.sniff_cases) || images.sniff_cases.length === 0) {
    throw new Error("output metadata image sniff cases are missing");
  }
  const sniffCase = record(images.sniff_cases[0], "first image sniff case");
  const dataUrl = requireString(sniffCase.data_url, "first image sniff data_url");
  const separator = dataUrl.indexOf(",");
  if (separator < 0) {
    throw new Error("first image sniff data_url has no payload separator");
  }
  const mapping = record(metadata.mcp_image_mapping, "MCP image mapping");
  const shape = record(mapping.shape, "MCP image mapping shape");
  const meta = record(shape._meta, "MCP image mapping metadata");
  return {
    type: requireString(shape.type, "MCP image content type"),
    data: dataUrl.slice(separator + 1),
    mimeType: requireString(sniffCase.mimeType, "MCP image content mimeType"),
    _meta: {
      "codex/imageDetail": requireString(
        meta["codex/imageDetail"],
        "MCP image content detail",
      ),
    },
  };
}

function pdfFixture(): Uint8Array {
  return Buffer.from(
    "%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Count 1/Kids[3 0 R]>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 240 120]/Contents 4 0 R/Resources<</XObject<</Im1 5 0 R>>>>>>endobj\n4 0 obj<</Length 92>>stream\nBT /F1 12 Tf 20 95 Td (Cua Node acceptance PDF) Tj ET\n0 0 240 120 re S\nq 40 0 40 40 cm /Im1 Do Q\nendstream\nendobj\n5 0 obj<</Type/XObject/Subtype/Image/Width 1/Height 1/ColorSpace/DeviceRGB/BitsPerComponent 8/Filter/DCTDecode/Length 0>>stream\nendstream\nendobj\ntrailer<</Root 1 0 R>>\n%%EOF\n",
    "utf8",
  );
}

function ppmFixture(): Uint8Array {
  return Buffer.from(
    "P3\n4 2\n255\n255 0 0 0 255 0 0 0 255 255 255 255\n255 255 0 255 0 255 0 255 255 32 32 32\n",
    "utf8",
  );
}

function fileSpecs(): FixtureFile[] {
  const interactivePage = `<!doctype html>
<html><body>
  <main id="app" data-page="cua-node-acceptance">
    <h1 id="title">Interactive acceptance page</h1>
    <button id="action" data-result="clicked">Click me</button>
    <input id="name" aria-label="Name" value="">
    <div id="scroll-panel" data-scroll-top="0" data-scroll-left="0">Scroll target</div>
    <output id="result">ready</output>
  </main>
</body></html>
`;
  const pdf = pdfFixture();
  const assetFiles: FixtureFile[] = [
    { path: "media/ocr-known.ppm", bytes: ppmFixture() },
    { path: "media/acceptance.pdf", bytes: pdf },
    { path: "media/red.png", bytes: ONE_PIXEL_PNG },
  ];
  const files: FixtureFile[] = [
    { path: "browser/interactive.html", bytes: Buffer.from(interactivePage, "utf8") },
    {
      path: "browser/playwright-local.html",
      bytes: Buffer.from(
        interactivePage.replace(
          "Interactive acceptance page",
          "Standalone Playwright page",
        ),
        "utf8",
      ),
    },
    {
      path: "mcp/golden-transcript.ndjson",
      bytes: Buffer.from(
        '{"id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}\n{"id":2,"method":"tools/list"}\n{"id":3,"method":"tools/call","params":{"name":"js","arguments":{"code":"counter"}}}\n',
        "utf8",
      ),
    },
    {
      path: "mcp/malformed-framing.ndjson",
      bytes: Buffer.from(
        '{"id":1,"method":"initialize"}\nnot-json\n{"id":2,"method":"tools/call"\n',
        "utf8",
      ),
    },
    {
      path: "mcp/sky-actions.ndjson",
      bytes: Buffer.from(
        '{"op":"Health"}\n{"op":"Screenshot"}\n{"op":"Move","x":10,"y":20}\n{"op":"Click","x":10,"y":20,"button":"middle","click_count":2}\n{"op":"Scroll","direction":"left","pixels":17}\n',
        "utf8",
      ),
    },
    {
      path: "mcp/image-content.json",
      bytes: Buffer.from(stableJson(expectedMcpImageContent()), "utf8"),
    },
    ...assetFiles,
    { path: "media/white.jpg", bytes: ONE_PIXEL_JPEG },
    { path: "media/blue.webp", bytes: ONE_PIXEL_WEBP },
    {
      path: "media/acceptance.svg",
      bytes: Buffer.from(
        '<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="4" height="2" fill="#3366ff"/></svg>\n',
        "utf8",
      ),
    },
    {
      path: "media/known-text.txt",
      bytes: Buffer.from("Cua node offline OCR fixture 123\n", "utf8"),
    },
    {
      path: "manifests/valid-runtime.json",
      bytes: Buffer.from(
        stableJson({
          runtime_version: "fixture-1",
          node_version: "v24.14.0",
          asset_hashes: assetFiles.map((entry) => ({
            path: entry.path,
            sha256: sha256(entry.bytes),
          })),
        }),
        "utf8",
      ),
    },
    {
      path: "manifests/corrupt-runtime.json",
      bytes: Buffer.from(
        '{"runtime_version":"fixture-1","assets":[{"path":"media/red.png","sha256":"0000000000000000000000000000000000000000000000000000000000000000"}]}\n',
        "utf8",
      ),
    },
    {
      path: "manifests/missing-runtime.json",
      bytes: Buffer.from(
        '{"runtime_version":"fixture-1","asset_hashes":[{"path":"media/missing-asset.bin","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}\n',
        "utf8",
      ),
    },
  ];
  return files.sort((left, right) => left.path.localeCompare(right.path));
}

export function expectedFixtureFiles(): FixtureFile[] {
  return fileSpecs();
}

export function fixturePath(relativePath: string): string {
  return join(FIXTURE_ROOT, relativePath);
}

export function fixtureBytes(relativePath: string): Uint8Array {
  const spec = fileSpecs().find((entry) => entry.path === relativePath);
  if (spec === undefined) {
    throw new Error(`unknown acceptance fixture: ${relativePath}`);
  }
  return spec.bytes;
}

export function fixtureIndex(): Record<string, unknown> {
  return {
    schema_version: "cua-node-acceptance/fixtures-v1",
    files: fileSpecs().map((entry) => ({
      path: entry.path,
      bytes: entry.bytes.byteLength,
      sha256: sha256(entry.bytes),
    })),
  };
}

export function generateFixtures(
  options: { checkOnly: boolean; root?: string } = { checkOnly: true },
): { files: number; changed: boolean } {
  const root = options.root ?? FIXTURE_ROOT;
  const specs = fileSpecs();
  let changed = false;
  for (const spec of specs) {
    const path = join(root, spec.path);
    if (options.checkOnly) {
      if (
        !existsSync(path) ||
        !Buffer.from(readFileSync(path)).equals(Buffer.from(spec.bytes))
      ) {
        throw new Error(`fixture drift: ${relative(root, path)}`);
      }
    } else {
      const existing = existsSync(path) ? readFileSync(path) : null;
      if (existing === null || !existing.equals(Buffer.from(spec.bytes))) {
        if (spec.bytes.byteLength === 0) {
          writeText(path, "");
        } else {
          writeBytes(path, spec.bytes);
        }
        changed = true;
      }
    }
  }
  const indexPath = join(root, "fixture-index.json");
  const index = stableJson(fixtureIndex());
  if (options.checkOnly) {
    if (!existsSync(indexPath) || readFileSync(indexPath, "utf8") !== index) {
      throw new Error("fixture drift: fixture-index.json");
    }
  } else if (!existsSync(indexPath) || readFileSync(indexPath, "utf8") !== index) {
    writeJson(indexPath, fixtureIndex());
    changed = true;
  }
  if (!options.checkOnly && existsSync(root)) {
    const expected = new Set([
      ...specs.map((entry) => entry.path),
      "fixture-index.json",
    ]);
    for (const child of readdirSync(root, { withFileTypes: true })) {
      if (child.isFile() && !expected.has(child.name)) {
        rmSync(join(root, child.name));
      }
    }
  }
  return { files: specs.length + 1, changed };
}

export function fixtureManifest(relativePath: string): Record<string, unknown> {
  const value = JSON.parse(
    Buffer.from(fixtureBytes(relativePath)).toString("utf8"),
  ) as Record<string, unknown>;
  return value;
}
