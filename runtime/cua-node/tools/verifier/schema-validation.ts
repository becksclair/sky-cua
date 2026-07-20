import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { record } from "./json";

type AjvError = { instancePath?: string; message?: string };
type AjvValidator = ((value: unknown) => boolean) & { errors?: AjvError[] | null };
type AjvInstance = { compile: (schema: Record<string, unknown>) => AjvValidator };
type AjvConstructor = new (options: {
  allErrors: boolean;
  strict: boolean;
}) => AjvInstance;

export const RUNTIME_MANIFEST_SCHEMA_PATH = resolve(
  __dirname,
  "../../contracts/runtime-manifest.schema.json",
);
export const RUNTIME_LOCK_SCHEMA_PATH = resolve(
  __dirname,
  "../../runtime-lock.schema.json",
);
export const NATIVE_ASSETS_LOCK_SCHEMA_PATH = resolve(
  __dirname,
  "../../native-assets.lock.schema.json",
);
export const BLOCKED_LOCK_SCHEMA_PATH = resolve(
  __dirname,
  "../../blocked-lock.schema.json",
);

const validators = new Map<string, AjvValidator>();

function validatorFor(schemaPath: string): AjvValidator {
  const cached = validators.get(schemaPath);
  if (cached !== undefined) return cached;
  const Ajv2020 = require("ajv/dist/2020").default as AjvConstructor;
  const schema = record(
    JSON.parse(readFileSync(schemaPath, "utf8")) as unknown,
    schemaPath,
  );
  const validator = new Ajv2020({ allErrors: true, strict: true }).compile(schema);
  validators.set(schemaPath, validator);
  return validator;
}

export function validateCanonicalSchema(value: unknown, schemaPath: string): string[] {
  try {
    const validator = validatorFor(schemaPath);
    if (validator(value)) return [];
    return (validator.errors ?? []).map((error) => {
      const location = error.instancePath?.length ? error.instancePath : "$";
      return `${location} ${error.message ?? "does not satisfy canonical schema"}`;
    });
  } catch (error) {
    return [
      error instanceof Error ? error.message : "canonical schema validation failed",
    ];
  }
}
