import { createHash } from "node:crypto";
import { readFileSync, readdirSync, statSync, writeFileSync, mkdirSync, rmSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { join, relative } from "node:path";

type TarEntry = { name: string; bytes: Buffer; mode?: number };

function filesUnder(root: string, prefix: string): TarEntry[] {
  const entries: TarEntry[] = [];
  for (const name of readdirSync(root).sort((a, b) => a.localeCompare(b))) {
    const path = join(root, name);
    const archiveName = `${prefix}/${name}`;
    if (statSync(path).isDirectory()) {
      entries.push(...filesUnder(path, archiveName));
    } else {
      entries.push({ name: archiveName, bytes: readFileSync(path) });
    }
  }
  return entries;
}

function octal(value: number, width: number): Buffer {
  const encoded = value.toString(8).padStart(width - 1, "0");
  return Buffer.from(`${encoded}\0`, "ascii");
}

function tarHeader(entry: TarEntry): Buffer {
  const header = Buffer.alloc(512);
  header.write(entry.name, 0, 100, "utf8");
  octal(entry.mode ?? 0o644, 8).copy(header, 100);
  octal(0, 8).copy(header, 108);
  octal(0, 8).copy(header, 116);
  octal(entry.bytes.length, 12).copy(header, 124);
  octal(0, 12).copy(header, 136);
  header.fill(0x20, 148, 156);
  header[156] = 0x30;
  Buffer.from("ustar\0", "ascii").copy(header, 257);
  Buffer.from("00", "ascii").copy(header, 263);
  Buffer.from("root", "ascii").copy(header, 265);
  Buffer.from("root", "ascii").copy(header, 297);
  let checksum = 0;
  for (const byte of header) {
    checksum += byte;
  }
  octal(checksum, 8).copy(header, 148);
  return header;
}

function deterministicTar(entries: readonly TarEntry[]): Buffer {
  const chunks: Buffer[] = [];
  for (const entry of entries) {
    chunks.push(tarHeader(entry), entry.bytes);
    const padding = (512 - (entry.bytes.length % 512)) % 512;
    if (padding > 0) {
      chunks.push(Buffer.alloc(padding));
    }
  }
  chunks.push(Buffer.alloc(1024));
  return Buffer.concat(chunks);
}

rmSync("out", { recursive: true, force: true });
mkdirSync("out", { recursive: true });
const entries = [
  { name: "package/package.json", bytes: readFileSync("package.json") },
  { name: "package/README.md", bytes: readFileSync("README.md") },
  { name: "package/scripts/acceptance-actions.mjs", bytes: readFileSync("scripts/acceptance-actions.mjs"), mode: 0o755 },
  ...filesUnder("dist", "package/dist")
];
const archive = gzipSync(deterministicTar(entries), { level: 9, mtime: 0 });
const output = "out/sky-cua-0.1.0.tgz";
writeFileSync(output, archive);
const sha256 = createHash("sha256").update(archive).digest("hex");
const integrity = `sha512-${createHash("sha512").update(archive).digest("base64")}`;
console.log(JSON.stringify({ output, sha256, integrity, files: entries.map((entry) => relative(".", entry.name)) }, null, 2));
